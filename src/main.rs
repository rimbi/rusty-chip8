use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    vec,
};

use anyhow::{bail, Context, Ok, Result};
use bracket_terminal::prelude::*;
use log::info;
use structopt::StructOpt;

const TERMINAL_WIDTH: u8 = 64;
const TERMINAL_HEIGHT: u8 = 32;
const RAM_SIZE: usize = 4096;

#[derive(Debug, StructOpt)]
struct Opt {
    #[structopt(name = "ROM_FILE_PATH", parse(from_os_str))]
    rom: PathBuf,
}

/// Chip8 emulator state.
#[derive(Debug, Default)]
struct Chip8 {
    pixels: Vec<Vec<bool>>,
    ram: Vec<u8>,
    pc: usize,
    i: u16,
    // stack: Vec<u16>,
    registers: [u8; 16],
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
            _ => bail!("unimplemented instruction: {} {} {} {}", i, x, y, n),
        };
        Ok(ins)
    }
}

impl GameState for Chip8 {
    fn tick(&mut self, ctx: &mut bracket_terminal::prelude::BTerm) {
        let inst = self
            .fetch_and_decode_next_instruction()
            .expect("instruction failure");
        self.execute_instruction(inst, ctx)
            .unwrap_or_else(|_| panic!("failed to execute instruction: {:?}", inst));
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
        self.pc += 2;
        Ok(inst)
    }

    /// Executes the Chip8 instruction.
    fn execute_instruction(&mut self, inst: Instruction, ctx: &mut BTerm) -> Result<()> {
        match inst {
            Instruction::Cls => ctx.cls(),
            Instruction::SetIndexRegister(nnn) => self.i = nnn,
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
        Self {
            pixels: vec![vec![false; 64]; 32],
            ram: vec![0; 512],
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
        .with_fps_cap(700.)
        .build()?;
    main_loop(context, ch8)
}
