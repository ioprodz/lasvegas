use simple_websockets::{Event, Message, Responder};
use std::collections::HashMap;
use std::net::TcpListener;

use crate::hardware::led;

pub fn start(addr: &str, controller: &mut rs_ws281x::Controller) {
    let listener = TcpListener::bind(addr).unwrap();
    let event_hub = simple_websockets::launch_from_listener(listener)
        .expect(&format!("failed to listen on {}", addr));

    println!("WebSocket server listening on {}", addr);

    let mut clients: HashMap<u64, Responder> = HashMap::new();

    loop {
        match event_hub.poll_event() {
            Event::Connect(client_id, responder) => {
                println!("Client #{} connected", client_id);
                clients.insert(client_id, responder.clone());
                responder.send(Message::Text("connected".to_string()));
            }
            Event::Disconnect(client_id) => {
                println!("Client #{} disconnected", client_id);
                clients.remove(&client_id);
            }
            Event::Message(_client_id, message) => match message {
                Message::Text(text) => {
                    println!("Received text message: {}", text);
                }
                Message::Binary(data) => {
                    if data.len() >= 4 {
                        led::set_all(controller, [data[1], data[2], data[3], 0]);
                    }
                }
            },
        }
    }
}
