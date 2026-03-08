use crate::hardware::led::LedStrip;

/// Warm white breathing effect.
pub fn pulse(strip: &mut LedStrip, frame: usize) {
    let phase = (frame as f32 * 0.05).sin();
    let brightness = ((phase + 1.0) / 2.0 * 255.0) as u8;
    strip.set_all([brightness, brightness / 2, brightness / 4, 0]);
    strip.render();
}
