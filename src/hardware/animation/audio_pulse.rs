use crate::hardware::led::{hsv_to_rgb, LedStrip};

/// Whole strip pulses with bass energy, color shifts with treble.
pub fn audio_pulse(strip: &mut LedStrip, bands: &[u8; 8]) {
    // Bass energy: average of first two bands
    let bass = ((bands[0] as u16 + bands[1] as u16) / 2) as f32 / 255.0;
    // Treble energy: average of last two bands
    let treble = ((bands[6] as u16 + bands[7] as u16) / 2) as f32 / 255.0;

    // Hue shifts with treble (0-360), brightness from bass
    let hue = treble * 360.0;
    let color = hsv_to_rgb(hue, 1.0, bass);
    strip.set_all(color);
    strip.render();
}
