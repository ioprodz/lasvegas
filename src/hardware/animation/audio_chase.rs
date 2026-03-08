use crate::hardware::led::{hsv_to_rgb, LedStrip};

/// Chase effect — speed driven by bass, color by dominant frequency.
pub fn audio_chase(strip: &mut LedStrip, frame: usize, bands: &[u8; 8]) {
    let len = strip.len();
    let bass = ((bands[0] as u16 + bands[1] as u16) / 2) as usize;
    // Speed: 1-8 LEDs per frame based on bass
    let speed = 1 + bass / 32;
    let pos = (frame * speed) % len;

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
    let tail_color = hsv_to_rgb(hue, 1.0, 1.0);

    for i in 0..len {
        let dist = ((i as isize) - pos as isize).unsigned_abs() % len;
        if dist < 20 {
            let fade = 1.0 - (dist as f32 / 20.0);
            strip.set(i, [
                (tail_color[0] as f32 * fade) as u8,
                (tail_color[1] as f32 * fade) as u8,
                (tail_color[2] as f32 * fade) as u8,
                0,
            ]);
        } else {
            strip.set(i, [0, 0, 0, 0]);
        }
    }
}
