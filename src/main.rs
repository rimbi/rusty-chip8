use std::{
    collections::HashMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
    vec,
};

use anyhow::{bail, Context, Ok, Result};
use bracket_terminal::prelude::*;
use log::info;
use structopt::StructOpt;

const TERMINAL_WIDTH: u8 = 64;
const TERMINAL_HEIGHT: u8 = 32;
const RAM_SIZE: usize = 4096;

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
#[derive(Debug)]
struct Chip8 {
    pixels: Vec<Vec<bool>>,
    ram: Vec<u8>,
    pc: usize,
    i: usize,
    stack: Vec<usize>,
    registers: [u8; 16],
    delay_timer: u8,
    sound_timer: u8,
    last_time: Instant,
    key_pressed: Option<u8>,
}

impl Default for Chip8 {
    fn default() -> Self {
        Self {
            pixels: Default::default(),
            ram: Default::default(),
            pc: Default::default(),
            i: Default::default(),
            stack: Default::default(),
            registers: Default::default(),
            delay_timer: Default::default(),
            sound_timer: Default::default(),
            last_time: Instant::now(),
            key_pressed: Default::default(),
        }
    }
}

/// Represents Chip8 instructions.
#[derive(Debug, Clone, Copy)]
enum Instruction {
    Cls,
    SetIndexRegister(u16),
    SetVRegister(u8, u8),
    Dxyn(u8, u8, u8),
    Add(u8, u8),
    Jump(u16),
    SubroutineCall(u16),
    SubroutineReturn,
    SkipEqual(u8, u8),
    SkipNotEqual(u8, u8),
    BinaryCodedDecimalConversion(u8),
    LoadRegisters(u8),
    FontCharacter(u8),
    SetDelayTimer(u8),
    ReadDelayTimer(u8),
    SetSoundTimer(u8),
    Random(u8, u8),          // CXNN
    SkipIfKeyPressed(u8),    // EX9E
    SkipIfKeyNotPressed(u8), // EXA1
    BinaryAnd(u8, u8),       // 8XY2
    RegisterAdd(u8, u8),     // 8XY4
    RegisterSet(u8, u8),     // 8XY0
    RegisterSub(u8, u8),     // 8XY5
    RegisterSubRev(u8, u8),  // 8XY7
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
            (0, 0, 0xE, 0, _, _) => Instruction::Cls,
            (0xA, _, _, _, _, nnn) => Instruction::SetIndexRegister(nnn),
            (1, _, _, _, _, nnn) => Instruction::Jump(nnn),
            (6, x, _, _, nn, _) => Instruction::SetVRegister(x, nn),
            (0xD, x, y, n, _, _) => Instruction::Dxyn(x, y, n),
            (7, x, _, _, nn, _) => Instruction::Add(x, nn),
            (2, _, _, _, _, nnn) => Instruction::SubroutineCall(nnn),
            (0, 0, 0xE, 0xE, _, _) => Instruction::SubroutineReturn,
            (3, x, _, _, nn, _) => Instruction::SkipEqual(x, nn),
            (4, x, _, _, nn, _) => Instruction::SkipNotEqual(x, nn),
            (0xF, x, 3, 3, _, _) => Instruction::BinaryCodedDecimalConversion(x),
            (0xF, x, 6, 5, _, _) => Instruction::LoadRegisters(x),
            (0xF, x, 2, 9, _, _) => Instruction::FontCharacter(x),
            (0xF, x, 1, 5, _, _) => Instruction::SetDelayTimer(x),
            (0xF, x, 0, 7, _, _) => Instruction::ReadDelayTimer(x),
            (0xF, x, 1, 8, _, _) => Instruction::SetSoundTimer(x),
            (0xC, x, _, _, nn, _) => Instruction::Random(x, nn),
            (0xE, x, 9, 0xE, _, _) => Instruction::SkipIfKeyPressed(x),
            (0xE, x, 0xA, 1, _, _) => Instruction::SkipIfKeyNotPressed(x),
            (8, x, y, 2, _, _) => Instruction::BinaryAnd(x, y),
            (8, x, y, 4, _, _) => Instruction::RegisterAdd(x, y),
            (8, x, y, 0, _, _) => Instruction::RegisterSet(x, y),
            (8, x, y, 5, _, _) => Instruction::RegisterSub(x, y),
            (8, x, y, 7, _, _) => Instruction::RegisterSubRev(x, y),
            _ => {
                std::thread::sleep(Duration::from_secs(5));
                bail!("unimplemented instruction: {} {} {} {}", i, x, y, n)
            }
        };
        Ok(ins)
    }

    pub fn requires_pc_inc(&self) -> usize {
        match self {
            Self::SubroutineCall(_) => 0,
            Self::SubroutineReturn => 4,
            Self::Jump(_) => 0,
            _ => 2,
        }
    }
}

impl GameState for Chip8 {
    fn tick(&mut self, ctx: &mut bracket_terminal::prelude::BTerm) {
        if self.last_time.elapsed().as_millis() > (1000. / 60.) as u128 {
            self.handle_keys(ctx);
            self.last_time = Instant::now();
            self.delay_timer = self.delay_timer.saturating_sub(1);
            self.sound_timer = self.sound_timer.saturating_sub(1);
        }
        let inst = self
            .fetch_and_decode_next_instruction()
            .expect("instruction failure");
        // println!("oc = {}, inst = {:?}", self.pc, inst);
        self.execute_instruction(inst, ctx)
            .unwrap_or_else(|_| panic!("failed to execute instruction: {:?}", inst));
        self.pc += inst.requires_pc_inc();
    }
}

impl Chip8 {
    /// Fetches and decodes Chip8 instructions from RAM.
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
    fn execute_instruction(&mut self, inst: Instruction, ctx: &mut BTerm) -> Result<()> {
        match inst {
            Instruction::Cls => ctx.cls(),
            Instruction::SetIndexRegister(nnn) => self.i = nnn as usize,
            Instruction::SetVRegister(x, nn) => self.registers[x as usize] = nn,
            Instruction::Dxyn(x, y, n) => {
                let x_org = (self.registers[x as usize] % TERMINAL_WIDTH) as usize;
                let mut y = (self.registers[y as usize] % TERMINAL_HEIGHT) as usize;
                self.registers[15] = 0;
                for row in &self.ram[(self.i as usize)..(self.i as usize + n as usize)] {
                    let mut x = x_org;
                    for i in (0..8).rev() {
                        let pixel = (row >> i) & 1;
                        if pixel == 1 {
                            let is_pixel_on =
                                self.is_pixel_on(x, y).context("failed to check pixel")?;
                            self.pixels[y][x] = !is_pixel_on;
                            if is_pixel_on {
                                self.clear_pixel(ctx, x, y)
                                    .context("failed to clear pixel")?;
                                self.registers[15] = 1;
                            } else {
                                self.draw_pixel(ctx, x, y).context("failed to draw pixel")?;
                            }
                        }
                        x += 1;
                        if x == TERMINAL_WIDTH.into() {
                            break;
                        }
                    }
                    y += 1;
                    if y == TERMINAL_HEIGHT.into() {
                        break;
                    }
                }
            }
            Instruction::Add(x, nn) => {
                self.registers[x as usize] = self.registers[x as usize].saturating_add(nn)
            }
            Instruction::Jump(nnn) => self.pc = nnn.into(),
            Instruction::SubroutineCall(nnn) => {
                self.stack.push(self.pc);
                self.pc = nnn as usize;
            }
            Instruction::SubroutineReturn => {
                self.pc = self
                    .stack
                    .pop()
                    .context("failed to return from subroutine: stack underflow")?;
            }
            Instruction::SkipEqual(x, nn) => {
                // println!("v{} = {}", x, self.registers[x as usize]);
                if self.registers[x as usize] == nn {
                    self.pc += 2;
                }
            }
            Instruction::SkipNotEqual(x, nn) => {
                if self.registers[x as usize] != nn {
                    self.pc += 2;
                }
            }
            Instruction::BinaryCodedDecimalConversion(x) => {
                let val = self.registers[x as usize];
                self.ram[self.i] = val / 100;
                self.ram[self.i + 1] = (val % 100) / 10;
                self.ram[self.i + 2] = val % 10;
            }
            Instruction::LoadRegisters(x) => {
                self.registers[0..=x as usize]
                    .copy_from_slice(&self.ram[self.i..=self.i + x as usize]);
            }
            Instruction::FontCharacter(x) => {
                self.i = FONT_ADDR + (self.registers[x as usize] as usize * FONT_SIZE)
            }
            Instruction::SetDelayTimer(x) => self.delay_timer = self.registers[x as usize],
            Instruction::ReadDelayTimer(x) => self.registers[x as usize] = self.delay_timer,
            Instruction::SetSoundTimer(x) => self.sound_timer = self.registers[x as usize],
            Instruction::Random(x, nn) => {
                let r: u8 = rand::random();
                self.registers[x as usize] = r & nn;
            }
            Instruction::SkipIfKeyPressed(x) => {
                if self.key_pressed == Some(self.registers[x as usize]) {
                    self.pc += 2;
                }
            }
            Instruction::SkipIfKeyNotPressed(x) => {
                if self.key_pressed != Some(self.registers[x as usize]) {
                    self.pc += 2;
                }
            }
            Instruction::BinaryAnd(x, y) => {
                self.registers[x as usize] &= self.registers[y as usize]
            }
            Instruction::RegisterAdd(x, y) => {
                let (res, carry) =
                    self.registers[x as usize].overflowing_add(self.registers[y as usize]);
                self.registers[x as usize] = res;
                self.registers[15] = carry as u8;
            }
            Instruction::RegisterSet(x, y) => {
                self.registers[x as usize] = self.registers[y as usize]
            }
            Instruction::RegisterSub(x, y) => {
                let (res, carry) =
                    self.registers[x as usize].overflowing_sub(self.registers[y as usize]);
                self.registers[x as usize] = res;
                self.registers[15] = !carry as u8;
            }
            Instruction::RegisterSubRev(x, y) => {
                let (res, carry) =
                    self.registers[y as usize].overflowing_sub(self.registers[x as usize]);
                self.registers[x as usize] = res;
                self.registers[15] = !carry as u8;
            }
        }
        Ok(())
    }

    /// Handles and reacts to the key strokes by the player.
    fn handle_keys(&mut self, ctx: &mut BTerm) {
        let keymap = [
            (VirtualKeyCode::Key1),
            (VirtualKeyCode::Key2),
            (VirtualKeyCode::Key3),
            (VirtualKeyCode::Key4),
            (VirtualKeyCode::Q),
            (VirtualKeyCode::W),
            (VirtualKeyCode::E),
            (VirtualKeyCode::R),
            (VirtualKeyCode::A),
            (VirtualKeyCode::S),
            (VirtualKeyCode::D),
            (VirtualKeyCode::F),
            (VirtualKeyCode::Z),
            (VirtualKeyCode::X),
            (VirtualKeyCode::C),
            (VirtualKeyCode::V),
        ]
        .iter()
        .zip(0u8..16)
        .collect::<HashMap<_, _>>();
        match ctx.key {
            k @ Some(
                VirtualKeyCode::Key1
                | VirtualKeyCode::Key2
                | VirtualKeyCode::Key3
                | VirtualKeyCode::Key4
                | VirtualKeyCode::Q
                | VirtualKeyCode::W
                | VirtualKeyCode::E
                | VirtualKeyCode::R
                | VirtualKeyCode::A
                | VirtualKeyCode::S
                | VirtualKeyCode::D
                | VirtualKeyCode::F
                | VirtualKeyCode::Z
                | VirtualKeyCode::X
                | VirtualKeyCode::C
                | VirtualKeyCode::V,
            ) => self.key_pressed = keymap.get(&k.unwrap()).cloned(),
            None => self.key_pressed = None,
            _ => {}
        }
        if self.key_pressed.is_some() {
            println!("key pressed = {:?}", self.key_pressed);
        }
    }

    /// Returns true if the pixel at the coordinates is on, otherwise false.
    ///
    /// If the coordinates is out of the screen area it returns an Error.
    fn is_pixel_on(&self, x: usize, y: usize) -> Result<bool> {
        check_coordinates(x, y)?;
        Ok(self.pixels[y][x])
    }

    /// Clears/turns off a pixel on a specific coordinate.
    ///
    /// If the coordinates is out of the screen area it returns an Error.
    fn clear_pixel(&self, ctx: &mut BTerm, x: usize, y: usize) -> Result<()> {
        check_coordinates(x, y)?;
        ctx.print(x, y, " ");
        Ok(())
    }

    /// Draws/turns on a pixel on a specific coordinate.
    ///
    /// If the coordinates is out of the screen area it returns an Error.
    fn draw_pixel(&self, ctx: &mut BTerm, x: usize, y: usize) -> Result<()> {
        check_coordinates(x, y)?;
        ctx.print(x, y, "#");
        Ok(())
    }

    /// Stores data in RAM.
    ///
    /// If the data is bigger than the available space it returns Error.
    fn store_in_ram(&mut self, rom: impl AsRef<[u8]>) -> Result<()> {
        let rom = &rom.as_ref();
        if rom.len() + self.ram.len() > RAM_SIZE {
            bail!("data is too big to fit into the ram");
        }
        self.ram.extend(*rom);
        Ok(())
    }

    pub fn new() -> Self {
        let mut ram = vec![0; 512];
        ram[FONT_ADDR..FONT_ADDR + FONTS.len()].copy_from_slice(&FONTS);
        Self {
            pixels: vec![vec![false; 64]; 32],
            ram,
            pc: 512,
            ..Default::default()
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

fn main() -> BError {
    env_logger::init();

    let opt = Opt::from_args();

    let rom = read_rom(&opt.rom)?;

    let mut ch8 = Chip8::new();

    ch8.store_in_ram(rom)
        .context("failed to store rom into the ram")?;
    info!("loaded rom size = {}", ch8.ram.len());

    let context = BTermBuilder::simple(TERMINAL_WIDTH, TERMINAL_HEIGHT)
        .map_err(|_| anyhow::Error::msg("Failed to create the window"))?
        .with_title("Chip8")
        .with_tile_dimensions(14, 14)
        .with_fps_cap(opt.clock as f32)
        .build()?;
    main_loop(context, ch8)
}
