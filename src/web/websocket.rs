use simple_websockets::{Event, Message, Responder};
use std::collections::HashMap;
use std::net::TcpListener;
use std::sync::mpsc;

use crate::command::{AudioAnalysis, Command, StateUpdate};
use crate::hardware::calibration::Calibration;
use crate::hardware::network;

pub fn start(
    addr: &str,
    cmd_tx: mpsc::Sender<Command>,
    state_rx: mpsc::Receiver<StateUpdate>,
) {
    let listener = TcpListener::bind(addr).unwrap();
    let event_hub = simple_websockets::launch_from_listener(listener)
        .expect(&format!("failed to listen on {}", addr));

    println!("WebSocket server listening on {}", super::display_addr(addr));

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
                StateUpdate::AudioDeviceList(json) => {
                    let msg = format!("hw_audio:devices:{}", json);
                    for responder in clients.values() {
                        responder.send(Message::Text(msg.clone()));
                    }
                }
                StateUpdate::HardwareAudioAnalysis(ref a) => {
                    // Binary format: [0x04, bands[8], kick, snare, hihat,
                    //   vocals, bassline, bpm_hi, bpm_lo, beat_phase,
                    //   note_midi, chord_root, chord_quality] = 20 bytes
                    let mut data = vec![0u8; 20];
                    data[0] = 0x04;
                    data[1..9].copy_from_slice(&a.bands);
                    data[9] = a.kick;
                    data[10] = a.snare;
                    data[11] = a.hihat;
                    data[12] = a.vocals;
                    data[13] = a.bass_line;
                    data[14] = (a.bpm >> 8) as u8;
                    data[15] = (a.bpm & 0xFF) as u8;
                    data[16] = a.beat_phase;
                    data[17] = a.note_midi;
                    data[18] = a.chord_root;
                    data[19] = a.chord_quality;
                    for responder in clients.values() {
                        responder.send(Message::Binary(data.clone()));
                    }
                }
                StateUpdate::HardwareAudioStatus(ref status) => {
                    let msg = format!("hw_audio:status:{}", status);
                    for responder in clients.values() {
                        responder.send(Message::Text(msg.clone()));
                    }
                }
                StateUpdate::BtDeviceList(ref json) => {
                    let msg = format!("bt:devices:{}", json);
                    for responder in clients.values() {
                        responder.send(Message::Text(msg.clone()));
                    }
                }
                StateUpdate::BtResult(ref result) => {
                    let msg = format!("bt:result:{}", result);
                    for responder in clients.values() {
                        responder.send(Message::Text(msg.clone()));
                    }
                }
                StateUpdate::NetStatus(ref json) => {
                    let msg = format!("net:status:{}", json);
                    for responder in clients.values() {
                        responder.send(Message::Text(msg.clone()));
                    }
                }
                StateUpdate::NetResult(ref result) => {
                    let msg = format!("net:result:{}", result);
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
                    // Send a fresh network snapshot
                    let _ = cmd_tx.send(Command::NetList);
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
    } else if text == "hw_audio:list" {
        Some(Command::ListAudioDevices)
    } else if let Some(device_id) = text.strip_prefix("hw_audio:start:") {
        Some(Command::StartHardwareAudio {
            device_id: device_id.trim().to_string(),
        })
    } else if text == "hw_audio:stop" {
        Some(Command::StopHardwareAudio)
    } else if text == "bt:scan" {
        Some(Command::BtScan)
    } else if text == "bt:list" {
        Some(Command::BtList)
    } else if let Some(mac) = text.strip_prefix("bt:pair:") {
        Some(Command::BtPair { mac: mac.trim().to_string() })
    } else if let Some(mac) = text.strip_prefix("bt:connect:") {
        Some(Command::BtConnect { mac: mac.trim().to_string() })
    } else if let Some(mac) = text.strip_prefix("bt:disconnect:") {
        Some(Command::BtDisconnect { mac: mac.trim().to_string() })
    } else if let Some(mac) = text.strip_prefix("bt:remove:") {
        Some(Command::BtRemove { mac: mac.trim().to_string() })
    } else if text == "net:list" {
        Some(Command::NetList)
    } else if text == "net:wifi:scan" {
        Some(Command::NetWifiScan)
    } else if let Some(json) = text.strip_prefix("net:wifi:upsert:") {
        network::known_wifi_from_json(json).map(Command::NetWifiUpsert)
    } else if let Some(ssid) = text.strip_prefix("net:wifi:remove:") {
        Some(Command::NetWifiRemove { ssid: ssid.trim().to_string() })
    } else if let Some(ssid) = text.strip_prefix("net:wifi:connect:") {
        Some(Command::NetWifiConnect { ssid: ssid.trim().to_string() })
    } else if let Some(json) = text.strip_prefix("net:ap:set:") {
        network::ap_config_from_json(json).map(Command::NetApSet)
    } else if let Some(flag) = text.strip_prefix("net:ap:toggle:") {
        Some(Command::NetApToggle { enabled: flag.trim() == "1" })
    } else if let Some(json) = text.strip_prefix("net:eth:set:") {
        network::eth_config_from_json(json).map(Command::NetEthSet)
    } else if let Some(t) = text.strip_prefix("net:stage:confirm:") {
        t.trim().parse().ok().map(|token| Command::NetStageConfirm { token })
    } else if let Some(t) = text.strip_prefix("net:stage:revert:") {
        t.trim().parse().ok().map(|token| Command::NetStageRevert { token })
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
