use crate::command::AudioAnalysis;
use crate::hardware::led::{hsv_to_rgb, LedStrip};
use std::cell::RefCell;

const COLS: usize = 60;
const ROWS: usize = 6;
const NOTE_HUES: [f32; 12] = [
    0.0, 30.0, 55.0, 80.0, 120.0, 160.0,
    195.0, 220.0, 260.0, 285.0, 320.0, 345.0,
];
const CHASE_PHASE: [usize; 6] = [0, 37, 17, 53, 8, 44];
const PULSE_PHASE: [f32; 6] = [0.0, 1.05, 2.09, 3.14, 4.19, 5.24];
const MAX_SPARKS: usize = 20;
const MAX_RIPPLES: usize = 6;

struct Spark {
    col_frac: f32,
    row: usize,
    life: f32,
    max_life: f32,
    hue: f32,
}

struct Ripple {
    age: f32,
    hue: f32,
}

struct Syn4State {
    frame: usize,
    // Chord / note
    chord_hue: f32,
    chord_hue_target: f32,
    // Energy
    energy: f32,
    energy_smooth: f32, // for energy bar block
    beat_brightness: f32,
    // Melody dot
    dot_x: f32,
    dot_y: f32,
    dot_prev_x: f32,
    dot_prev_y: f32,
    dot_velocity: f32,
    dot_trail: Vec<(f32, f32, f32, f32)>, // x, y, hue, life
    // Layout
    active_count: usize,
    perm: [usize; 6],
    change_cooldown: usize,
    // Hybrid block particles
    sparks: Vec<Spark>,
    ripples: Vec<Ripple>,
    // Onset
    prev_kick: u8,
    prev_snare: u8,
    kick_cooldown: usize,
    // RNG
    seed: u32,
}

impl Syn4State {
    fn rand(&mut self) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed
    }
}

thread_local! {
    static STATE: RefCell<Syn4State> = RefCell::new(Syn4State {
        frame: 0,
        chord_hue: 0.0,
        chord_hue_target: 0.0,
        energy: 0.0,
        energy_smooth: 0.0,
        beat_brightness: 0.0,
        dot_x: 30.0,
        dot_y: 2.5,
        dot_prev_x: 30.0,
        dot_prev_y: 2.5,
        dot_velocity: 0.0,
        dot_trail: Vec::new(),
        active_count: 4,
        perm: [0, 1, 2, 3, 4, 5],
        change_cooldown: 0,
        sparks: Vec::new(),
        ripples: Vec::new(),
        prev_kick: 0,
        prev_snare: 0,
        kick_cooldown: 0,
        seed: 54321,
    });
}

/// Synesthesia V4 — melody dot detection drives hybrid block layout changes.
/// The melody dot + vivid trail floats on top. Hybrid's 6 block types
/// (pulse, h-chase, sparkle, v-chase, energy bar, ripple) fill the background.
/// Fast melody movement triggers layout reshuffles; kick/snare spawn ripples.
pub fn audio_synesthesia4(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();
        s.frame = s.frame.wrapping_add(1);
        let frame = s.frame;

        let kick_f = a.kick as f32 / 255.0;
        let _snare_f = a.snare as f32 / 255.0;
        let hihat_f = a.hihat as f32 / 255.0;
        let vocals_f = a.vocals as f32 / 255.0;
        let _bass_line_f = a.bass_line as f32 / 255.0;
        let beat_phase = a.beat_phase as f32 / 255.0;
        let bass_f = ((a.bands[0] as u16 + a.bands[1] as u16) / 2) as f32 / 255.0;
        let mid_f = ((a.bands[3] as u16 + a.bands[4] as u16) / 2) as f32 / 255.0;
        let treble_f = ((a.bands[6] as u16 + a.bands[7] as u16) / 2) as f32 / 255.0;
        let high_f = ((a.bands[5] as u16 + a.bands[6] as u16 + a.bands[7] as u16) / 3) as f32 / 255.0;

        // ---- Energy ----
        let raw_energy: f32 = a.bands.iter().map(|&b| b as f32).sum::<f32>() / (255.0 * 8.0);
        if raw_energy > s.energy {
            s.energy += (raw_energy - s.energy) * 0.3;
        } else {
            s.energy += (raw_energy - s.energy) * 0.05;
        }
        let energy = s.energy.max(0.15);

        // Energy bar smooth
        if mid_f > s.energy_smooth {
            s.energy_smooth += (mid_f - s.energy_smooth) * 0.35;
        } else {
            s.energy_smooth += (mid_f - s.energy_smooth) * 0.1;
        }

        // ---- Chord hue ----
        if a.chord_root < 12 {
            s.chord_hue_target = NOTE_HUES[a.chord_root as usize];
        }
        s.chord_hue = lerp_hue(s.chord_hue, s.chord_hue_target, 0.08);

        // ---- Beat ----
        let beat_pulse = ((1.0 - beat_phase) * std::f32::consts::TAU).cos() * 0.5 + 0.5;
        s.beat_brightness += (beat_pulse - s.beat_brightness) * 0.3;

        // ---- Spectral centroid ----
        let total: f32 = a.bands.iter().map(|&b| b as f32).sum::<f32>().max(1.0);
        let norm: [f32; 8] = std::array::from_fn(|i| a.bands[i] as f32 / total);
        let centroid: f32 = norm.iter().enumerate().map(|(i, &n)| i as f32 * n).sum();

        // ============================================
        // MELODY DOT — tracks pitch
        // ============================================
        s.dot_prev_x = s.dot_x;
        s.dot_prev_y = s.dot_y;

        if a.note_midi > 0 {
            let target_x = ((a.note_midi as f32 - 36.0) / 48.0).clamp(0.0, 1.0)
                * (COLS - 1) as f32;
            let target_y = (centroid / 7.0).clamp(0.0, 1.0) * (ROWS - 1) as f32;
            s.dot_x += (target_x - s.dot_x) * 0.25;
            s.dot_y += (target_y - s.dot_y) * 0.25;
        } else {
            let t = frame as f32 * 0.015;
            s.dot_x += t.cos() * 0.3;
            s.dot_y += (t * 0.7).sin() * 0.15;
            s.dot_x = s.dot_x.clamp(1.0, (COLS - 2) as f32);
            s.dot_y = s.dot_y.clamp(0.0, (ROWS - 1) as f32);
        }

        // Velocity
        let dx = s.dot_x - s.dot_prev_x;
        let dy = (s.dot_y - s.dot_prev_y) * 3.0;
        let instant_vel = (dx * dx + dy * dy).sqrt();
        if instant_vel > s.dot_velocity {
            s.dot_velocity += (instant_vel - s.dot_velocity) * 0.5;
        } else {
            s.dot_velocity += (instant_vel - s.dot_velocity) * 0.15;
        }

        // Trail
        let dot_hue = if a.note_midi > 0 {
            NOTE_HUES[a.note_midi as usize % 12]
        } else {
            (s.chord_hue + frame as f32 * 0.3) % 360.0
        };
        if s.dot_trail.len() < 50 {
            s.dot_trail.push((s.dot_x, s.dot_y, dot_hue, 0.0));
        }

        // ============================================
        // LAYOUT CHANGE — triggered by dot velocity
        // ============================================
        s.change_cooldown = s.change_cooldown.saturating_sub(1);

        if s.dot_velocity > 2.0 && s.change_cooldown == 0 {
            s.change_cooldown = 30;

            // Block count from velocity
            let vel_norm = (s.dot_velocity / 8.0).clamp(0.0, 1.0);
            s.active_count = ((vel_norm * 4.99) as usize + 2).min(6);

            // Permutation from dot state
            let px = (s.dot_x * 4.0) as u32;
            let py = (s.dot_y * 8.0) as u32;
            let pv = (s.dot_velocity * 10.0) as u32;
            let mut h = px.wrapping_mul(7) ^ py.wrapping_mul(13) ^ pv.wrapping_mul(19);
            let mut perm = [0usize, 1, 2, 3, 4, 5];
            for i in (1..6).rev() {
                h = h.wrapping_mul(1664525).wrapping_add(1013904223);
                let j = (h as usize) % (i + 1);
                perm.swap(i, j);
            }
            s.perm = perm;

            // Spawn a ripple on layout change
            if s.ripples.len() < MAX_RIPPLES {
                s.ripples.push(Ripple { age: 0.0, hue: dot_hue });
            }
        }

        // ---- Onset detection ----
        let kick_onset = a.kick > s.prev_kick.saturating_add(25);
        s.prev_kick = a.kick;
        s.kick_cooldown = s.kick_cooldown.saturating_sub(1);
        let snare_onset = a.snare > s.prev_snare.saturating_add(30);
        s.prev_snare = a.snare;

        // Kick/snare also spawn ripples
        if kick_onset && s.kick_cooldown == 0 && s.ripples.len() < MAX_RIPPLES {
            s.kick_cooldown = 6;
            let hue = (s.chord_hue + 180.0) % 360.0;
            s.ripples.push(Ripple { age: 0.0, hue });
        }
        if snare_onset && s.ripples.len() < MAX_RIPPLES {
            let hue = (s.chord_hue + 60.0) % 360.0;
            s.ripples.push(Ripple { age: 0.0, hue });
        }

        // ---- Sparkle particles (for sparkle blocks) ----
        let spawn_count = (high_f * 3.0) as usize;
        for _ in 0..spawn_count {
            if s.sparks.len() >= MAX_SPARKS { break; }
            let col_frac = (s.rand() % 1000) as f32 / 1000.0;
            let row = (s.rand() as usize) % ROWS;
            let max_life = 8.0 + (s.rand() % 20) as f32;
            let hue = (mid_f * 360.0 + (s.rand() % 60) as f32 - 30.0).rem_euclid(360.0);
            s.sparks.push(Spark { col_frac, row, life: 0.0, max_life, hue });
        }

        // ---- Chase color from chord + dominant instrument ----
        let chase_hue = if a.chord_root < 12 {
            (NOTE_HUES[a.chord_root as usize] + 120.0) % 360.0
        } else {
            s.chord_hue
        };
        let chase_color = hsv_to_rgb(chase_hue, 1.0, 1.0);
        let bass_u = ((a.bands[0] as u16 + a.bands[1] as u16) / 2) as usize;
        let chase_speed = 1 + bass_u / 32;

        // ---- Clear ----
        strip.set_all([0, 0, 0, 0]);

        // ============================================
        // RENDER MELODY DOT TRAIL (behind blocks)
        // ============================================
        let trail_max_life = 40.0;
        for &(tx, ty, thue, life) in s.dot_trail.iter() {
            let fade = (1.0 - life / trail_max_life).max(0.0);
            let fade = fade * fade;
            let b = fade * 0.5;
            if b < 0.02 { continue; }
            draw_glow(strip, tx, ty, thue, 0.85, b, 1.5);
        }

        // ============================================
        // RENDER HYBRID BLOCKS
        // ============================================
        let active_count = s.active_count;
        let block_width = COLS / active_count;
        let block_leds = block_width * ROWS;

        for slot in 0..active_count {
            let col_start = slot * block_width;
            let block_type = s.perm[slot % 6] % 6;

            match block_type {
                0 => {
                    // ---- PULSE (driven by kick + beat) ----
                    let phase = PULSE_PHASE[slot % 6];
                    let pulse_mod = (frame as f32 * 0.08 + phase).sin() * 0.5 + 0.5;
                    let kick_boost = kick_f * 0.5;
                    let brightness = ((bass_f + kick_boost) * (0.4 + pulse_mod * 0.6))
                        .max(energy * 0.15);
                    let hue = if a.chord_root < 12 {
                        (NOTE_HUES[a.chord_root as usize] + slot as f32 * 30.0) % 360.0
                    } else {
                        (treble_f * 360.0 + slot as f32 * 30.0) % 360.0
                    };
                    let color = hsv_to_rgb(hue, 1.0, brightness);
                    fill_block(strip, col_start, block_width, color);
                }
                1 => {
                    // ---- HORIZONTAL CHASE ----
                    render_chase(strip, frame, col_start, block_width, block_leds,
                                 slot, chase_speed, &chase_color, false);
                }
                2 => {
                    // ---- SPARKLE (driven by hihat + high freq) ----
                    let hihat_bg = hihat_f * 0.08;
                    let bg = hsv_to_rgb(s.chord_hue, 0.6,
                        (bass_f * 0.12).max(hihat_bg).max(energy * 0.05));
                    fill_block(strip, col_start, block_width, bg);
                    for spark in s.sparks.iter() {
                        let spark_col = (spark.col_frac * block_width as f32) as usize;
                        let spark_col = spark_col.min(block_width - 1);
                        let t = spark.life / spark.max_life;
                        let brightness = if t < 0.15 { t / 0.15 }
                            else { (1.0 - (t - 0.15) / 0.85).max(0.0) };
                        let brightness = brightness * brightness;
                        let color = hsv_to_rgb(spark.hue, 0.7, brightness);
                        let led = spark.row * COLS + col_start + spark_col;
                        strip.set(led, color);
                    }
                }
                3 => {
                    // ---- VERTICAL CHASE ----
                    render_chase(strip, frame, col_start, block_width, block_leds,
                                 slot, chase_speed, &chase_color, true);
                }
                4 => {
                    // ---- ENERGY BAR (driven by vocals + mid) ----
                    let bar_energy = s.energy_smooth.max(vocals_f * 0.5).max(energy * 0.2);
                    let half = block_width / 2;
                    let bar_half = (bar_energy * half as f32) as usize;
                    let hue = if vocals_f > 0.2 {
                        (s.chord_hue + 30.0) % 360.0 // warm when vocals
                    } else {
                        150.0 + mid_f * 60.0
                    };

                    for row in 0..ROWS {
                        for col_off in 0..block_width {
                            let led = row * COLS + col_start + col_off;
                            let dist = if col_off >= half { col_off - half }
                                else { half - 1 - col_off };
                            if dist < bar_half {
                                let t = dist as f32 / bar_half.max(1) as f32;
                                let val = 1.0 - t * 0.4;
                                add_color(strip, led, hsv_to_rgb(hue, 0.85, val));
                            } else if dist < bar_half + 2 {
                                let glow = 0.1 * (1.0 - (dist - bar_half) as f32 / 2.0);
                                add_color(strip, led, hsv_to_rgb(hue, 0.5, glow));
                            }
                        }
                    }
                }
                5 => {
                    // ---- RIPPLE (triggered by kick, snare, layout changes) ----
                    let cx = block_width as f32 / 2.0;
                    let cy = ROWS as f32 / 2.0;
                    let max_radius = (block_width as f32 / 2.0) + 2.0;
                    let mut has_active = false;

                    for ripple in s.ripples.iter() {
                        let radius = ripple.age * 0.8;
                        let fade = (1.0 - ripple.age / 50.0).max(0.0);
                        let fade = fade * fade;
                        let ring_w = 1.5 + ripple.age * 0.15;
                        let color = hsv_to_rgb(ripple.hue, 0.9, 1.0);

                        for row in 0..ROWS {
                            for col_off in 0..block_width {
                                let ddx = col_off as f32 - cx;
                                let ddy = row as f32 - cy;
                                let dist = (ddx * ddx + ddy * ddy).sqrt();
                                let ring_dist = (dist - radius).abs();
                                if ring_dist < ring_w && dist < max_radius {
                                    has_active = true;
                                    let b = (1.0 - ring_dist / ring_w) * fade;
                                    let led = row * COLS + col_start + col_off;
                                    add_color(strip, led, [
                                        (color[0] as f32 * b) as u8,
                                        (color[1] as f32 * b) as u8,
                                        (color[2] as f32 * b) as u8,
                                        0,
                                    ]);
                                }
                            }
                        }
                    }

                    // Idle: breathing ring
                    if !has_active {
                        let idle_r = ((frame as f32 * 0.04).sin() * 0.5 + 0.5)
                            * (cx.min(cy) + 1.0);
                        let idle_b = energy * 0.4;
                        let idle_hue = (s.chord_hue + 90.0) % 360.0;
                        for row in 0..ROWS {
                            for col_off in 0..block_width {
                                let ddx = col_off as f32 - cx;
                                let ddy = row as f32 - cy;
                                let dist = (ddx * ddx + ddy * ddy).sqrt();
                                let rd = (dist - idle_r).abs();
                                if rd < 1.0 {
                                    let b = (1.0 - rd) * idle_b;
                                    if b > 0.02 {
                                        add_color(strip, row * COLS + col_start + col_off,
                                            hsv_to_rgb(idle_hue, 0.85, b));
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // ============================================
        // RENDER DOT HEAD (on top of everything)
        // ============================================
        {
            let b = if a.note_midi > 0 { 1.0 } else { energy * 0.5 };
            draw_glow(strip, s.dot_x, s.dot_y, dot_hue, 0.8, b, 2.0);
        }

        // ---- Age / cull ----
        s.dot_trail.iter_mut().for_each(|t| t.3 += 1.0);
        s.dot_trail.retain(|t| t.3 < trail_max_life);
        s.sparks.iter_mut().for_each(|sp| sp.life += 1.0);
        s.sparks.retain(|sp| sp.life < sp.max_life);
        s.ripples.iter_mut().for_each(|r| r.age += 1.0);
        s.ripples.retain(|r| r.age < 50.0);
    });
}

fn fill_block(strip: &mut LedStrip, col_start: usize, width: usize, color: [u8; 4]) {
    for row in 0..ROWS {
        for col in col_start..(col_start + width) {
            add_color(strip, row * COLS + col, color);
        }
    }
}

fn render_chase(
    strip: &mut LedStrip, frame: usize, col_start: usize,
    block_width: usize, block_leds: usize, slot: usize,
    chase_speed: usize, chase_color: &[u8; 4], vertical: bool,
) {
    let phase = CHASE_PHASE[slot % 6];
    let pos = ((frame + phase) * chase_speed) % block_leds;
    let tail_len = (block_leds / 5).max(6);

    for row in 0..ROWS {
        for col_off in 0..block_width {
            let local_idx = if vertical {
                col_off * ROWS + row
            } else {
                row * block_width + col_off
            };
            let led_idx = row * COLS + col_start + col_off;
            let dist = ((local_idx as isize) - (pos as isize)).unsigned_abs() % block_leds;
            let dist = dist.min(block_leds - dist);
            if dist < tail_len {
                let fade = 1.0 - (dist as f32 / tail_len as f32);
                add_color(strip, led_idx, [
                    (chase_color[0] as f32 * fade) as u8,
                    (chase_color[1] as f32 * fade) as u8,
                    (chase_color[2] as f32 * fade) as u8,
                    0,
                ]);
            }
        }
    }
}

fn draw_glow(strip: &mut LedStrip, x: f32, y: f32, hue: f32, sat: f32, brightness: f32, radius: f32) {
    if brightness < 0.02 { return; }
    let r_ceil = radius.ceil() as isize;
    let cx = x.round() as isize;
    let cy = y.round() as isize;
    for dy in -r_ceil..=r_ceil {
        let row = cy + dy;
        if row < 0 || row >= ROWS as isize { continue; }
        for dx in -r_ceil..=r_ceil {
            let col = cx + dx;
            if col < 0 || col >= COLS as isize { continue; }
            let dist_x = col as f32 - x;
            let dist_y = (row as f32 - y) * 2.0;
            let dist = (dist_x * dist_x + dist_y * dist_y).sqrt();
            if dist > radius { continue; }
            let falloff = 1.0 - dist / radius;
            let b = brightness * falloff * falloff;
            if b < 0.01 { continue; }
            let color = hsv_to_rgb(hue, sat, b);
            let idx = row as usize * COLS + col as usize;
            add_color(strip, idx, color);
        }
    }
}

fn lerp_hue(a: f32, b: f32, t: f32) -> f32 {
    let mut diff = b - a;
    if diff > 180.0 { diff -= 360.0; }
    else if diff < -180.0 { diff += 360.0; }
    (a + diff * t).rem_euclid(360.0)
}

fn add_color(strip: &mut LedStrip, idx: usize, color: [u8; 4]) {
    let leds = strip.controller_leds();
    let existing = leds[idx];
    strip.set(idx, [
        existing[0].max(color[0]),
        existing[1].max(color[1]),
        existing[2].max(color[2]),
        0,
    ]);
}
