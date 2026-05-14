mod command;
mod hardware;
mod web;

use command::{AudioAnalysis, Command, StateUpdate};
use hardware::calibration::Calibration;
use hardware::led::LedStrip;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Refresh both Bluetooth device list and audio device list (called after BT operations).
fn refresh_bt_and_audio(tx: &mpsc::Sender<StateUpdate>) {
    let bt_devices = hardware::bluetooth::list_devices();
    let bt_json = hardware::bluetooth::devices_to_json(&bt_devices);
    let _ = tx.send(StateUpdate::BtDeviceList(bt_json));

    let audio_devices = hardware::audio::list_input_devices();
    let audio_json: Vec<String> = audio_devices
        .iter()
        .map(|d| {
            format!(
                r#"{{"id":"{}","name":"{}"}}"#,
                d.id.replace('"', r#"\""#),
                d.name.replace('"', r#"\""#)
            )
        })
        .collect();
    let _ = tx.send(StateUpdate::AudioDeviceList(format!(
        "[{}]",
        audio_json.join(",")
    )));
}

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

    // Keep a clone of cmd_tx for hardware audio capture thread
    let cmd_tx_clone = cmd_tx.clone();

    // Web UI (background thread)
    thread::spawn(|| web::http::start("0.0.0.0:80"));

    // WebSocket server (background thread)
    thread::spawn(move || web::websocket::start("0.0.0.0:8080", cmd_tx, state_rx));

    // Periodic network status broadcaster (so UI sees IP/signal/client changes).
    {
        let tx = state_tx.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(5));
            let _ = tx.send(StateUpdate::NetStatus(hardware::network::status_json()));
        });
    }

    // Hardware audio capture state
    let mut hw_audio_active = false;
    let mut hw_audio_stop: Option<Arc<AtomicBool>> = None;
    let mut hw_audio_thread: Option<JoinHandle<()>> = None;

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
                    // If hardware audio is active, stream analysis back to browser
                    if hw_audio_active {
                        let _ = state_tx.send(StateUpdate::HardwareAudioAnalysis(analysis.clone()));
                    }
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
                Command::ListAudioDevices => {
                    let devices = hardware::audio::list_input_devices();
                    let json_arr: Vec<String> = devices
                        .iter()
                        .map(|d| {
                            format!(
                                r#"{{"id":"{}","name":"{}"}}"#,
                                d.id.replace('"', r#"\""#),
                                d.name.replace('"', r#"\""#)
                            )
                        })
                        .collect();
                    let json = format!("[{}]", json_arr.join(","));
                    let _ = state_tx.send(StateUpdate::AudioDeviceList(json));
                }
                Command::StartHardwareAudio { device_id } => {
                    // Stop any existing hardware capture
                    if let Some(stop_flag) = hw_audio_stop.take() {
                        stop_flag.store(true, Ordering::Relaxed);
                        if let Some(thread) = hw_audio_thread.take() {
                            let _ = thread.join();
                        }
                    }

                    let stop_flag = Arc::new(AtomicBool::new(false));
                    hw_audio_stop = Some(stop_flag.clone());

                    match hardware::audio::start_capture(&device_id, cmd_tx_clone.clone(), stop_flag) {
                        Ok(handle) => {
                            hw_audio_thread = Some(handle);
                            hw_audio_active = true;
                            let _ = state_tx.send(StateUpdate::HardwareAudioStatus("started".to_string()));
                            println!("Hardware audio started on {}", device_id);
                        }
                        Err(e) => {
                            hw_audio_active = false;
                            let _ = state_tx.send(StateUpdate::HardwareAudioStatus(format!("error:{}", e)));
                            println!("Hardware audio failed: {}", e);
                        }
                    }
                }
                Command::StopHardwareAudio => {
                    if let Some(stop_flag) = hw_audio_stop.take() {
                        stop_flag.store(true, Ordering::Relaxed);
                        if let Some(thread) = hw_audio_thread.take() {
                            let _ = thread.join();
                        }
                    }
                    hw_audio_active = false;
                    let _ = state_tx.send(StateUpdate::HardwareAudioStatus("stopped".to_string()));
                    println!("Hardware audio stopped");
                }
                Command::BtScan => {
                    let tx = state_tx.clone();
                    thread::spawn(move || {
                        let _ = tx.send(StateUpdate::BtResult("scan:scanning".to_string()));
                        let devices = hardware::bluetooth::scan(8);
                        let json = hardware::bluetooth::devices_to_json(&devices);
                        let _ = tx.send(StateUpdate::BtDeviceList(json));
                        let _ = tx.send(StateUpdate::BtResult("scan:ok".to_string()));
                    });
                }
                Command::BtList => {
                    let devices = hardware::bluetooth::list_devices();
                    let json = hardware::bluetooth::devices_to_json(&devices);
                    let _ = state_tx.send(StateUpdate::BtDeviceList(json));
                }
                Command::BtPair { mac } => {
                    let tx = state_tx.clone();
                    thread::spawn(move || {
                        match hardware::bluetooth::pair(&mac) {
                            Ok(()) => {
                                let _ = tx.send(StateUpdate::BtResult(format!("pair:ok:{}", mac)));
                            }
                            Err(e) => {
                                let _ = tx.send(StateUpdate::BtResult(format!("pair:error:{}:{}", mac, e)));
                            }
                        }
                        refresh_bt_and_audio(&tx);
                    });
                }
                Command::BtConnect { mac } => {
                    let tx = state_tx.clone();
                    thread::spawn(move || {
                        match hardware::bluetooth::connect(&mac) {
                            Ok(()) => {
                                let _ = tx.send(StateUpdate::BtResult(format!("connect:ok:{}", mac)));
                            }
                            Err(e) => {
                                let _ = tx.send(StateUpdate::BtResult(format!("connect:error:{}:{}", mac, e)));
                            }
                        }
                        // Small delay for PulseAudio/PipeWire to register the new source
                        thread::sleep(Duration::from_secs(2));
                        refresh_bt_and_audio(&tx);
                    });
                }
                Command::BtDisconnect { mac } => {
                    let tx = state_tx.clone();
                    thread::spawn(move || {
                        match hardware::bluetooth::disconnect(&mac) {
                            Ok(()) => {
                                let _ = tx.send(StateUpdate::BtResult(format!("disconnect:ok:{}", mac)));
                            }
                            Err(e) => {
                                let _ = tx.send(StateUpdate::BtResult(format!("disconnect:error:{}:{}", mac, e)));
                            }
                        }
                        thread::sleep(Duration::from_millis(500));
                        refresh_bt_and_audio(&tx);
                    });
                }
                Command::BtRemove { mac } => {
                    let tx = state_tx.clone();
                    thread::spawn(move || {
                        match hardware::bluetooth::remove(&mac) {
                            Ok(()) => {
                                let _ = tx.send(StateUpdate::BtResult(format!("remove:ok:{}", mac)));
                            }
                            Err(e) => {
                                let _ = tx.send(StateUpdate::BtResult(format!("remove:error:{}:{}", mac, e)));
                            }
                        }
                        thread::sleep(Duration::from_millis(500));
                        refresh_bt_and_audio(&tx);
                    });
                }
                Command::NetList => {
                    let _ = state_tx.send(StateUpdate::NetStatus(hardware::network::status_json()));
                }
                Command::NetWifiScan => {
                    let tx = state_tx.clone();
                    thread::spawn(move || {
                        let aps = hardware::network::wifi_scan();
                        let json: Vec<String> = aps.iter().map(|a| format!(
                            r#"{{"ssid":"{}","signal":{},"security":"{}","in_use":{}}}"#,
                            a.ssid.replace('\\', "\\\\").replace('"', "\\\""),
                            a.signal,
                            a.security.replace('"', "\\\""),
                            a.in_use,
                        )).collect();
                        let _ = tx.send(StateUpdate::NetResult(format!("wifi:scan:[{}]", json.join(","))));
                    });
                }
                Command::NetWifiUpsert(w) => {
                    let tx = state_tx.clone();
                    thread::spawn(move || {
                        let ssid = w.ssid.clone();
                        match hardware::network::known_wifi_upsert(w) {
                            Ok(()) => { let _ = tx.send(StateUpdate::NetResult(format!("wifi:upsert:ok:{}", ssid))); }
                            Err(e) => { let _ = tx.send(StateUpdate::NetResult(format!("wifi:upsert:error:{}:{}", ssid, e))); }
                        }
                        let _ = tx.send(StateUpdate::NetStatus(hardware::network::status_json()));
                    });
                }
                Command::NetWifiRemove { ssid } => {
                    let tx = state_tx.clone();
                    thread::spawn(move || {
                        let s = ssid.clone();
                        match hardware::network::known_wifi_remove(&ssid) {
                            Ok(()) => { let _ = tx.send(StateUpdate::NetResult(format!("wifi:remove:ok:{}", s))); }
                            Err(e) => { let _ = tx.send(StateUpdate::NetResult(format!("wifi:remove:error:{}:{}", s, e))); }
                        }
                        let _ = tx.send(StateUpdate::NetStatus(hardware::network::status_json()));
                    });
                }
                Command::NetWifiConnect { ssid } => {
                    // Wi-Fi client switch is risky — stage it.
                    let tx = state_tx.clone();
                    thread::spawn(move || {
                        let profile = format!("lv-client-{}", ssid.chars().map(|c| {
                            if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }
                        }).collect::<String>());
                        let ssid_owned = ssid.clone();
                        let res = hardware::network::stage_change(&profile, move || {
                            hardware::network::wifi_connect_now(&ssid_owned)
                        });
                        match res {
                            Ok(token) => {
                                let _ = tx.send(StateUpdate::NetResult(format!("stage:pending:{}:30", token)));
                            }
                            Err(e) => {
                                let _ = tx.send(StateUpdate::NetResult(format!("wifi:connect:error:{}:{}", ssid, e)));
                            }
                        }
                        let _ = tx.send(StateUpdate::NetStatus(hardware::network::status_json()));
                    });
                }
                Command::NetApSet(cfg) => {
                    let tx = state_tx.clone();
                    thread::spawn(move || {
                        match hardware::network::ap_set(cfg) {
                            Ok(()) => { let _ = tx.send(StateUpdate::NetResult("ap:set:ok".to_string())); }
                            Err(e) => { let _ = tx.send(StateUpdate::NetResult(format!("ap:set:error:{}", e))); }
                        }
                        let _ = tx.send(StateUpdate::NetStatus(hardware::network::status_json()));
                    });
                }
                Command::NetApToggle { enabled } => {
                    let tx = state_tx.clone();
                    thread::spawn(move || {
                        match hardware::network::ap_toggle(enabled) {
                            Ok(()) => { let _ = tx.send(StateUpdate::NetResult(format!("ap:toggle:ok:{}", enabled))); }
                            Err(e) => { let _ = tx.send(StateUpdate::NetResult(format!("ap:toggle:error:{}", e))); }
                        }
                        let _ = tx.send(StateUpdate::NetStatus(hardware::network::status_json()));
                    });
                }
                Command::NetEthSet(cfg) => {
                    // eth0 changes always staged.
                    let tx = state_tx.clone();
                    thread::spawn(move || {
                        let res = hardware::network::stage_change("lv-eth0", move || {
                            hardware::network::eth_set(cfg)
                        });
                        match res {
                            Ok(token) => { let _ = tx.send(StateUpdate::NetResult(format!("stage:pending:{}:30", token))); }
                            Err(e) => { let _ = tx.send(StateUpdate::NetResult(format!("eth:set:error:{}", e))); }
                        }
                        let _ = tx.send(StateUpdate::NetStatus(hardware::network::status_json()));
                    });
                }
                Command::NetStageConfirm { token } => {
                    let tx = state_tx.clone();
                    thread::spawn(move || {
                        let ok = hardware::network::confirm_stage(token);
                        let _ = tx.send(StateUpdate::NetResult(format!("stage:confirm:{}:{}", token, if ok { "ok" } else { "unknown" })));
                        let _ = tx.send(StateUpdate::NetStatus(hardware::network::status_json()));
                    });
                }
                Command::NetStageRevert { token } => {
                    let tx = state_tx.clone();
                    thread::spawn(move || {
                        let ok = hardware::network::revert_stage(token);
                        let _ = tx.send(StateUpdate::NetResult(format!("stage:revert:{}:{}", token, if ok { "ok" } else { "unknown" })));
                        let _ = tx.send(StateUpdate::NetStatus(hardware::network::status_json()));
                    });
                }
            }
        }

        // Run animation frame if active
        if let Some(ref name) = active_animation {
            match name.as_str() {
                "rainbow" => hardware::animation::rainbow_cycle(strip, frame),
                "pulse" => hardware::animation::pulse(strip, frame),
                "chase" => hardware::animation::color_chase(strip, frame),
                "audio_chase" => hardware::animation::audio_chase(strip, frame, &audio_bands),
                "audio_sparkle" => hardware::animation::audio_sparkle(strip, &audio_bands),
                "audio_energy" => hardware::animation::audio_energy(strip, &audio_bands),
                "audio_hybrid" => hardware::animation::audio_hybrid(strip, frame, &audio_bands),
                "audio_synesthesia4" => hardware::animation::audio_synesthesia4(strip, frame, &audio_analysis),
                "audio_harmonic2" => hardware::animation::audio_harmonic2(strip, frame, &audio_analysis),
                "audio_harmonic3" => hardware::animation::audio_harmonic3(strip, frame, &audio_analysis),
                "audio_harmonic4" => hardware::animation::audio_harmonic4(strip, frame, &audio_analysis),
                "audio_harmonic5" => hardware::animation::audio_harmonic5(strip, frame, &audio_analysis),
                "audio_harmonic6" => hardware::animation::audio_harmonic6(strip, frame, &audio_analysis),
                "audio_harmonic7" => hardware::animation::audio_harmonic7(strip, frame, &audio_analysis),
                "audio_harmonic8" => hardware::animation::audio_harmonic8(strip, frame, &audio_analysis),
                "audio_fire" => hardware::animation::audio_fire(strip, frame, &audio_analysis),
                "pacman" => hardware::animation::pacman(strip, frame, &audio_analysis),
                "retro_arcade" => hardware::animation::retro_arcade(strip, frame, &audio_analysis),
                "game_of_life" => hardware::animation::game_of_life(strip, frame, &audio_analysis),
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
