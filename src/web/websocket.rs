use simple_websockets::{Event, Message, Responder};
use std::collections::HashMap;
use std::net::TcpListener;
use std::sync::mpsc;

use crate::command::{Command, StateUpdate};

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
            }
        }

        // Process WebSocket events (non-blocking check then yield)
        match event_hub.next_event() {
            Some(event) => match event {
                Event::Connect(client_id, responder) => {
                    println!("Client #{} connected", client_id);
                    clients.insert(client_id, responder);
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
        _ => None,
    }
}
