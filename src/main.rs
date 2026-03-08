use rs_ws281x::ControllerBuilder;
use rs_ws281x::ChannelBuilder;
use rs_ws281x::StripType;
use simple_websockets::{Event, Responder, Message};
use std::collections::HashMap;
use std::io::Write;
use std::net::TcpListener;
use std::thread::sleep;
use std::time::Duration;

const INDEX_HTML: &str = include_str!("../static/index.html");

fn main() {
    // Spawn HTTP server for web UI on port 3000
    std::thread::spawn(|| {
        let listener = TcpListener::bind("0.0.0.0:3000").unwrap();
        println!("Web UI available at http://0.0.0.0:3000");
        for stream in listener.incoming() {
            if let Ok(mut stream) = stream {
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
                    INDEX_HTML.len(),
                    INDEX_HTML
                );
                let _ = stream.write_all(response.as_bytes());
            }
        }
    });

    // Construct a single channel controller. Note that the
    // Controller is initialized by default and is cleaned up on drop

    let controller = &mut create_led_controller();
    startup_animation(controller);

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

fn startup_animation(controller: &mut rs_ws281x::Controller) {
    println!("Running startup animation...");
    let delay = Duration::from_millis(30);
    let led_count = 360;

    // 1. Sequential wipe in white — test each LED one by one
    for i in 0..led_count {
        let leds = controller.leds_mut(0);
        leds[i] = [255, 255, 255, 0];
        controller.render().unwrap();
        sleep(Duration::from_millis(5));
    }
    sleep(Duration::from_millis(200));

    // 2. Full red at increasing brightness levels
    for brightness in (0u8..=255).step_by(15) {
        let leds = controller.leds_mut(0);
        for led in leds.iter_mut() {
            *led = [brightness, 0, 0, 0];
        }
        controller.render().unwrap();
        sleep(delay);
    }
    sleep(Duration::from_millis(200));

    // 3. Full green at increasing brightness
    for brightness in (0u8..=255).step_by(15) {
        let leds = controller.leds_mut(0);
        for led in leds.iter_mut() {
            *led = [0, brightness, 0, 0];
        }
        controller.render().unwrap();
        sleep(delay);
    }
    sleep(Duration::from_millis(200));

    // 4. Full blue at increasing brightness
    for brightness in (0u8..=255).step_by(15) {
        let leds = controller.leds_mut(0);
        for led in leds.iter_mut() {
            *led = [0, 0, brightness, 0];
        }
        controller.render().unwrap();
        sleep(delay);
    }
    sleep(Duration::from_millis(200));

    // 5. Rainbow sweep — each LED gets a different hue, scrolls across
    for offset in 0..360 {
        let leds = controller.leds_mut(0);
        for (i, led) in leds.iter_mut().enumerate() {
            let hue = ((i + offset) % 360) as f32;
            *led = hsv_to_rgb(hue, 1.0, 1.0);
        }
        controller.render().unwrap();
        sleep(Duration::from_millis(10));
    }

    // 6. Fade out to black
    for brightness in (0u8..=255).rev().step_by(5) {
        let leds = controller.leds_mut(0);
        for led in leds.iter_mut() {
            *led = [brightness, brightness, brightness, 0];
        }
        controller.render().unwrap();
        sleep(Duration::from_millis(15));
    }

    // All off
    render_color([0, 0, 0, 0], controller);
    println!("Startup animation complete.");
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 4] {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as u16 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
        0,
    ]
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

