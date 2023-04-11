use rs_ws281x::ControllerBuilder;
use rs_ws281x::ChannelBuilder;
use rs_ws281x::StripType;
use simple_websockets::{Event, Responder, Message};
use std::collections::HashMap;
use std::net::{TcpListener};

fn main() {
    // Construct a single channel controller. Note that the
    // Controller is initialized by default and is cleaned up on drop

    let controller = &mut create_led_controller();
    render_color([255, 0, 50, 0], controller);
    
    let listener = TcpListener::bind("0.0.0.0:8080").unwrap();
    let event_hub = simple_websockets::launch_from_listener(listener)
        .expect("failed to listen on port 8080");
    // map between client ids and the client's `Responder`:
    let mut clients: HashMap<u64, Responder> = HashMap::new();

    loop {
        match event_hub.poll_event() {
            Event::Connect(client_id, responder) => {
                println!("A client connected with id #{}", client_id);
                // add their Responder to our `clients` map:
                clients.insert(client_id, responder.clone());

                responder.send(Message::Text("test".to_string()));
            }
            Event::Disconnect(client_id) => {
                println!("Client #{} disconnected.", client_id);
                // remove the disconnected client from the clients map:
                clients.remove(&client_id);
            },
            Event::Message(_client_id, message) => {
                match message {
                    Message::Text(text) => {
                        println!("Received text message: {}", text);
                    }
                    Message::Binary(data) => {
                        render_color([data[1], data[2], data[3], 0], controller);
                    }
                }
            },
        }
    }

}

fn render_color(color: [u8; 4], controller: &mut rs_ws281x::Controller) -> () {

    let leds = controller.leds_mut(0);
    for led in leds {
        *led = color;
    }
    controller.render().unwrap();
}

fn create_led_controller() -> rs_ws281x::Controller {
    return ControllerBuilder::new()
        .freq(800_000)
        .dma(10)
        .channel(
            0, // Channel Index
            ChannelBuilder::new()
            .pin(18) // GPIO 10 = SPI0 MOSI
            .count(60*6) // Number of LEDs
            .strip_type(StripType::Ws2811Gbr)
            .brightness(255) // default: 255
            .build(),
            )
        .build()
        .unwrap();
}

