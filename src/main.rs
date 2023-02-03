use bevy::{
    diagnostic::FrameTimeDiagnosticsPlugin,
    input::keyboard::KeyboardInput, prelude::*, window::PresentMode,
};
use std::{
    collections::HashMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    thread::sleep,
    time::Duration,
    vec,
};

use anyhow::{bail, Context, Ok, Result};
use log::info;
use structopt::StructOpt;

const TERMINAL_WIDTH: u8 = 64;
const TERMINAL_HEIGHT: u8 = 32;
const RAM_SIZE: usize = 4096;
const PROGRAM_START: usize = 512;
const FPS: u64 = 60;

// Font settings
const FONT_ADDR: usize = 0x50;
const FONT_SIZE: usize = 5;
const FONTS: [u8; 80] = [
    0xF0, 0x90, 0x90, 0x90, 0xF0, // 0
    0x20, 0x60, 0x20, 0x20, 0x70, // 1
    0xF0, 0x10, 0xF0, 0x80, 0xF0, // 2
    0xF0, 0x10, 0xF0, 0x10, 0xF0, // 3
    0x90, 0x90, 0xF0, 0x10, 0x10, // 4
    0xF0, 0x80, 0xF0, 0x10, 0xF0, // 5
    0xF0, 0x80, 0xF0, 0x90, 0xF0, // 6
    0xF0, 0x10, 0x20, 0x40, 0x40, // 7
    0xF0, 0x90, 0xF0, 0x90, 0xF0, // 8
    0xF0, 0x90, 0xF0, 0x10, 0xF0, // 9
    0xF0, 0x90, 0xF0, 0x90, 0x90, // A
    0xE0, 0x90, 0xE0, 0x90, 0xE0, // B
    0xF0, 0x80, 0x80, 0x80, 0xF0, // C
    0xE0, 0x90, 0x90, 0x90, 0xE0, // D
    0xF0, 0x80, 0xF0, 0x80, 0xF0, // E
    0xF0, 0x80, 0xF0, 0x80, 0x80, // F
];

/// Chip8 emulator state.
#[derive(Debug, Default)]
struct Chip8 {
    clock: u64,
    pixels: Vec<Vec<bool>>,
    ram: Vec<u8>,
    pc: usize,
    i: usize,
    stack: Vec<usize>,
    registers: [u8; 16],
    delay_timer: u8,
    sound_timer: u8,
    key_pressed: Option<u8>,
    waiting_for_input: Option<u8>,
}

#[derive(Resource)]
struct Chip8Resource(Chip8);

/// Represents Chip8 instructions.
#[derive(Debug, Clone, Copy)]
enum Instruction {
    Cls00E0,
    SetIndexRegisterANNN(u16),
    SetVRegister6XNN(u8, u8),
    Dxyn(u8, u8, u8),
    Add7XNN(u8, u8),
    Jump1NNN(u16),
    SubroutineCall2NNN(u16),
    SubroutineReturn00EE,
    SkipEqual3XNN(u8, u8),
    SkipNotEqual4XNN(u8, u8),
    BinaryCodedDecimalConversionFX33(u8),
    FontCharacterFX29(u8),
    SetDelayTimerFX15(u8),
    ReadDelayTimerFX07(u8),
    GetKeyFX0A(u8),
    SetSoundTimerFX18(u8),
    AddToIndexFX1E(u8),
    StoreRegistersToMemoryFX55(u8),
    LoadRegistersFromMemoryFX65(u8),
    RandomCXNN(u8, u8),
    SkipIfKeyPressedEX9E(u8),
    SkipIfKeyNotPressedEXA1(u8),
    BinaryAnd8XY2(u8, u8),
    RegisterAdd8XY4(u8, u8),
    RegisterSet8XY0(u8, u8),
    RegisterSub8XY5(u8, u8),
    RegisterSubRev8XY7(u8, u8),
    ShiftRight8XY6(u8, u8),
    ShiftLeft8XYE(u8, u8),
    SkipIfEqual5XY0(u8, u8),
    SkipIfNotEqual9XY0(u8, u8),
    Xor8XY3(u8, u8),
}

impl Instruction {
    pub fn new(b1: u8, b2: u8) -> Result<Self> {
        let i = b1 >> 4;
        let x = b1 & 0xf;
        let y = b2 >> 4;
        let n = b2 & 0xf;
        let nn = b2;
        let nnn = u16::from_ne_bytes([nn, x]);
        let ins = match (i, x, y, n, nn, nnn) {
            (0, 0, 0xE, 0, _, _) => Instruction::Cls00E0,
            (0xA, _, _, _, _, nnn) => Instruction::SetIndexRegisterANNN(nnn),
            (1, _, _, _, _, nnn) => Instruction::Jump1NNN(nnn),
            (6, x, _, _, nn, _) => Instruction::SetVRegister6XNN(x, nn),
            (0xD, x, y, n, _, _) => Instruction::Dxyn(x, y, n),
            (2, _, _, _, _, nnn) => Instruction::SubroutineCall2NNN(nnn),
            (0, 0, 0xE, 0xE, _, _) => Instruction::SubroutineReturn00EE,
            (3, x, _, _, nn, _) => Instruction::SkipEqual3XNN(x, nn),
            (4, x, _, _, nn, _) => Instruction::SkipNotEqual4XNN(x, nn),
            (5, x, y, 0, _, _) => Instruction::SkipIfEqual5XY0(x, y),
            (9, x, y, 0, _, _) => Instruction::SkipIfNotEqual9XY0(x, y),
            (7, x, _, _, nn, _) => Instruction::Add7XNN(x, nn),
            (8, x, y, 3, _, _) => Instruction::Xor8XY3(x, y),
            (0xF, x, 3, 3, _, _) => Instruction::BinaryCodedDecimalConversionFX33(x),
            (0xF, x, 2, 9, _, _) => Instruction::FontCharacterFX29(x),
            (0xF, x, 1, 5, _, _) => Instruction::SetDelayTimerFX15(x),
            (0xF, x, 0, 7, _, _) => Instruction::ReadDelayTimerFX07(x),
            (0xF, x, 0, 0xA, _, _) => Instruction::GetKeyFX0A(x),
            (0xF, x, 1, 8, _, _) => Instruction::SetSoundTimerFX18(x),
            (0xF, x, 1, 0xE, _, _) => Instruction::AddToIndexFX1E(x),
            (0xF, x, 5, 5, _, _) => Instruction::StoreRegistersToMemoryFX55(x),
            (0xF, x, 6, 5, _, _) => Instruction::LoadRegistersFromMemoryFX65(x),
            (0xC, x, _, _, nn, _) => Instruction::RandomCXNN(x, nn),
            (0xE, x, 9, 0xE, _, _) => Instruction::SkipIfKeyPressedEX9E(x),
            (0xE, x, 0xA, 1, _, _) => Instruction::SkipIfKeyNotPressedEXA1(x),
            (8, x, y, 2, _, _) => Instruction::BinaryAnd8XY2(x, y),
            (8, x, y, 4, _, _) => Instruction::RegisterAdd8XY4(x, y),
            (8, x, y, 0, _, _) => Instruction::RegisterSet8XY0(x, y),
            (8, x, y, 5, _, _) => Instruction::RegisterSub8XY5(x, y),
            (8, x, y, 6, _, _) => Instruction::ShiftRight8XY6(x, y),
            (8, x, y, 0xE, _, _) => Instruction::ShiftLeft8XYE(x, y),
            (8, x, y, 7, _, _) => Instruction::RegisterSubRev8XY7(x, y),
            _ => {
                std::thread::sleep(Duration::from_secs(5));
                bail!("unimplemented instruction: {} {} {} {}", i, x, y, n)
            }
        };
        Ok(ins)
    }

    pub fn requires_pc_inc(&self) -> usize {
        match self {
            Self::SubroutineCall2NNN(_) => 0,
            Self::Jump1NNN(_) => 0,
            _ => 2,
        }
    }
}

impl Chip8 {
    /// Fetches and decodes Chip8 instructions from RAM.
    fn tick(&mut self, graphics: &mut impl Graphics) {
        if self.waiting_for_input.is_some() {
            return;
        }
        let inst = self
            .fetch_and_decode_next_instruction()
            .expect("instruction failure");
        self.execute_instruction(inst, graphics)
            .unwrap_or_else(|_| panic!("failed to execute instruction: {inst:?}"));
        self.pc += inst.requires_pc_inc();
        // if self.sound_timer > 0 {
        //     println!("beeeeeep!");
        // }
    }

    fn decrease_timers(&mut self) {
        self.delay_timer = self.delay_timer.saturating_sub(1);
        self.sound_timer = self.sound_timer.saturating_sub(1);
    }

    fn fetch_and_decode_next_instruction(&mut self) -> Result<Instruction> {
        let b1 = *self
            .ram
            .get(self.pc)
            .unwrap_or_else(|| panic!("invalid memory address: {}", self.pc));
        let b2 = *self
            .ram
            .get(self.pc + 1)
            .unwrap_or_else(|| panic!("invalid memory address: {}", self.pc));
        let inst = Instruction::new(b1, b2).context("failed to decode instruction")?;
        Ok(inst)
    }

    /// Executes the Chip8 instruction.
    fn execute_instruction(&mut self, inst: Instruction, graphics: &mut impl Graphics) -> Result<()> {
        debug!(
            "pc = {}, index = {}, registers = {:?}\n",
            self.pc, self.i, self.registers
        );
        debug!("{:?}", inst);
        match inst {
            Instruction::Cls00E0 => {
                for (y, row) in self.pixels.iter().enumerate() {
                    for (x, pixel) in row.iter().enumerate() {
                        if *pixel {
                            graphics
                                .clear_pixel(x, y)
                                .context("failed to clear pixel")?;
                        }
                    }
                }
                self.pixels = vec![vec![false; TERMINAL_WIDTH as usize]; TERMINAL_HEIGHT as usize];
            }
            Instruction::SetIndexRegisterANNN(nnn) => self.i = nnn as usize,
            Instruction::SetVRegister6XNN(x, nn) => self.registers[x as usize] = nn,
            Instruction::Dxyn(x, y, n) => {
                let x_org = (self.registers[x as usize] % TERMINAL_WIDTH) as usize;
                let mut y = (self.registers[y as usize] % TERMINAL_HEIGHT) as usize;
                self.registers[15] = 0;
                let mut collision = false;
                let sprites = &self.ram[self.i..(self.i + n as usize)];
                for row in sprites {
                    let mut x = x_org;
                    for i in (0..8).rev() {
                        let pixel = (row >> i) & 1;
                        if pixel == 1 {
                            let is_pixel_on =
                                self.is_pixel_on(x, y).context("failed to check pixel")?;
                            self.pixels[y][x] = !is_pixel_on;
                            if is_pixel_on {
                                graphics
                                    .clear_pixel(x, y)
                                    .context("failed to clear pixel")?;
                                collision = true;
                            } else {
                                graphics.draw_pixel(x, y).context("failed to draw pixel")?;
                            }
                        }
                        x += 1;
                        if x == (TERMINAL_WIDTH as usize) {
                            break;
                        }
                    }
                    y += 1;
                    if y == (TERMINAL_HEIGHT as usize) {
                        break;
                    }
                }
                if collision {
                    self.registers[15] = 1;
                }
            }
            Instruction::Add7XNN(x, nn) => {
                let (res, _) = self.registers[x as usize].overflowing_add(nn);
                self.registers[x as usize] = res;
            }
            Instruction::Jump1NNN(nnn) => self.pc = nnn.into(),
            Instruction::SubroutineCall2NNN(nnn) => {
                self.stack.push(self.pc);
                self.pc = nnn as usize;
            }
            Instruction::SubroutineReturn00EE => {
                self.pc = self
                    .stack
                    .pop()
                    .context("failed to return from subroutine: stack underflow")?;
            }
            Instruction::SkipEqual3XNN(x, nn) => {
                if self.registers[x as usize] == nn {
                    self.pc += 2;
                }
            }
            Instruction::SkipNotEqual4XNN(x, nn) => {
                if self.registers[x as usize] != nn {
                    self.pc += 2;
                }
            }
            Instruction::BinaryCodedDecimalConversionFX33(x) => {
                let val = self.registers[x as usize];
                self.ram[self.i] = val / 100;
                self.ram[self.i + 1] = (val % 100) / 10;
                self.ram[self.i + 2] = val % 10;
            }
            Instruction::FontCharacterFX29(x) => {
                self.i = FONT_ADDR + (self.registers[x as usize] as usize * FONT_SIZE)
            }
            Instruction::SetDelayTimerFX15(x) => {
                self.delay_timer = self.registers[x as usize];
            }
            Instruction::ReadDelayTimerFX07(x) => self.registers[x as usize] = self.delay_timer,
            Instruction::SetSoundTimerFX18(x) => self.sound_timer = self.registers[x as usize],
            Instruction::AddToIndexFX1E(x) => {
                let (res, overflow) = self.i.overflowing_add(self.registers[x as usize] as usize);
                self.i = res;
                if overflow {
                    self.registers[15] = 1;
                }
            }
            Instruction::StoreRegistersToMemoryFX55(x) => {
                let x = x as usize;
                self.ram[self.i..=self.i + x].copy_from_slice(&self.registers[0..=x]);
            }
            Instruction::LoadRegistersFromMemoryFX65(x) => {
                let x = x as usize;
                let data = &self.ram[self.i..=self.i + x];
                self.registers[0..=x].copy_from_slice(data);
            }
            Instruction::RandomCXNN(x, nn) => {
                let r: u8 = rand::random();
                self.registers[x as usize] = r & nn;
            }
            Instruction::SkipIfKeyPressedEX9E(x) => {
                if self.key_pressed == Some(self.registers[x as usize]) {
                    self.pc += 2;
                }
            }
            Instruction::SkipIfKeyNotPressedEXA1(x) => {
                if self.key_pressed != Some(self.registers[x as usize]) {
                    self.pc += 2;
                }
            }
            Instruction::BinaryAnd8XY2(x, y) => {
                self.registers[x as usize] &= self.registers[y as usize]
            }
            Instruction::RegisterAdd8XY4(x, y) => {
                let (res, carry) =
                    self.registers[x as usize].overflowing_add(self.registers[y as usize]);
                self.registers[x as usize] = res;
                self.registers[15] = carry as u8;
            }
            Instruction::RegisterSet8XY0(x, y) => {
                self.registers[x as usize] = self.registers[y as usize]
            }
            Instruction::RegisterSub8XY5(x, y) => {
                let (res, carry) =
                    self.registers[x as usize].overflowing_sub(self.registers[y as usize]);
                self.registers[x as usize] = res;
                self.registers[15] = !carry as u8;
            }
            Instruction::RegisterSubRev8XY7(x, y) => {
                let (res, carry) =
                    self.registers[y as usize].overflowing_sub(self.registers[x as usize]);
                self.registers[x as usize] = res;
                self.registers[15] = !carry as u8;
            }
            Instruction::GetKeyFX0A(x) => self.waiting_for_input = Some(x),
            Instruction::ShiftRight8XY6(x, _) => {
                self.registers[15] = self.registers[x as usize] & 1u8;
                self.registers[x as usize] >>= 1;
            }
            Instruction::ShiftLeft8XYE(x, _) => {
                self.registers[15] = self.registers[x as usize] & (1u8 << 7);
                self.registers[x as usize] <<= 1;
            }
            Instruction::SkipIfEqual5XY0(x, y) => {
                if self.registers[x as usize] == self.registers[y as usize] {
                    self.pc += 2
                }
            }
            Instruction::SkipIfNotEqual9XY0(x, y) => {
                if self.registers[x as usize] != self.registers[y as usize] {
                    self.pc += 2
                }
            }
            Instruction::Xor8XY3(x, y) => {
                self.registers[x as usize] ^= self.registers[y as usize];
            }
        }
        Ok(())
    }

    /// Returns true if the pixel at the coordinates is on, otherwise false.
    ///
    /// If the coordinates is out of the screen area it returns an Error.
    fn is_pixel_on(&self, x: usize, y: usize) -> Result<bool> {
        check_coordinates(x, y)?;
        Ok(self.pixels[y][x])
    }

    /// Stores data in RAM.
    ///
    /// If the data is bigger than the available space it returns Error.
    fn store_in_ram(&mut self, rom: impl AsRef<[u8]>) -> Result<()> {
        let rom = &rom.as_ref();
        if rom.len() + PROGRAM_START > RAM_SIZE {
            bail!("data is too big to fit into the ram");
        }
        self.ram[PROGRAM_START..PROGRAM_START + rom.len()].copy_from_slice(rom);
        Ok(())
    }

    pub fn new(clock: u64) -> Self {
        let mut ram = vec![0; RAM_SIZE];
        ram[FONT_ADDR..FONT_ADDR + FONTS.len()].copy_from_slice(&FONTS);
        Self {
            clock,
            pixels: vec![vec![false; TERMINAL_WIDTH as usize]; TERMINAL_HEIGHT as usize],
            ram,
            pc: PROGRAM_START,
            ..Default::default()
        }
    }

    fn handle_key_pressed(&mut self, key: u8) {
        self.key_pressed = Some(key);
        if let Some(x) = self.waiting_for_input {
            self.registers[x as usize] = key;
            self.waiting_for_input = None;
        }
    }
}

trait Graphics {
    /// Clears/turns off a pixel on a specific coordinate.
    ///
    /// If the coordinates is out of the screen area it returns an Error.
    fn clear_pixel(&mut self, x: usize, y: usize) -> Result<()>;

    /// Draws/turns on a pixel on a specific coordinate.
    ///
    /// If the coordinates is out of the screen area it returns an Error.
    fn draw_pixel(&mut self, x: usize, y: usize) -> Result<()>;
}

struct BevyGraphics<'w, 's> {
    commands: Commands<'w, 's>,
}

impl BevyGraphics<'_, '_> {
    /// Draws/turns on a pixel on a specific coordinate.
    ///
    /// If the coordinates is out of the screen area it returns an Error.
    fn draw_pixel(&mut self, x: usize, y: usize, color: Option<Color>) -> Result<()> {
        check_coordinates(x, y)?;
        let x = x as i32;
        let y = y as i32;
        let rectangle = SpriteBundle {
            sprite: Sprite {
                color: color.unwrap_or(Color::WHITE),
                custom_size: Some(Vec2::new(10.0, 10.0)),
                ..default()
            },
            transform: Transform::from_xyz(((x - 32) * 10) as f32, ((16 - y) * 10) as f32, 0.),
            ..default()
        };
        self.commands.spawn(rectangle);
        Ok(())
    }
}

impl Graphics for BevyGraphics<'_, '_> {
    fn clear_pixel(&mut self, x: usize, y: usize) -> Result<()> {
        self.draw_pixel(x, y, Some(Color::BLACK))
    }

    fn draw_pixel(&mut self, x: usize, y: usize) -> Result<()> {
        self.draw_pixel(x, y, None)
    }
}

fn setup(mut commands: Commands) {
    commands.spawn(Camera2dBundle::default());
}

#[derive(Resource)]
struct CPUClock(Timer);

#[derive(Resource)]
struct TimerClock(Timer);

fn tick(
    commands: Commands,
    time: Res<Time>,
    mut timer_clock: ResMut<TimerClock>,
    mut ch8: ResMut<Chip8Resource>,
) {
    if timer_clock.0.tick(time.delta()).just_finished() {
        ch8.0.decrease_timers();
        let mut graphics = BevyGraphics { commands };
        for _ in 0..12 {
            sleep(Duration::from_millis(1000 / ch8.0.clock));
            ch8.0.tick(&mut graphics);
        }
    }
}

fn keyboard_events(mut key_evr: EventReader<KeyboardInput>, mut ch8: ResMut<Chip8Resource>) {
    use bevy::input::ButtonState;

    let keymap = [
        (KeyCode::Key1),
        (KeyCode::Key2),
        (KeyCode::Key3),
        (KeyCode::Key4),
        (KeyCode::Q),
        (KeyCode::W),
        (KeyCode::E),
        (KeyCode::R),
        (KeyCode::A),
        (KeyCode::S),
        (KeyCode::D),
        (KeyCode::F),
        (KeyCode::Z),
        (KeyCode::X),
        (KeyCode::C),
        (KeyCode::V),
    ]
    .iter()
    // .zip(0u8..16)
    .zip([1, 2, 3, 12, 4, 5, 6, 13, 7, 8, 9, 14, 10, 0, 11, 15])
    .collect::<HashMap<_, _>>();

    for ev in key_evr.iter() {
        match ev.state {
            ButtonState::Pressed => {
                if let k @ Some(
                    KeyCode::Key1
                    | KeyCode::Key2
                    | KeyCode::Key3
                    | KeyCode::Key4
                    | KeyCode::Q
                    | KeyCode::W
                    | KeyCode::E
                    | KeyCode::R
                    | KeyCode::A
                    | KeyCode::S
                    | KeyCode::D
                    | KeyCode::F
                    | KeyCode::Z
                    | KeyCode::X
                    | KeyCode::C
                    | KeyCode::V,
                ) = ev.key_code
                {
                    ch8.0.handle_key_pressed(keymap.get(&k.unwrap()).cloned().unwrap());
                }
            }
            ButtonState::Released => ch8.0.key_pressed = None,
        }
    }
}

fn check_coordinates(x: usize, y: usize) -> Result<()> {
    if x >= TERMINAL_WIDTH.into() {
        bail!("invalid X coordinate to draw: {}", x);
    }
    if y >= TERMINAL_HEIGHT.into() {
        bail!("invalid Y coordinate to draw: {}", y);
    }
    Ok(())
}

/// Reads rom into the buffer
fn read_rom(path: &impl AsRef<Path>) -> Result<Vec<u8>> {
    let mut data = vec![];
    let rom_size = File::open(path)
        .context("Failed to open rom file")?
        .read_to_end(&mut data)
        .context("Failed to read rom file")?;
    info!("rom size = {}", rom_size);
    Ok(data)
}

#[derive(Debug, StructOpt)]
struct Opt {
    #[structopt(name = "ROM_FILE_PATH", parse(from_os_str))]
    rom: PathBuf,
    #[structopt(short, long, default_value = "700")]
    clock: u64,
}

fn main() -> Result<()> {
    // env_logger::init();

    let opt = Opt::from_args();

    let rom = read_rom(&opt.rom)?;

    let mut ch8 = Chip8::new(opt.clock);

    ch8.store_in_ram(rom)
        .context("failed to store rom into the ram")?;
    info!("loaded rom size = {}", ch8.ram.len());

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            window: WindowDescriptor {
                title: "Chip8".to_string(),
                width: (TERMINAL_WIDTH as u16 * 10).into(),
                height: (TERMINAL_HEIGHT as u16 * 10).into(),
                present_mode: PresentMode::AutoVsync,
                transparent: true,
                ..default()
            },
            ..default()
        }))
        .add_plugin(FrameTimeDiagnosticsPlugin::default())
        .insert_resource(Chip8Resource(ch8))
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(TimerClock(Timer::new(
            Duration::from_millis(1000 / FPS),
            TimerMode::Repeating,
        )))
        .add_startup_system(setup)
        .add_system(keyboard_events)
        .add_system(tick)
        .run();

    Ok(())
}
