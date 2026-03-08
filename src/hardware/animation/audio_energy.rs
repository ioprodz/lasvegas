use crate::hardware::led::{hsv_to_rgb, LedStrip};
use std::cell::RefCell;

const COLS: usize = 60;
const ROWS: usize = 6;

thread_local! {
    static SMOOTHED: RefCell<[f32; 6]> = RefCell::new([0.0; 6]);
}

/// Energy bars — each of the 6 rows represents a frequency range.
/// Bars grow outward from the center of each row, width proportional
/// to that band's energy. Smoothed for fluid motion.
pub fn audio_energy(strip: &mut LedStrip, bands: &[u8; 8]) {
    SMOOTHED.with(|smoothed| {
        let sm = &mut *smoothed.borrow_mut();

        let row_vals: [f32; 6] = [
            (bands[0] as f32 + bands[1] as f32) / (2.0 * 255.0),
            bands[2] as f32 / 255.0,
            bands[3] as f32 / 255.0,
            bands[4] as f32 / 255.0,
            (bands[5] as f32 + bands[6] as f32) / (2.0 * 255.0),
            bands[7] as f32 / 255.0,
        ];

        // Smooth: fast attack, slower decay
        for i in 0..6 {
            if row_vals[i] > sm[i] {
                sm[i] += (row_vals[i] - sm[i]) * 0.4;
            } else {
                sm[i] += (row_vals[i] - sm[i]) * 0.12;
            }
        }

        let row_hues: [f32; 6] = [0.0, 25.0, 55.0, 150.0, 210.0, 280.0];
        let half = COLS / 2;

        for row in 0..ROWS {
            let energy = sm[row];
            let bar_half_width = (energy * half as f32) as usize;
            let hue = row_hues[row];

            for col in 0..COLS {
                let led_idx = row * COLS + col;
                let dist_from_center = if col >= half {
                    col - half
                } else {
                    half - 1 - col
                };

                if dist_from_center < bar_half_width {
                    let t = dist_from_center as f32 / bar_half_width.max(1) as f32;
                    let val = 1.0 - t * 0.5;
                    let sat = 0.8 + t * 0.2;
                    strip.set(led_idx, hsv_to_rgb(hue, sat, val));
                } else {
                    let tail_dist = dist_from_center - bar_half_width;
                    if tail_dist < 3 {
                        let glow = 0.08 * (1.0 - tail_dist as f32 / 3.0);
                        strip.set(led_idx, hsv_to_rgb(hue, 0.5, glow));
                    } else {
                        strip.set(led_idx, [0, 0, 0, 0]);
                    }
                }
            }
        }
    });
}
