use super::led::{self, LED_COUNT};

/// One frame of an animation. Returns the updated frame counter.
/// Each function writes directly to the controller and renders.
pub fn rainbow_cycle(controller: &mut rs_ws281x::Controller, frame: usize) {
    let leds = controller.leds_mut(0);
    for (i, led) in leds.iter_mut().enumerate() {
        let hue = ((i * 360 / LED_COUNT) + frame) % 360;
        *led = led::hsv_to_rgb(hue as f32, 1.0, 1.0);
    }
    controller.render().unwrap();
}

pub fn pulse(controller: &mut rs_ws281x::Controller, frame: usize) {
    // Sine-wave breathing in warm white
    let phase = (frame as f32 * 0.05).sin();
    let brightness = ((phase + 1.0) / 2.0 * 255.0) as u8;
    led::set_all(controller, [brightness, brightness / 2, brightness / 4, 0]);
}

pub fn color_chase(controller: &mut rs_ws281x::Controller, frame: usize) {
    let leds = controller.leds_mut(0);
    for (i, led) in leds.iter_mut().enumerate() {
        // 10-LED wide chase segment
        let dist = ((i as isize) - (frame % LED_COUNT) as isize).unsigned_abs() % LED_COUNT;
        if dist < 10 {
            let brightness = (255 - (dist as u16 * 25).min(255)) as u8;
            *led = [0, brightness, brightness, 0]; // cyan chase
        } else {
            *led = [0, 0, 0, 0];
        }
    }
    controller.render().unwrap();
}
