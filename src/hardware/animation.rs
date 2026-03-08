use super::led::{self, LED_COUNT};

/// Smooth rotating rainbow across all LEDs.
pub fn rainbow_cycle(controller: &mut rs_ws281x::Controller, frame: usize) {
    let leds = controller.leds_mut(0);
    for (i, led) in leds.iter_mut().enumerate() {
        let hue = ((i * 360 / LED_COUNT) + frame) % 360;
        *led = led::hsv_to_rgb(hue as f32, 1.0, 1.0);
    }
    controller.render().unwrap();
}

/// Warm white breathing effect.
pub fn pulse(controller: &mut rs_ws281x::Controller, frame: usize) {
    let phase = (frame as f32 * 0.05).sin();
    let brightness = ((phase + 1.0) / 2.0 * 255.0) as u8;
    led::set_all(controller, [brightness, brightness / 2, brightness / 4, 0]);
}

/// Cyan light segment chasing around the strip.
pub fn color_chase(controller: &mut rs_ws281x::Controller, frame: usize) {
    let leds = controller.leds_mut(0);
    for (i, led) in leds.iter_mut().enumerate() {
        let dist = ((i as isize) - (frame % LED_COUNT) as isize).unsigned_abs() % LED_COUNT;
        if dist < 10 {
            let brightness = (255 - (dist as u16 * 25).min(255)) as u8;
            *led = [0, brightness, brightness, 0];
        } else {
            *led = [0, 0, 0, 0];
        }
    }
    controller.render().unwrap();
}

/// Frequency spectrum — 8 bands mapped across the strip as colored bars.
/// Each band gets 45 LEDs (360/8). Brightness = band amplitude.
/// Color: bass=red, mid=green, treble=blue (HSV hue mapped by band index).
pub fn audio_spectrum(controller: &mut rs_ws281x::Controller, bands: &[u8; 8]) {
    let leds_per_band = LED_COUNT / 8;
    let leds = controller.leds_mut(0);

    for (band_idx, &amplitude) in bands.iter().enumerate() {
        let hue = (band_idx as f32) * 360.0 / 8.0;
        let value = amplitude as f32 / 255.0;
        let color = led::hsv_to_rgb(hue, 1.0, value);

        let start = band_idx * leds_per_band;
        let end = start + leds_per_band;
        for led in leds[start..end].iter_mut() {
            *led = color;
        }
    }
    controller.render().unwrap();
}

/// Whole strip pulses with bass energy, color shifts with treble.
pub fn audio_pulse(controller: &mut rs_ws281x::Controller, bands: &[u8; 8]) {
    // Bass energy: average of first two bands
    let bass = ((bands[0] as u16 + bands[1] as u16) / 2) as f32 / 255.0;
    // Treble energy: average of last two bands
    let treble = ((bands[6] as u16 + bands[7] as u16) / 2) as f32 / 255.0;

    // Hue shifts with treble (0-360), brightness from bass
    let hue = treble * 360.0;
    let color = led::hsv_to_rgb(hue, 1.0, bass);
    led::set_all(controller, color);
}

/// Chase effect — speed driven by bass, color by dominant frequency.
pub fn audio_chase(controller: &mut rs_ws281x::Controller, frame: usize, bands: &[u8; 8]) {
    let bass = ((bands[0] as u16 + bands[1] as u16) / 2) as usize;
    // Speed: 1-8 LEDs per frame based on bass
    let speed = 1 + bass / 32;
    let pos = (frame * speed) % LED_COUNT;

    // Find dominant band for color
    let mut max_band = 0usize;
    let mut max_val = 0u8;
    for (i, &v) in bands.iter().enumerate() {
        if v > max_val {
            max_val = v;
            max_band = i;
        }
    }
    let hue = (max_band as f32) * 360.0 / 8.0;
    let tail_color = led::hsv_to_rgb(hue, 1.0, 1.0);

    let leds = controller.leds_mut(0);
    for (i, led) in leds.iter_mut().enumerate() {
        let dist = ((i as isize) - pos as isize).unsigned_abs() % LED_COUNT;
        if dist < 20 {
            let fade = 1.0 - (dist as f32 / 20.0);
            *led = [
                (tail_color[0] as f32 * fade) as u8,
                (tail_color[1] as f32 * fade) as u8,
                (tail_color[2] as f32 * fade) as u8,
                0,
            ];
        } else {
            *led = [0, 0, 0, 0];
        }
    }
    controller.render().unwrap();
}
