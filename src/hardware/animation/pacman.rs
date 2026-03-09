use crate::command::AudioAnalysis;
use crate::hardware::led::LedStrip;
use std::cell::RefCell;

const COLS: usize = 60;
const ROWS: usize = 6;

// Pac-Man is ~5 cols wide, centered vertically on the 6-row grid
const PAC_RADIUS: f32 = 2.5;
const DOT_SPACING: usize = 5; // dots every N columns
const POWER_PELLET_EVERY: usize = 4; // every Nth dot is a power pellet

struct PacState {
    frame: usize,
    pac_x: f32,         // Pac-Man horizontal position (fractional col)
    speed: f32,         // current movement speed (cols per frame)
    mouth_open: f32,    // 0.0 = closed, 1.0 = fully open
    mouth_target: f32,  // target mouth position
    dots: [bool; COLS], // which columns have dots
    energy: f32,
    bass_smooth: f32,
    kick_prev: u8,
    beat_phase_prev: u8,
    chomp_timer: f32,    // counts down after eating a dot
    ghost_x: [f32; 2],   // ghost positions
    ghost_hue: [f32; 2], // ghost colors
}

thread_local! {
    static STATE: RefCell<PacState> = RefCell::new(PacState {
        frame: 0,
        pac_x: 3.0,
        speed: 0.3,
        mouth_open: 0.0,
        mouth_target: 0.0,
        dots: [false; COLS],
        energy: 0.3,
        bass_smooth: 0.0,
        kick_prev: 0,
        beat_phase_prev: 0,
        chomp_timer: 0.0,
        ghost_x: [45.0, 55.0],
        ghost_hue: [0.0, 0.55],
    });
}

/// Pac-Man animation — Pac-Man moves across the strip eating dots on beats.
/// Mouth opens and closes to the rhythm. Ghosts trail behind.
pub fn pacman(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();

        // Initialize dots on first frame
        if s.frame == 0 {
            for col in 0..COLS {
                if col % DOT_SPACING == DOT_SPACING / 2 {
                    s.dots[col] = true;
                }
            }
            // Place ghosts ahead of pac-man
            s.ghost_x[0] = 30.0;
            s.ghost_x[1] = 45.0;
            s.ghost_hue[0] = 0.0; // red ghost
            s.ghost_hue[1] = 0.55; // cyan ghost
        }
        s.frame = s.frame.wrapping_add(1);

        // ── Audio ──
        let bass_f = ((a.bands[0] as u16 + a.bands[1] as u16) / 2) as f32 / 255.0;
        smooth(
            &mut s.energy,
            a.bands.iter().map(|&b| b as f32).sum::<f32>() / (255.0 * 8.0),
            0.3,
            0.05,
        );
        smooth(&mut s.bass_smooth, bass_f, 0.4, 0.08);
        let energy = s.energy.max(0.15);

        // Detect beat: kick onset or beat_phase wrapping
        let kick_onset = a.kick > 150 && s.kick_prev < 100;
        let beat_wrap = a.beat_phase < 30 && s.beat_phase_prev > 200;
        let on_beat = kick_onset || beat_wrap;
        s.kick_prev = a.kick;
        s.beat_phase_prev = a.beat_phase;

        // ── Mouth: snaps open on beat, smoothly closes ──
        if on_beat {
            s.mouth_target = 1.0;
        } else {
            // Use beat_phase to drive continuous mouth movement
            let phase = a.beat_phase as f32 / 255.0;
            // Mouth opens at phase 0, closes by phase 0.5
            let mouth_from_phase = if phase < 0.3 {
                1.0 - phase / 0.3
            } else if phase > 0.7 {
                (phase - 0.7) / 0.3
            } else {
                0.0
            };
            s.mouth_target = mouth_from_phase * (0.5 + energy * 0.5);
        }
        smooth(&mut s.mouth_open, s.mouth_target, 0.5, 0.2);

        // ── Movement speed: tempo-driven ──
        let base_speed = if a.bpm > 0 {
            (a.bpm as f32 / 120.0) * 0.3
        } else {
            0.2 + energy * 0.3
        };
        smooth(&mut s.speed, base_speed.clamp(0.15, 0.8), 0.1, 0.05);
        s.pac_x += s.speed;
        if s.pac_x >= COLS as f32 {
            s.pac_x -= COLS as f32;
        }

        // ── Eat dots ──
        let pac_col = s.pac_x as usize % COLS;
        if s.dots[pac_col] {
            s.dots[pac_col] = false;
            s.chomp_timer = 8.0;
        }
        s.chomp_timer = (s.chomp_timer - 1.0).max(0.0);

        // Respawn dots far ahead of pac-man
        for col in 0..COLS {
            if !s.dots[col] && col % DOT_SPACING == DOT_SPACING / 2 {
                let dist = ((col as f32 - s.pac_x + COLS as f32) % COLS as f32)
                    .min((s.pac_x - col as f32 + COLS as f32) % COLS as f32);
                // Respawn if far enough ahead
                let ahead = (col as f32 - s.pac_x + COLS as f32) % COLS as f32;
                if ahead > 20.0 && dist > 15.0 {
                    s.dots[col] = true;
                }
            }
        }

        // ── Move ghosts: they follow pac-man at a distance ──
        for i in 0..2 {
            let target_dist = 12.0 + i as f32 * 8.0;
            let target_x = (s.pac_x - target_dist + COLS as f32) % COLS as f32;
            // Move ghost toward target
            let dx = ((target_x - s.ghost_x[i]) + COLS as f32) % COLS as f32;
            if dx < COLS as f32 / 2.0 {
                s.ghost_x[i] += dx * 0.06;
            } else {
                s.ghost_x[i] -= (COLS as f32 - dx) * 0.06;
            }
            if s.ghost_x[i] >= COLS as f32 {
                s.ghost_x[i] -= COLS as f32;
            }
            if s.ghost_x[i] < 0.0 {
                s.ghost_x[i] += COLS as f32;
            }
        }

        // ── Clear strip ──
        // Dark blue background (pac-man maze feel)
        for row in 0..ROWS {
            for col in 0..COLS {
                strip.set(row * COLS + col, [0, 0, 8, 0]);
            }
        }

        // ── Draw dots ──
        for col in 0..COLS {
            if s.dots[col] {
                let dot_idx = col / DOT_SPACING;
                let is_power = dot_idx % POWER_PELLET_EVERY == 0;
                if is_power {
                    // Power pellet: larger, brighter, pulses
                    let pulse = 0.7 + 0.3 * ((s.frame as f32 * 0.1).sin());
                    let bright = (200.0 * pulse) as u8;
                    // 3 rows centered
                    for row in 1..5 {
                        strip.set(row * COLS + col, [bright, bright, bright, 0]);
                    }
                } else {
                    // Small dot: single pixel in middle rows
                    for row in 2..4 {
                        strip.set(row * COLS + col, [180, 180, 140, 0]);
                    }
                }
            }
        }

        // ── Draw ghosts ──
        for i in 0..2 {
            let gx = s.ghost_x[i];
            let hue = s.ghost_hue[i];
            let (gr, gg, gb) = hsv_simple(hue, 0.9, 0.8);
            // Ghost body: 4 cols wide, 5 rows tall
            for row in 0..ROWS {
                for dc in -2i32..=2 {
                    let col = ((gx as i32 + dc) % COLS as i32 + COLS as i32) as usize % COLS;
                    let dist = dc.unsigned_abs() as f32;
                    // Top is rounded
                    if row == 0 && dist > 1.0 {
                        continue;
                    }
                    // Bottom has wavy tentacles
                    if row == ROWS - 1 {
                        let wiggle = ((s.frame as f32 * 0.15 + dc as f32 * 1.5).sin() > 0.0) as u8;
                        if wiggle == 0 {
                            continue;
                        }
                    }
                    let dim = 1.0 - dist * 0.15;
                    let r = (gr as f32 * dim) as u8;
                    let g = (gg as f32 * dim) as u8;
                    let b = (gb as f32 * dim) as u8;
                    strip.set(row * COLS + col, [r, g, b, 0]);
                }
            }
            // Ghost eyes: white with blue pupils
            let eye_l = ((gx as i32 - 1) % COLS as i32 + COLS as i32) as usize % COLS;
            let eye_r = ((gx as i32 + 1) % COLS as i32 + COLS as i32) as usize % COLS;
            strip.set(1 * COLS + eye_l, [220, 220, 255, 0]);
            strip.set(1 * COLS + eye_r, [220, 220, 255, 0]);
            strip.set(2 * COLS + eye_l, [40, 40, 200, 0]);
            strip.set(2 * COLS + eye_r, [40, 40, 200, 0]);
        }

        // ── Draw Pac-Man ──
        let mouth_angle = s.mouth_open * 45.0; // max 45 degrees opening
        let cx = s.pac_x;
        let cy = (ROWS as f32 - 1.0) / 2.0; // vertically centered

        // Chomp flash: brief brightening when eating
        let chomp_boost = if s.chomp_timer > 0.0 { 0.15 } else { 0.0 };

        for row in 0..ROWS {
            for dc in -3i32..=3 {
                let col = ((cx as i32 + dc) % COLS as i32 + COLS as i32) as usize % COLS;
                let dx = dc as f32;
                let dy = row as f32 - cy;
                let dist = (dx * dx + dy * dy).sqrt();

                if dist > PAC_RADIUS + 0.3 {
                    continue;
                }

                // Check if this pixel is inside the mouth opening
                let angle = dy.atan2(dx).to_degrees(); // -180..180, 0 = right (direction of travel)
                let mouth_deg = mouth_angle;
                let in_mouth = angle.abs() < mouth_deg && dx > -0.5;

                if in_mouth {
                    // Mouth interior: dark (shows background)
                    continue;
                }

                // Pac-Man body: yellow with slight edge darkening
                let edge = (dist / PAC_RADIUS).min(1.0);
                let bright = (1.0 - edge * 0.3 + chomp_boost).min(1.0);
                let r = (255.0 * bright) as u8;
                let g = (220.0 * bright) as u8;
                strip.set(row * COLS + col, [r, g, 0, 0]);
            }
        }
    });
}

fn hsv_simple(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = ((h % 1.0) + 1.0) % 1.0;
    let c = v * s;
    let x = c * (1.0 - ((h * 6.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h * 6.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

fn smooth(current: &mut f32, target: f32, attack: f32, decay: f32) {
    if target > *current {
        *current += (target - *current) * attack;
    } else {
        *current += (target - *current) * decay;
    }
}
