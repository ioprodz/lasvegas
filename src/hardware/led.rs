use rs_ws281x::{ChannelBuilder, ControllerBuilder, StripType};
use std::thread::sleep;
use std::time::Duration;

pub const LED_COUNT: usize = 60 * 6;
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
                .count(LED_COUNT as i32)
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

/// Read current LED state as flat [r, g, b, r, g, b, ...] vec.
pub fn read_state(controller: &mut rs_ws281x::Controller) -> Vec<u8> {
    let leds = controller.leds_mut(0);
    let mut state = Vec::with_capacity(leds.len() * 3);
    for led in leds.iter() {
        state.push(led[0]); // r
        state.push(led[1]); // g
        state.push(led[2]); // b
    }
    state
}

pub fn startup_animation(controller: &mut rs_ws281x::Controller) {
    println!("Running startup animation...");

    // 1. Fast white wipe — 6 LEDs at a time (~300ms)
    for i in (0..LED_COUNT).step_by(6) {
        let leds = controller.leds_mut(0);
        for j in i..(i + 6).min(LED_COUNT) {
            leds[j] = [255, 255, 255, 0];
        }
        controller.render().unwrap();
        sleep(Duration::from_millis(5));
    }

    // 2. R/G/B flash — quick full-color test (~600ms)
    for &color in &[[255, 0, 0, 0], [0, 255, 0, 0], [0, 0, 255, 0]] {
        set_all(controller, color);
        sleep(Duration::from_millis(200));
    }

    // 3. Quick rainbow sweep (~1200ms)
    for offset in (0..360).step_by(3) {
        let leds = controller.leds_mut(0);
        for (i, led) in leds.iter_mut().enumerate() {
            let hue = ((i + offset) % 360) as f32;
            *led = hsv_to_rgb(hue, 1.0, 1.0);
        }
        controller.render().unwrap();
        sleep(Duration::from_millis(10));
    }

    // 4. Fast fade out (~500ms)
    for brightness in (0u8..=255).rev().step_by(15) {
        set_all(controller, [brightness, brightness, brightness, 0]);
        sleep(Duration::from_millis(20));
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
