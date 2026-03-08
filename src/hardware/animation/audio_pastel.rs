use crate::hardware::led::{hsv_to_rgb, LedStrip};
use std::cell::RefCell;

const COLS: usize = 60;
const ROWS: usize = 6;
const SECTIONS: usize = 6;
const COLS_PER_SECTION: usize = COLS / SECTIONS; // 10 columns each

struct PastelState {
    /// Smoothed energy per section (slow, breathy envelope)
    energy: [f32; SECTIONS],
    /// Slowly drifting base hue per section
    hue: [f32; SECTIONS],
    frame: f32,
}

thread_local! {
    static STATE: RefCell<PastelState> = RefCell::new(PastelState {
        energy: [0.0; SECTIONS],
        hue: [330.0, 270.0, 200.0, 160.0, 50.0, 20.0],
        frame: 0.0,
    });
}

/// Pastel sections — 6 vertical blocks spanning all rows, each tied to
/// a frequency range. Blocks fade in and out with soft pastel colors
/// that drift gently over time. Soft edges between sections blend together.
pub fn audio_pastel(strip: &mut LedStrip, bands: &[u8; 8]) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();
        s.frame += 1.0;

        // Map 8 bands → 6 sections
        let raw: [f32; SECTIONS] = [
            (bands[0] as f32 + bands[1] as f32) / (2.0 * 255.0),
            bands[2] as f32 / 255.0,
            bands[3] as f32 / 255.0,
            bands[4] as f32 / 255.0,
            (bands[5] as f32 + bands[6] as f32) / (2.0 * 255.0),
            bands[7] as f32 / 255.0,
        ];

        for i in 0..SECTIONS {
            let target = raw[i];
            if target > s.energy[i] {
                s.energy[i] += (target - s.energy[i]) * 0.15;
            } else {
                s.energy[i] += (target - s.energy[i]) * 0.04;
            }

            let drift_speed = 0.05 + s.energy[i] * 0.15;
            s.hue[i] = (s.hue[i] + drift_speed) % 360.0;
        }

        // Precompute per-section values
        let mut sec_sat = [0.0f32; SECTIONS];
        let mut sec_val = [0.0f32; SECTIONS];
        for i in 0..SECTIONS {
            sec_sat[i] = 0.25 + s.energy[i] * 0.15;
            sec_val[i] = 0.06 + ease_in_out(s.energy[i]) * 0.94;
        }

        for col in 0..COLS {
            // Determine which section this column belongs to and blend at edges
            let section_f = col as f32 / COLS_PER_SECTION as f32 - 0.5;
            let section_idx = (section_f as usize).min(SECTIONS - 1);

            // Fractional position within section (0.0 = center, edges → neighbor)
            let frac = section_f - section_idx as f32;

            // Blend with neighbor section at the edges (last 30% on each side)
            let (hue, sat, val) = if frac < 0.3 && section_idx > 0 {
                // Blend with left neighbor
                let t = frac / 0.3; // 0 at left edge → 1 at 30% in
                let t = ease_in_out(t);
                let prev = section_idx - 1;
                (
                    lerp_hue(s.hue[prev], s.hue[section_idx], t),
                    sec_sat[prev] + (sec_sat[section_idx] - sec_sat[prev]) * t,
                    sec_val[prev] + (sec_val[section_idx] - sec_val[prev]) * t,
                )
            } else if frac > 0.7 && section_idx + 1 < SECTIONS {
                // Blend with right neighbor
                let t = (frac - 0.7) / 0.3; // 0 at 70% → 1 at right edge
                let t = ease_in_out(t);
                let next = section_idx + 1;
                (
                    lerp_hue(s.hue[section_idx], s.hue[next], t),
                    sec_sat[section_idx] + (sec_sat[next] - sec_sat[section_idx]) * t,
                    sec_val[section_idx] + (sec_val[next] - sec_val[section_idx]) * t,
                )
            } else {
                (s.hue[section_idx], sec_sat[section_idx], sec_val[section_idx])
            };

            // Gentle brightness wave for organic feel
            let wave = ((col as f32 / COLS as f32) * std::f32::consts::PI
                + s.frame * 0.02)
                .sin();
            let col_mod = 1.0 + wave * 0.10;

            let final_val = (val * col_mod).clamp(0.0, 1.0);

            // Paint this column across all rows (full-height block)
            for row in 0..ROWS {
                // Slight vertical variation so it's not perfectly uniform
                let row_wave = ((row as f32 / ROWS as f32) * std::f32::consts::PI
                    + s.frame * 0.015 + col as f32 * 0.05)
                    .sin();
                let row_mod = 1.0 + row_wave * 0.08;
                let pixel_val = (final_val * row_mod).clamp(0.0, 1.0);

                let led_idx = row * COLS + col;
                strip.set(led_idx, hsv_to_rgb(hue, sat, pixel_val));
            }
        }
    });
}

/// Smooth S-curve ease for natural fade-in / fade-out.
fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Lerp between two hues on the shortest path around the color wheel.
fn lerp_hue(a: f32, b: f32, t: f32) -> f32 {
    let mut diff = b - a;
    if diff > 180.0 {
        diff -= 360.0;
    } else if diff < -180.0 {
        diff += 360.0;
    }
    (a + diff * t).rem_euclid(360.0)
}
