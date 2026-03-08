use rs_ws281x::{ChannelBuilder, ControllerBuilder, StripType};
use std::thread::sleep;
use std::time::Duration;

const LED_COUNT: i32 = 60 * 6;
const GPIO_PIN: i32 = 18;
const FREQUENCY: u32 = 800_000;
const DMA_CHANNEL: i32 = 10;

pub fn create_controller() -> rs_ws281x::Controller {
    ControllerBuilder::new()
        .freq(FREQUENCY)
        .dma(DMA_CHANNEL)
        .channel(
            0,
            ChannelBuilder::new()
                .pin(GPIO_PIN)
                .count(LED_COUNT)
                .strip_type(StripType::Ws2811Gbr)
                .brightness(255)
                .build(),
        )
        .build()
        .unwrap()
}

pub fn set_all(controller: &mut rs_ws281x::Controller, color: [u8; 4]) {
    let leds = controller.leds_mut(0);
    for led in leds {
        *led = color;
    }
    controller.render().unwrap();
}

pub fn startup_animation(controller: &mut rs_ws281x::Controller) {
    println!("Running startup animation...");
    let delay = Duration::from_millis(30);
    let led_count = LED_COUNT as usize;

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
        set_all(controller, [brightness, 0, 0, 0]);
        sleep(delay);
    }
    sleep(Duration::from_millis(200));

    // 3. Full green at increasing brightness
    for brightness in (0u8..=255).step_by(15) {
        set_all(controller, [0, brightness, 0, 0]);
        sleep(delay);
    }
    sleep(Duration::from_millis(200));

    // 4. Full blue at increasing brightness
    for brightness in (0u8..=255).step_by(15) {
        set_all(controller, [0, 0, brightness, 0]);
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
        set_all(controller, [brightness, brightness, brightness, 0]);
        sleep(Duration::from_millis(15));
    }

    set_all(controller, [0, 0, 0, 0]);
    println!("Startup animation complete.");
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 4] {
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
