use anyhow::{Context, Ok, Result};
use bevy::{
    diagnostic::FrameTimeDiagnosticsPlugin, input::keyboard::KeyboardInput, prelude::*,
    window::PresentMode,
};
use log::info;
use rusty_chip8::{Audio, Chip8, Graphics, FPS, TERMINAL_HEIGHT, TERMINAL_WIDTH};
use std::{
    collections::HashMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
    vec,
};
use structopt::StructOpt;

#[derive(Resource)]
struct Chip8Resource(Chip8);

struct BevyGraphics<'w, 's> {
    commands: Commands<'w, 's>,
}

impl BevyGraphics<'_, '_> {
    /// Draws/turns on a pixel on a specific coordinate.
    ///
    /// If the coordinates is out of the screen area it returns an Error.
    fn draw_pixel(&mut self, x: usize, y: usize, color: Option<Color>) {
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
    }
}

impl Graphics for BevyGraphics<'_, '_> {
    fn clear_pixel(&mut self, x: usize, y: usize) {
        self.draw_pixel(x, y, Some(Color::BLACK))
    }

    fn draw_pixel(&mut self, x: usize, y: usize) {
        self.draw_pixel(x, y, None)
    }
}

fn setup(mut commands: Commands) {
    commands.spawn(Camera2dBundle::default());
}

struct AudioEmulator;

impl Audio for AudioEmulator {
    fn start_beep(&mut self) {
        info!("Starting BEEEEP!")
    }
    
    fn stop_beep(&mut self) {
        info!("Stopping BEEEEP!")
    }
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
    let mut graphics = BevyGraphics { commands };
    let mut audio = AudioEmulator;
    if timer_clock.0.tick(time.delta()).just_finished() {
        ch8.0.tick(&mut graphics, &mut audio);
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
                    ch8.0
                        .handle_key_pressed(keymap.get(&k.unwrap()).cloned().unwrap());
                }
            }
            ButtonState::Released => ch8.0.handle_key_released(),
        }
    }
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
