use crate::hardware::led::LedStrip;

/// Cyan light segment chasing around the strip.
pub fn color_chase(strip: &mut LedStrip, frame: usize) {
    let len = strip.len();
    for i in 0..len {
        let dist = ((i as isize) - (frame % len) as isize).unsigned_abs() % len;
        if dist < 10 {
            let brightness = (255 - (dist as u16 * 25).min(255)) as u8;
            strip.set(i, [0, brightness, brightness, 0]);
        } else {
            strip.set(i, [0, 0, 0, 0]);
        }
    }
    strip.render();
}
