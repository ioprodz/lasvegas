use crate::command::AudioAnalysis;
use crate::hardware::led::LedStrip;
use std::cell::RefCell;

const COLS: usize = 60;
const ROWS: usize = 6;
const NUM_FLAMES: usize = 12; // distinct flame tongues across the strip

struct Flame {
    center: f32,    // column center (fractional)
    width: f32,     // how many columns wide
    height: f32,    // current flame height (0..1 maps to rows)
    sway: f32,      // lateral sway phase
    speed: f32,     // sway speed multiplier
    intensity: f32, // brightness multiplier
}

struct FireState {
    heat: [[f32; COLS]; ROWS],
    flames: [Flame; NUM_FLAMES],
    frame: usize,
    energy: f32,
    bass_smooth: f32,
    mid_smooth: f32,
    seed: u32,
}

impl FireState {
    fn rand(&mut self) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed
    }
    fn randf(&mut self) -> f32 {
        (self.rand() % 10000) as f32 / 10000.0
    }
}

const INIT_FLAME: Flame = Flame {
    center: 0.0,
    width: 3.0,
    height: 0.3,
    sway: 0.0,
    speed: 1.0,
    intensity: 0.5,
};

thread_local! {
    static STATE: RefCell<FireState> = RefCell::new(FireState {
        heat: [[0.0; COLS]; ROWS],
        flames: [INIT_FLAME; NUM_FLAMES],
        frame: 0,
        energy: 0.3,
        bass_smooth: 0.0,
        mid_smooth: 0.0,
        seed: 31337,
    });
}

/// Audio Fire — distinct flame tongues that sway and reach with the music.
pub fn audio_fire(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();

        // Initialize flame positions on first frame
        if s.frame == 0 {
            for i in 0..NUM_FLAMES {
                let spacing = COLS as f32 / NUM_FLAMES as f32;
                s.flames[i].center = spacing * i as f32 + spacing * 0.5;
                let w = 2.0 + s.randf() * 2.5;
                let sp = 0.6 + s.randf() * 0.8;
                let sw = s.randf() * 6.28;
                s.flames[i].width = w;
                s.flames[i].speed = sp;
                s.flames[i].sway = sw;
            }
        }
        s.frame = s.frame.wrapping_add(1);

        // ── Audio ──
        let bass_f = ((a.bands[0] as u16 + a.bands[1] as u16) / 2) as f32 / 255.0;
        let mid_f = ((a.bands[3] as u16 + a.bands[4] as u16) / 2) as f32 / 255.0;
        smooth(
            &mut s.energy,
            a.bands.iter().map(|&b| b as f32).sum::<f32>() / (255.0 * 8.0),
            0.3,
            0.05,
        );
        smooth(&mut s.bass_smooth, bass_f, 0.4, 0.08);
        smooth(&mut s.mid_smooth, mid_f, 0.3, 0.06);
        let energy = s.energy.max(0.15);
        let intensity = (energy * 0.5 + s.bass_smooth * 0.5).clamp(0.0, 1.0);

        // ── Update flames ──
        let t = s.frame as f32 * 0.04;
        for i in 0..NUM_FLAMES {
            let f = &mut s.flames[i];
            // Sway: lateral movement driven by sine, speed affected by mid freq
            f.sway += f.speed * (0.03 + s.mid_smooth * 0.04);
            // Height: bass and energy make flames taller
            let target_h = 0.2 + intensity * 0.7 + s.bass_smooth * 0.3;
            // Each flame has slightly different target based on position
            let phase = (i as f32 * 1.7 + t * 0.5).sin() * 0.15;
            smooth(
                &mut f.height,
                (target_h + phase).clamp(0.1, 1.2),
                0.15,
                0.06,
            );
            // Intensity pulses per-flame
            let pulse = (i as f32 * 2.3 + t * 0.8).sin() * 0.15;
            smooth(
                &mut f.intensity,
                (0.5 + energy * 0.5 + pulse).clamp(0.2, 1.0),
                0.2,
                0.08,
            );
        }

        // ── Clear heat map ──
        for row in 0..ROWS {
            for col in 0..COLS {
                // Decay existing heat quickly
                s.heat[row][col] *= 0.3;
            }
        }

        // ── Paint each flame onto the heat map ──
        // Pre-collect flame params to avoid borrow issues
        let flame_params: [(f32, f32, f32, f32, f32); NUM_FLAMES] = {
            let mut params = [(0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32); NUM_FLAMES];
            for i in 0..NUM_FLAMES {
                let f = &s.flames[i];
                params[i] = (f.center, f.width, f.height, f.sway, f.intensity);
            }
            params
        };

        for &(center, width, height, sway, flame_intensity) in flame_params.iter() {
            let half_w = width * 0.5;
            // Flame height in rows (0..ROWS, can exceed for overflow)
            let flame_rows = height * ROWS as f32;

            for row in 0..ROWS {
                let row_from_bottom = (ROWS - 1 - row) as f32;
                if row_from_bottom > flame_rows {
                    continue;
                }

                // Vertical falloff: strongest at bottom, tapers to tip
                let vert_t = row_from_bottom / flame_rows.max(0.01);
                let vert_strength = 1.0 - vert_t * vert_t; // quadratic falloff

                // Sway: tip sways more than base
                let sway_amount = sway.sin() * vert_t * 2.5;
                let effective_center = center + sway_amount;

                // Narrower at the tip
                let row_width = half_w * (1.0 - vert_t * 0.6);

                for col in 0..COLS {
                    let dx = (col as f32 - effective_center).abs();
                    // Wrap distance for edge flames
                    let dx = dx.min(COLS as f32 - dx);
                    if dx > row_width + 1.0 {
                        continue;
                    }
                    // Horizontal falloff: gaussian-ish
                    let horiz = if dx < row_width {
                        1.0 - (dx / row_width) * (dx / row_width)
                    } else {
                        0.0
                    };

                    // Random flicker per pixel
                    let flicker = 0.85 + s.randf() * 0.15;

                    let h = vert_strength * horiz * flame_intensity * flicker;
                    // Additive blend onto heat map
                    s.heat[row][col] = (s.heat[row][col] + h).min(1.0);
                }
            }
        }

        // ── Small random sparks between flames (fills dark gaps slightly) ──
        let bottom = ROWS - 1;
        for col in 0..COLS {
            let spark = s.randf();
            if spark < 0.15 * energy {
                let h = s.randf() * 0.25 * energy;
                s.heat[bottom][col] = (s.heat[bottom][col] + h).min(1.0);
            }
        }

        // ── Render ──
        for row in 0..ROWS {
            for col in 0..COLS {
                let heat = s.heat[row][col];
                if heat < 0.01 {
                    strip.set(row * COLS + col, [0, 0, 0, 0]);
                    continue;
                }
                let (r, g, b) = fire_color(heat);
                strip.set(row * COLS + col, [r, g, b, 0]);
            }
        }
    });
}

/// Fire color palette: heat 0..1 → (R, G, B)
/// Mostly reds and oranges. Yellow only at tips. No white.
fn fire_color(heat: f32) -> (u8, u8, u8) {
    let h = heat.clamp(0.0, 1.0);
    if h < 0.10 {
        // Black to dark red
        let t = h / 0.10;
        let r = (t * 60.0) as u8;
        (r, 0, 0)
    } else if h < 0.30 {
        // Dark red to red
        let t = (h - 0.10) / 0.20;
        let r = 60 + (t * 195.0) as u8;
        (r, 0, 0)
    } else if h < 0.55 {
        // Red to deep orange
        let t = (h - 0.30) / 0.25;
        let g = (t * 100.0) as u8;
        (255, g, 0)
    } else if h < 0.75 {
        // Deep orange to orange
        let t = (h - 0.55) / 0.20;
        let g = 100 + (t * 60.0) as u8;
        (255, g, 0)
    } else if h < 0.90 {
        // Orange to bright orange
        let t = (h - 0.75) / 0.15;
        let g = 160 + (t * 40.0) as u8;
        (255, g, 0)
    } else {
        // Bright orange tip (warm, not yellow)
        let t = (h - 0.90) / 0.10;
        let g = 200 + (t.min(1.0) * 20.0) as u8;
        (255, g, 0)
    }
}

fn smooth(current: &mut f32, target: f32, attack: f32, decay: f32) {
    if target > *current {
        *current += (target - *current) * attack;
    } else {
        *current += (target - *current) * decay;
    }
}
