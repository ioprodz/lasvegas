use crate::hardware::led::{hsv_to_rgb, LedStrip};

/// Frequency spectrum — 8 bands mapped across the strip as colored bars.
/// Each band gets LED_COUNT/8 LEDs. Brightness = band amplitude.
/// Color: bass=red, mid=green, treble=blue (HSV hue mapped by band index).
pub fn audio_spectrum(strip: &mut LedStrip, bands: &[u8; 8]) {
    let len = strip.len();
    let leds_per_band = len / 8;

    for (band_idx, &amplitude) in bands.iter().enumerate() {
        let hue = (band_idx as f32) * 360.0 / 8.0;
        let value = amplitude as f32 / 255.0;
        let color = hsv_to_rgb(hue, 1.0, value);

        let start = band_idx * leds_per_band;
        let end = start + leds_per_band;
        for i in start..end {
            strip.set(i, color);
        }
    }
    strip.render();
}
