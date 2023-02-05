#![deny(clippy::all)]
#![warn(clippy::pedantic)]
#![deny(clippy::as_conversions)]
#![warn(clippy::nursery)]
#![warn(clippy::cargo)]

use std::{thread::sleep, time::Duration, vec};

use anyhow::{bail, Context, Ok, Result};
use log::debug;

/// Number of horizontal sprites.
pub const TERMINAL_WIDTH: usize = 64;
/// Number of vertical sprites.
pub const TERMINAL_HEIGHT: usize = 32;
/// Frame rate per second.
pub const FPS: u64 = 60;
const RAM_SIZE: usize = 4096;
const PROGRAM_START: usize = 512;

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

/// Chip8 emulator.
#[derive(Debug, Default)]
pub struct Chip8 {
    clock: u64,
    pixels: Vec<Vec<bool>>,
    ram: Vec<u8>,
    pc: usize,
    i: usize,
    stack: Vec<usize>,
    registers: [u8; 16],
    delay_timer: u8,
    sound_timer: u8,
    beeping: bool,
    key_pressed: Option<u8>,
    waiting_for_input: Option<usize>,
}

/// Represents Chip8 instructions.
#[derive(Debug, Clone, Copy)]
enum Instruction {
    Cls00E0,
    SetIndexRegisterANNN(usize),
    SetVRegister6XNN(usize, u8),
    Dxyn(usize, usize, usize),
    Add7XNN(usize, u8),
    Jump1NNN(u16),
    SubroutineCall2NNN(u16),
    SubroutineReturn00EE,
    SkipEqual3XNN(usize, u8),
    SkipNotEqual4XNN(usize, u8),
    BinaryCodedDecimalConversionFX33(usize),
    FontCharacterFX29(usize),
    SetDelayTimerFX15(usize),
    ReadDelayTimerFX07(usize),
    GetKeyFX0A(usize),
    SetSoundTimerFX18(usize),
    AddToIndexFX1E(usize),
    StoreRegistersToMemoryFX55(usize),
    LoadRegistersFromMemoryFX65(usize),
    RandomCXNN(usize, u8),
    SkipIfKeyPressedEX9E(usize),
    SkipIfKeyNotPressedEXA1(usize),
    BinaryAnd8XY2(usize, usize),
    RegisterAdd8XY4(usize, usize),
    RegisterSet8XY0(usize, usize),
    RegisterSub8XY5(usize, usize),
    RegisterSubRev8XY7(usize, usize),
    ShiftRight8XY6(usize, usize),
    ShiftLeft8XYE(usize, usize),
    SkipIfEqual5XY0(usize, usize),
    SkipIfNotEqual9XY0(usize, usize),
    Xor8XY3(usize, usize),
}

impl Instruction {
    fn new(b1: u8, b2: u8) -> Result<Self> {
        let i = b1 >> 4;
        let x = b1 & 0xf;
        let y = b2 >> 4;
        let n = usize::from(b2 & 0xf);
        let nn = b2;
        let nnn = u16::from_ne_bytes([nn, x]);
        let x = usize::from(x);
        let y = usize::from(y);
        let ins = match (i, x, y, n, nn, nnn) {
            (0, 0, 0xE, 0, _, _) => Self::Cls00E0,
            (0xA, _, _, _, _, nnn) => Self::SetIndexRegisterANNN(nnn.into()),
            (1, _, _, _, _, nnn) => Self::Jump1NNN(nnn),
            (6, x, _, _, nn, _) => Self::SetVRegister6XNN(x, nn),
            (0xD, x, y, n, _, _) => Self::Dxyn(x, y, n),
            (2, _, _, _, _, nnn) => Self::SubroutineCall2NNN(nnn),
            (0, 0, 0xE, 0xE, _, _) => Self::SubroutineReturn00EE,
            (3, x, _, _, nn, _) => Self::SkipEqual3XNN(x, nn),
            (4, x, _, _, nn, _) => Self::SkipNotEqual4XNN(x, nn),
            (5, x, y, 0, _, _) => Self::SkipIfEqual5XY0(x, y),
            (9, x, y, 0, _, _) => Self::SkipIfNotEqual9XY0(x, y),
            (7, x, _, _, nn, _) => Self::Add7XNN(x, nn),
            (8, x, y, 3, _, _) => Self::Xor8XY3(x, y),
            (0xF, x, 3, 3, _, _) => Self::BinaryCodedDecimalConversionFX33(x),
            (0xF, x, 2, 9, _, _) => Self::FontCharacterFX29(x),
            (0xF, x, 1, 5, _, _) => Self::SetDelayTimerFX15(x),
            (0xF, x, 0, 7, _, _) => Self::ReadDelayTimerFX07(x),
            (0xF, x, 0, 0xA, _, _) => Self::GetKeyFX0A(x),
            (0xF, x, 1, 8, _, _) => Self::SetSoundTimerFX18(x),
            (0xF, x, 1, 0xE, _, _) => Self::AddToIndexFX1E(x),
            (0xF, x, 5, 5, _, _) => Self::StoreRegistersToMemoryFX55(x),
            (0xF, x, 6, 5, _, _) => Self::LoadRegistersFromMemoryFX65(x),
            (0xC, x, _, _, nn, _) => Self::RandomCXNN(x, nn),
            (0xE, x, 9, 0xE, _, _) => Self::SkipIfKeyPressedEX9E(x),
            (0xE, x, 0xA, 1, _, _) => Self::SkipIfKeyNotPressedEXA1(x),
            (8, x, y, 2, _, _) => Self::BinaryAnd8XY2(x, y),
            (8, x, y, 4, _, _) => Self::RegisterAdd8XY4(x, y),
            (8, x, y, 0, _, _) => Self::RegisterSet8XY0(x, y),
            (8, x, y, 5, _, _) => Self::RegisterSub8XY5(x, y),
            (8, x, y, 6, _, _) => Self::ShiftRight8XY6(x, y),
            (8, x, y, 0xE, _, _) => Self::ShiftLeft8XYE(x, y),
            (8, x, y, 7, _, _) => Self::RegisterSubRev8XY7(x, y),
            _ => {
                std::thread::sleep(Duration::from_secs(5));
                bail!("unimplemented instruction: {} {} {} {}", i, x, y, n)
            }
        };
        Ok(ins)
    }

    const fn requires_pc_inc(self) -> usize {
        match self {
            Self::SubroutineCall2NNN(_) | Self::Jump1NNN(_) => 0,
            _ => 2,
        }
    }
}

impl Chip8 {
    /// Returns a Chip8 instance.
    ///
    /// # Arguments
    ///
    /// * `clock` - refers to the instructions per second. The common value used is `700`.
    #[must_use]
    pub fn new(clock: u64) -> Self {
        let mut ram = vec![0; RAM_SIZE];
        ram[FONT_ADDR..FONT_ADDR + FONTS.len()].copy_from_slice(&FONTS);
        Self {
            clock,
            pixels: vec![vec![false; TERMINAL_WIDTH]; TERMINAL_HEIGHT],
            ram,
            pc: PROGRAM_START,
            ..Default::default()
        }
    }

    /// Fetches, decodes and executes Chip8 instructions from RAM.
    ///
    /// This function is supposed to be called [FPS] times per second.
    ///
    /// # Panics
    ///
    /// Panics when an invalid (or unimplemented) instruction encountered.
    pub fn tick(&mut self, graphics: &mut impl Graphics, audio: &mut impl Audio) {
        self.decrease_timers();
        for _ in 0..self.clock / FPS {
            sleep(Duration::from_millis(1000 / self.clock));
            if self.waiting_for_input.is_some() {
                return;
            }
            let inst = self
                .fetch_and_decode_next_instruction()
                .expect("instruction failure");
            self.execute_instruction(inst, graphics)
                .unwrap_or_else(|_| panic!("failed to execute instruction: {inst:?}"));
            self.pc += inst.requires_pc_inc();
            if self.sound_timer > 0 && !self.beeping {
                audio.start_beep();
                self.beeping = true;
            } else if self.sound_timer == 0 && self.beeping {
                audio.stop_beep();
                self.beeping = false;
            }
        }
    }

    /// Decreases sound and delay timers.
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
    #[allow(clippy::too_many_lines)]
    fn execute_instruction(
        &mut self,
        inst: Instruction,
        graphics: &mut impl Graphics,
    ) -> Result<()> {
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
                            graphics.clear_pixel(x, y);
                        }
                    }
                }
                self.pixels = vec![vec![false; TERMINAL_WIDTH]; TERMINAL_HEIGHT];
            }
            Instruction::SetIndexRegisterANNN(nnn) => self.i = nnn,
            Instruction::SetVRegister6XNN(x, nn) => self.registers[x] = nn,
            Instruction::Dxyn(x, y, n) => {
                let x_org = usize::from(self.registers[x]) % TERMINAL_WIDTH;
                let mut y = usize::from(self.registers[y]) % TERMINAL_HEIGHT;
                self.registers[15] = 0;
                let mut collision = false;
                let sprites = &self.ram[self.i..self.i + n];
                for row in sprites {
                    let mut x = x_org;
                    for i in (0..8).rev() {
                        let pixel = (row >> i) & 1;
                        if pixel == 1 {
                            let is_pixel_on =
                                self.is_pixel_on(x, y).context("failed to check pixel")?;
                            self.pixels[y][x] = !is_pixel_on;
                            if is_pixel_on {
                                graphics.clear_pixel(x, y);
                                collision = true;
                            } else {
                                graphics.draw_pixel(x, y);
                            }
                        }
                        x += 1;
                        if x == TERMINAL_WIDTH {
                            break;
                        }
                    }
                    y += 1;
                    if y == TERMINAL_HEIGHT {
                        break;
                    }
                }
                if collision {
                    self.registers[15] = 1;
                }
            }
            Instruction::Add7XNN(x, nn) => {
                let (res, _) = self.registers[x].overflowing_add(nn);
                self.registers[x] = res;
            }
            Instruction::Jump1NNN(nnn) => self.pc = nnn.into(),
            Instruction::SubroutineCall2NNN(nnn) => {
                self.stack.push(self.pc);
                self.pc = usize::from(nnn);
            }
            Instruction::SubroutineReturn00EE => {
                self.pc = self
                    .stack
                    .pop()
                    .context("failed to return from subroutine: stack underflow")?;
            }
            Instruction::SkipEqual3XNN(x, nn) => {
                if self.registers[x] == nn {
                    self.pc += 2;
                }
            }
            Instruction::SkipNotEqual4XNN(x, nn) => {
                if self.registers[x] != nn {
                    self.pc += 2;
                }
            }
            Instruction::BinaryCodedDecimalConversionFX33(x) => {
                let val = self.registers[x];
                self.ram[self.i] = val / 100;
                self.ram[self.i + 1] = (val % 100) / 10;
                self.ram[self.i + 2] = val % 10;
            }
            Instruction::FontCharacterFX29(x) => {
                self.i = FONT_ADDR + (usize::from(self.registers[x]) * FONT_SIZE);
            }
            Instruction::SetDelayTimerFX15(x) => {
                self.delay_timer = self.registers[x];
            }
            Instruction::ReadDelayTimerFX07(x) => self.registers[x] = self.delay_timer,
            Instruction::SetSoundTimerFX18(x) => self.sound_timer = self.registers[x],
            Instruction::AddToIndexFX1E(x) => {
                let (res, overflow) = self.i.overflowing_add(self.registers[x].into());
                self.i = res;
                if overflow {
                    self.registers[15] = 1;
                }
            }
            Instruction::StoreRegistersToMemoryFX55(x) => {
                self.ram[self.i..=self.i + x].copy_from_slice(&self.registers[0..=x]);
            }
            Instruction::LoadRegistersFromMemoryFX65(x) => {
                let data = &self.ram[self.i..=self.i + x];
                self.registers[0..=x].copy_from_slice(data);
            }
            Instruction::RandomCXNN(x, nn) => {
                let r: u8 = rand::random();
                self.registers[x] = r & nn;
            }
            Instruction::SkipIfKeyPressedEX9E(x) => {
                if self.key_pressed == Some(self.registers[x]) {
                    self.pc += 2;
                }
            }
            Instruction::SkipIfKeyNotPressedEXA1(x) => {
                if self.key_pressed != Some(self.registers[x]) {
                    self.pc += 2;
                }
            }
            Instruction::BinaryAnd8XY2(x, y) => {
                self.registers[x] &= self.registers[y];
            }
            Instruction::RegisterAdd8XY4(x, y) => {
                let (res, carry) = self.registers[x].overflowing_add(self.registers[y]);
                self.registers[x] = res;
                self.registers[15] = u8::from(carry);
            }
            Instruction::RegisterSet8XY0(x, y) => {
                self.registers[x] = self.registers[y];
            }
            Instruction::RegisterSub8XY5(x, y) => {
                let (res, carry) = self.registers[x].overflowing_sub(self.registers[y]);
                self.registers[x] = res;
                self.registers[15] = u8::from(!carry);
            }
            Instruction::RegisterSubRev8XY7(x, y) => {
                let (res, carry) = self.registers[y].overflowing_sub(self.registers[x]);
                self.registers[x] = res;
                self.registers[15] = u8::from(!carry);
            }
            Instruction::GetKeyFX0A(x) => self.waiting_for_input = Some(x),
            Instruction::ShiftRight8XY6(x, _) => {
                self.registers[15] = self.registers[x] & 1u8;
                self.registers[x] >>= 1;
            }
            Instruction::ShiftLeft8XYE(x, _) => {
                self.registers[15] = self.registers[x] & (1u8 << 7);
                self.registers[x] <<= 1;
            }
            Instruction::SkipIfEqual5XY0(x, y) => {
                if self.registers[x] == self.registers[y] {
                    self.pc += 2;
                }
            }
            Instruction::SkipIfNotEqual9XY0(x, y) => {
                if self.registers[x] != self.registers[y] {
                    self.pc += 2;
                }
            }
            Instruction::Xor8XY3(x, y) => {
                self.registers[x] ^= self.registers[y];
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
    /// # Errors
    ///
    /// If the data is bigger than the available space it returns Error.
    pub fn store_in_ram(&mut self, rom: impl AsRef<[u8]>) -> Result<()> {
        let rom = &rom.as_ref();
        if rom.len() + PROGRAM_START > RAM_SIZE {
            bail!("data is too big to fit into the ram");
        }
        self.ram[PROGRAM_START..PROGRAM_START + rom.len()].copy_from_slice(rom);
        Ok(())
    }

    /// Handles released key.
    ///
    /// The real key press/release logic is supposed to be handled by the client.
    pub fn handle_key_released(&mut self) {
        self.key_pressed = None;
    }

    /// Handles pressed key.
    ///
    /// The real key press/release logic is supposed to be handled by the client.
    ///
    /// # Arguments
    ///
    /// * `key` - The key is supposed to be a value in the range `0..16`.
    ///     Chip8's original keypad has 16 buttons.
    pub fn handle_key_pressed(&mut self, key: u8) {
        self.key_pressed = Some(key);
        if let Some(x) = self.waiting_for_input {
            self.registers[x] = key;
            self.waiting_for_input = None;
        }
    }
}

/// Checks if the coordinates are valid.
fn check_coordinates(x: usize, y: usize) -> Result<()> {
    if x >= TERMINAL_WIDTH {
        bail!("invalid X coordinate to draw: {}", x);
    }
    if y >= TERMINAL_HEIGHT {
        bail!("invalid Y coordinate to draw: {}", y);
    }
    Ok(())
}

/// Graphics abstraction for Chip8.
///
/// Clients are supposed to implement this trait in accordance with
/// the graphics library used.
pub trait Graphics {
    /// Clears/turns off a pixel on a specific coordinate.
    fn clear_pixel(&mut self, x: usize, y: usize);

    /// Draws/turns on a pixel on a specific coordinate.
    fn draw_pixel(&mut self, x: usize, y: usize);
}

/// Audio abstraction for Chip8.
///
/// Clients are supposed to implement this trait in accordance with
/// the sound library used.
pub trait Audio {
    /// Starts the beep sound.
    fn start_beep(&mut self);

    /// Stops the beep sound.
    fn stop_beep(&mut self);
}
