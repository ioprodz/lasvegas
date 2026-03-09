mod command;
mod hardware;
mod web;

use command::{AudioAnalysis, Command, StateUpdate};
use hardware::calibration::Calibration;
use hardware::led::LedStrip;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

fn main() {
    // Hardware
    let strip = &mut LedStrip::new();
    strip.startup_animation();

    // Load saved calibration
    let mut calibration = Calibration::load();
    println!("Loaded calibration: {:?}", calibration);

    // Channels: commands from WebSocket, state updates to WebSocket
    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
    let (state_tx, state_rx) = mpsc::channel::<StateUpdate>();

    // Web UI (background thread)
    thread::spawn(|| web::http::start("0.0.0.0:3000"));

    // WebSocket server (background thread)
    thread::spawn(move || web::websocket::start("0.0.0.0:8080", cmd_tx, state_rx));

    // Main loop: owns the LED strip, runs animations
    let mut active_animation: Option<String> = None;
    let mut frame: usize = 0;
    let mut audio_bands: [u8; 8] = [0; 8];
    let mut audio_analysis = AudioAnalysis::default();
    let frame_duration = Duration::from_millis(16); // ~60fps

    loop {
        let frame_start = Instant::now();

        // Drain incoming commands
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                Command::SetColor { r, g, b } => {
                    active_animation = None;
                    strip.set_all([r, g, b, 0]);
                    strip.render_calibrated(&calibration);
                    let state = strip.read_state();
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
                    strip.set_all([0, 0, 0, 0]);
                    strip.render_calibrated(&calibration);
                    let state = strip.read_state();
                    let _ = state_tx.send(StateUpdate::LedState(state));
                }
                Command::AudioData { bands } => {
                    for (i, &b) in bands.iter().enumerate().take(8) {
                        audio_bands[i] = b;
                    }
                }
                Command::ExtendedAudioData(analysis) => {
                    audio_bands = analysis.bands;
                    audio_analysis = analysis;
                }
                Command::SetCalibration(cal) => {
                    calibration = cal;
                    // Re-render current state with new calibration
                    strip.render_calibrated(&calibration);
                    let state = strip.read_state();
                    let _ = state_tx.send(StateUpdate::LedState(state));
                }
                Command::SaveCalibration => {
                    match calibration.save() {
                        Ok(_) => println!("Calibration saved"),
                        Err(e) => println!("Failed to save calibration: {}", e),
                    }
                }
                Command::GetCalibration => {
                    let json = calibration.to_json();
                    let _ = state_tx.send(StateUpdate::CalibrationData(json));
                }
            }
        }

        // Run animation frame if active
        if let Some(ref name) = active_animation {
            match name.as_str() {
                "rainbow" => hardware::animation::rainbow_cycle(strip, frame),
                "pulse" => hardware::animation::pulse(strip, frame),
                "chase" => hardware::animation::color_chase(strip, frame),
                "audio_spectrum" => hardware::animation::audio_spectrum(strip, &audio_bands),
                "audio_pulse" => hardware::animation::audio_pulse(strip, &audio_bands),
                "audio_chase" => hardware::animation::audio_chase(strip, frame, &audio_bands),
                "audio_ripple" => hardware::animation::audio_ripple(strip, &audio_bands),
                "audio_waterfall" => hardware::animation::audio_waterfall(strip, &audio_bands),
                "audio_sparkle" => hardware::animation::audio_sparkle(strip, &audio_bands),
                "audio_energy" => hardware::animation::audio_energy(strip, &audio_bands),
                "audio_pastel" => hardware::animation::audio_pastel(strip, &audio_bands),
                "audio_hybrid" => hardware::animation::audio_hybrid(strip, frame, &audio_bands),
                "audio_synesthesia" => hardware::animation::audio_synesthesia(strip, frame, &audio_analysis),
                "audio_synesthesia2" => hardware::animation::audio_synesthesia2(strip, frame, &audio_analysis),
                "audio_synesthesia3" => hardware::animation::audio_synesthesia3(strip, frame, &audio_analysis),
                "audio_synesthesia4" => hardware::animation::audio_synesthesia4(strip, frame, &audio_analysis),
                "audio_harmonic" => hardware::animation::audio_harmonic(strip, frame, &audio_analysis),
                _ => {}
            }
            frame = frame.wrapping_add(1);

            strip.render_calibrated(&calibration);

            // Send state to UI every 3rd frame (~20fps updates to browser)
            if frame % 3 == 0 {
                let state = strip.read_state();
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
