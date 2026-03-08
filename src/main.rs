mod command;
mod hardware;
mod web;

use command::{Command, StateUpdate};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

fn main() {
    // Hardware
    let controller = &mut hardware::led::create_controller();
    hardware::led::startup_animation(controller);

    // Channels: commands from WebSocket, state updates to WebSocket
    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
    let (state_tx, state_rx) = mpsc::channel::<StateUpdate>();

    // Web UI (background thread)
    thread::spawn(|| web::http::start("0.0.0.0:3000"));

    // WebSocket server (background thread)
    thread::spawn(move || web::websocket::start("0.0.0.0:8080", cmd_tx, state_rx));

    // Main loop: owns the LED controller, runs animations
    let mut active_animation: Option<String> = None;
    let mut frame: usize = 0;
    let mut audio_bands: [u8; 8] = [0; 8];
    let frame_duration = Duration::from_millis(16); // ~60fps

    loop {
        let frame_start = Instant::now();

        // Drain incoming commands
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                Command::SetColor { r, g, b } => {
                    active_animation = None;
                    hardware::led::set_all(controller, [r, g, b, 0]);
                    let state = hardware::led::read_state(controller);
                    let _ = state_tx.send(StateUpdate::LedState(state));
                }
                Command::StartAnimation { name } => {
                    println!("Starting animation: {}", name);
                    active_animation = Some(name);
                    frame = 0;
                }
                Command::StopAnimation => {
                    println!("Stopping animation");
                    active_animation = None;
                    audio_bands = [0; 8];
                    hardware::led::set_all(controller, [0, 0, 0, 0]);
                    let state = hardware::led::read_state(controller);
                    let _ = state_tx.send(StateUpdate::LedState(state));
                }
                Command::AudioData { bands } => {
                    for (i, &b) in bands.iter().enumerate().take(8) {
                        audio_bands[i] = b;
                    }
                }
            }
        }

        // Run animation frame if active
        if let Some(ref name) = active_animation {
            match name.as_str() {
                "rainbow" => hardware::animation::rainbow_cycle(controller, frame),
                "pulse" => hardware::animation::pulse(controller, frame),
                "chase" => hardware::animation::color_chase(controller, frame),
                "audio_spectrum" => hardware::animation::audio_spectrum(controller, &audio_bands),
                "audio_pulse" => hardware::animation::audio_pulse(controller, &audio_bands),
                "audio_chase" => hardware::animation::audio_chase(controller, frame, &audio_bands),
                _ => {}
            }
            frame = frame.wrapping_add(1);

            // Send state to UI every 3rd frame (~20fps updates to browser)
            if frame % 3 == 0 {
                let state = hardware::led::read_state(controller);
                let _ = state_tx.send(StateUpdate::LedState(state));
            }
        }

        // Sleep remainder of frame
        let elapsed = frame_start.elapsed();
        if elapsed < frame_duration {
            thread::sleep(frame_duration - elapsed);
        }
    }
}
