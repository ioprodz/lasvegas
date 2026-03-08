use crate::hardware::led::{hsv_to_rgb, LedStrip, LED_COUNT};
use std::cell::RefCell;

const MAX_RIPPLES: usize = 12;
const RIPPLE_SPEED: f32 = 3.0;
const RIPPLE_LIFETIME: f32 = 80.0;
const BEAT_THRESHOLD: u8 = 20;

struct Ripple {
    age: f32,
    center: usize,
    hue: f32,
    intensity: f32,
}

struct RippleState {
    ripples: Vec<Ripple>,
    prev_bass: u8,
    hue_offset: f32,
}

thread_local! {
    static STATE: RefCell<RippleState> = RefCell::new(RippleState {
        ripples: Vec::new(),
        prev_bass: 0,
        hue_offset: 0.0,
    });
}

/// Beat-reactive ripples — detects bass transients and spawns expanding
/// waves from the center. Multiple ripples layer and blend additively.
pub fn audio_ripple(strip: &mut LedStrip, bands: &[u8; 8]) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();

        let bass = ((bands[0] as u16 + bands[1] as u16) / 2) as u8;
        let mid = ((bands[3] as u16 + bands[4] as u16) / 2) as f32 / 255.0;

        // Beat detection: sharp rise in bass
        let bass_delta = bass.saturating_sub(s.prev_bass);
        s.prev_bass = bass;

        if bass_delta > BEAT_THRESHOLD && s.ripples.len() < MAX_RIPPLES {
            s.hue_offset += 47.0;
            let center = (LED_COUNT / 2) as f32 + (mid * 60.0 - 30.0);
            s.ripples.push(Ripple {
                age: 0.0,
                center: center as usize % LED_COUNT,
                hue: s.hue_offset % 360.0,
                intensity: (bass as f32 / 255.0).max(0.5),
            });
        }

        let len = strip.len();
        let mut buffer = vec![[0u16; 3]; len];

        for ripple in s.ripples.iter() {
            let radius = ripple.age * RIPPLE_SPEED;
            let fade = 1.0 - (ripple.age / RIPPLE_LIFETIME);
            let fade = fade * fade * ripple.intensity;
            if fade <= 0.0 {
                continue;
            }

            let color = hsv_to_rgb(ripple.hue, 0.9, 1.0);
            let ring_width = 8.0 + ripple.age * 0.3;

            for i in 0..len {
                let dist = ((i as isize) - (ripple.center as isize)).unsigned_abs() as f32;
                let dist = dist.min((len as f32) - dist);
                let ring_dist = (dist - radius).abs();

                if ring_dist < ring_width {
                    let brightness = (1.0 - ring_dist / ring_width) * fade;
                    buffer[i][0] = (buffer[i][0] + (color[0] as f32 * brightness) as u16).min(255);
                    buffer[i][1] = (buffer[i][1] + (color[1] as f32 * brightness) as u16).min(255);
                    buffer[i][2] = (buffer[i][2] + (color[2] as f32 * brightness) as u16).min(255);
                }
            }
        }

        for i in 0..len {
            strip.set(i, [buffer[i][0] as u8, buffer[i][1] as u8, buffer[i][2] as u8, 0]);
        }

        s.ripples.iter_mut().for_each(|r| r.age += 1.0);
        s.ripples.retain(|r| r.age < RIPPLE_LIFETIME);
    });
}
