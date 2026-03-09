use simple_websockets::{Event, Message, Responder};
use std::collections::HashMap;
use std::net::TcpListener;
use std::sync::mpsc;

use crate::command::{AudioAnalysis, Command, StateUpdate};
use crate::hardware::calibration::Calibration;

pub fn start(
    addr: &str,
    cmd_tx: mpsc::Sender<Command>,
    state_rx: mpsc::Receiver<StateUpdate>,
) {
    let listener = TcpListener::bind(addr).unwrap();
    let event_hub = simple_websockets::launch_from_listener(listener)
        .expect(&format!("failed to listen on {}", addr));

    println!("WebSocket server listening on {}", addr);

    let mut clients: HashMap<u64, Responder> = HashMap::new();

    loop {
        // Drain state updates and broadcast to clients
        while let Ok(update) = state_rx.try_recv() {
            match update {
                StateUpdate::LedState(state) => {
                    for responder in clients.values() {
                        responder.send(Message::Binary(state.clone()));
                    }
                }
                StateUpdate::CalibrationData(json) => {
                    let msg = format!("calibration:{}", json);
                    for responder in clients.values() {
                        responder.send(Message::Text(msg.clone()));
                    }
                }
            }
        }

        // Process WebSocket events (non-blocking check then yield)
        match event_hub.next_event() {
            Some(event) => match event {
                Event::Connect(client_id, responder) => {
                    println!("Client #{} connected", client_id);
                    clients.insert(client_id, responder);
                    // Send current calibration to new client
                    let _ = cmd_tx.send(Command::GetCalibration);
                }
                Event::Disconnect(client_id) => {
                    println!("Client #{} disconnected", client_id);
                    clients.remove(&client_id);
                }
                Event::Message(_client_id, message) => match message {
                    Message::Text(text) => {
                        println!("Received: {}", text);
                        if let Some(cmd) = parse_command(&text) {
                            let _ = cmd_tx.send(cmd);
                        }
                    }
                    Message::Binary(data) => {
                        if let Some(cmd) = parse_binary(&data) {
                            let _ = cmd_tx.send(cmd);
                        }
                    }
                },
            },
            None => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }
}

fn parse_command(text: &str) -> Option<Command> {
    let text = text.trim();
    if text == "stop" {
        Some(Command::StopAnimation)
    } else if let Some(name) = text.strip_prefix("animate:") {
        Some(Command::StartAnimation {
            name: name.trim().to_string(),
        })
    } else if let Some(params) = text.strip_prefix("calibrate:") {
        Calibration::from_command(params).map(Command::SetCalibration)
    } else if text == "save_calibration" {
        Some(Command::SaveCalibration)
    } else if text == "get_calibration" {
        Some(Command::GetCalibration)
    } else {
        None
    }
}

fn parse_binary(data: &[u8]) -> Option<Command> {
    if data.is_empty() {
        return None;
    }
    match data[0] {
        0x01 if data.len() >= 4 => Some(Command::SetColor {
            r: data[1],
            g: data[2],
            b: data[3],
        }),
        0x02 if data.len() >= 2 => Some(Command::AudioData {
            bands: data[1..].to_vec(),
        }),
        // Extended audio analysis: [0x03, bands[8], kick, snare, hihat,
        //   vocals, bassline, bpm_hi, bpm_lo, beat_phase, note_midi,
        //   chord_root, chord_quality]  = 21 bytes
        0x03 if data.len() >= 20 => {
            let mut bands = [0u8; 8];
            bands.copy_from_slice(&data[1..9]);
            Some(Command::ExtendedAudioData(AudioAnalysis {
                bands,
                kick: data[9],
                snare: data[10],
                hihat: data[11],
                vocals: data[12],
                bass_line: data[13],
                bpm: ((data[14] as u16) << 8) | (data[15] as u16),
                beat_phase: data[16],
                note_midi: data[17],
                chord_root: data[18],
                chord_quality: data[19],
            }))
        }
        _ => None,
    }
}
