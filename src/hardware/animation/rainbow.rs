use crate::hardware::led::{hsv_to_rgb, LedStrip};

/// Smooth rotating rainbow across all LEDs.
pub fn rainbow_cycle(strip: &mut LedStrip, frame: usize) {
    let len = strip.len();
    for i in 0..len {
        let hue = ((i * 360 / len) + frame) % 360;
        strip.set(i, hsv_to_rgb(hue as f32, 1.0, 1.0));
    }
}
