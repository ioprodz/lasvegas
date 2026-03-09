use crate::command::AudioAnalysis;
use crate::hardware::led::{hsv_to_rgb, LedStrip};
use std::cell::RefCell;

const COLS: usize = 60;
const ROWS: usize = 6;
const NOTE_HUES: [f32; 12] = [
    0.0, 30.0, 55.0, 80.0, 120.0, 160.0, 195.0, 220.0, 260.0, 285.0, 320.0, 345.0,
];
const CHASE_PHASE: [usize; 6] = [0, 37, 17, 53, 8, 44];
const PULSE_PHASE: [f32; 6] = [0.0, 1.05, 2.09, 3.14, 4.19, 5.24];
const MAX_SPARKS: usize = 20;
const MAX_TRAVELERS: usize = 12;
const PROGRESSION_LEN: usize = 4;

/// A light that travels across the strip when a note changes.
/// Color is determined by the musical interval.
struct Traveler {
    x: f32,        // current column position
    target_x: f32, // destination column
    speed: f32,    // columns per frame
    hue: f32,
    life: f32, // 0..1, fades out
    row: usize,
}

struct Spark {
    col_frac: f32,
    row: usize,
    life: f32,
    max_life: f32,
    hue: f32,
}

struct HarmonicState {
    frame: usize,
    // Chord progression tracking
    chord_history: [u8; PROGRESSION_LEN], // rolling window of chord roots (0-11)
    chord_count: usize,                   // how many unique chords seen so far
    prev_chord_root: u8,                  // last chord root to detect changes
    chord_stable_frames: usize,           // frames since last chord change
    // Fingerprint & derived config
    fingerprint: u64,
    active_count: usize,
    block_types: [usize; 6],
    palette_offset: f32,
    speed_mult: f32,
    // Note tracking
    prev_note_midi: u8,
    travelers: Vec<Traveler>,
    // Audio smoothing
    chord_hue: f32,
    chord_hue_target: f32,
    energy: f32,
    energy_smooth: f32,
    beat_brightness: f32,
    // Particles
    sparks: Vec<Spark>,
    // Onset detection
    prev_kick: u8,
    prev_snare: u8,
    kick_cooldown: usize,
    // RNG
    seed: u32,
}

impl HarmonicState {
    fn rand(&mut self) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed
    }

    /// Hash the chord progression window into a deterministic fingerprint.
    fn compute_fingerprint(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325; // FNV offset
        for i in 0..PROGRESSION_LEN {
            h ^= self.chord_history[i] as u64;
            h = h.wrapping_mul(0x100000001b3); // FNV prime
        }
        h
    }

    /// Derive visual configuration from the fingerprint.
    fn apply_fingerprint(&mut self) {
        let fp = self.fingerprint;

        // Block count: 2-6, from bits 0-7
        self.active_count = 2 + ((fp & 0xFF) as usize % 5); // 2..=6

        // Block types for each slot, from subsequent byte ranges
        for i in 0..6 {
            let shift = 8 + i * 4;
            self.block_types[i] = ((fp >> shift) & 0x0F) as usize % 6;
        }

        // Palette offset: 0-330 degrees, from bits 32-39
        self.palette_offset = ((fp >> 32) & 0xFF) as f32 / 255.0 * 330.0;

        // Speed multiplier: 0.6 - 1.8, from bits 40-47
        self.speed_mult = 0.6 + ((fp >> 40) & 0xFF) as f32 / 255.0 * 1.2;
    }
}

thread_local! {
    static STATE: RefCell<HarmonicState> = RefCell::new(HarmonicState {
        frame: 0,
        chord_history: [255; PROGRESSION_LEN],
        chord_count: 0,
        prev_chord_root: 255,
        chord_stable_frames: 0,
        fingerprint: 0,
        active_count: 3,
        block_types: [0, 1, 2, 3, 4, 5],
        palette_offset: 0.0,
        speed_mult: 1.0,
        prev_note_midi: 0,
        travelers: Vec::new(),
        chord_hue: 0.0,
        chord_hue_target: 0.0,
        energy: 0.0,
        energy_smooth: 0.0,
        beat_brightness: 0.0,
        sparks: Vec::new(),
        prev_kick: 0,
        prev_snare: 0,
        kick_cooldown: 0,
        seed: 77777,
    });
}

/// Harmonic Memory — chord progressions deterministically drive layout.
/// Same chord sequence → same block configuration, colors, and speed.
/// Note transitions create colored travelers that flow across the strip.
pub fn audio_harmonic(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();
        s.frame = s.frame.wrapping_add(1);
        let frame = s.frame;

        // ---- Unpack audio ----
        let kick_f = a.kick as f32 / 255.0;
        let hihat_f = a.hihat as f32 / 255.0;
        let vocals_f = a.vocals as f32 / 255.0;
        let beat_phase = a.beat_phase as f32 / 255.0;
        let bass_f = ((a.bands[0] as u16 + a.bands[1] as u16) / 2) as f32 / 255.0;
        let mid_f = ((a.bands[3] as u16 + a.bands[4] as u16) / 2) as f32 / 255.0;
        let treble_f = ((a.bands[6] as u16 + a.bands[7] as u16) / 2) as f32 / 255.0;
        let high_f =
            ((a.bands[5] as u16 + a.bands[6] as u16 + a.bands[7] as u16) / 3) as f32 / 255.0;

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

        // ---- Chord hue (with palette offset) ----
        if a.chord_root < 12 {
            s.chord_hue_target = (NOTE_HUES[a.chord_root as usize] + s.palette_offset) % 360.0;
        }
        s.chord_hue = lerp_hue(s.chord_hue, s.chord_hue_target, 0.08);

        // ---- Beat ----
        let beat_pulse = ((1.0 - beat_phase) * std::f32::consts::TAU).cos() * 0.5 + 0.5;
        s.beat_brightness += (beat_pulse - s.beat_brightness) * 0.3;

        // ============================================
        // CHORD PROGRESSION TRACKING
        // ============================================
        s.chord_stable_frames += 1;

        if a.chord_root < 12 && a.chord_root != s.prev_chord_root && s.chord_stable_frames > 8 {
            // New chord detected — shift window
            s.chord_stable_frames = 0;
            s.prev_chord_root = a.chord_root;

            // Shift history left, append new chord
            for i in 0..(PROGRESSION_LEN - 1) {
                s.chord_history[i] = s.chord_history[i + 1];
            }
            s.chord_history[PROGRESSION_LEN - 1] = a.chord_root;
            s.chord_count = (s.chord_count + 1).min(PROGRESSION_LEN);

            // Recompute fingerprint once we have enough chords
            if s.chord_count >= 2 {
                let new_fp = s.compute_fingerprint();
                if new_fp != s.fingerprint {
                    s.fingerprint = new_fp;
                    s.apply_fingerprint();
                }
            }
        }

        // ============================================
        // NOTE TRANSITION TRAVELERS
        // ============================================
        if a.note_midi > 0 && a.note_midi != s.prev_note_midi && s.prev_note_midi > 0 {
            let from_x =
                ((s.prev_note_midi as f32 - 36.0) / 48.0).clamp(0.0, 1.0) * (COLS - 1) as f32;
            let to_x = ((a.note_midi as f32 - 36.0) / 48.0).clamp(0.0, 1.0) * (COLS - 1) as f32;

            // Interval determines color
            let interval =
                ((a.note_midi as i16 - s.prev_note_midi as i16).unsigned_abs() % 12) as usize;
            let interval_hue = NOTE_HUES[interval];

            // Direction and speed from distance
            let dist = (to_x - from_x).abs();
            let speed = (dist / 20.0).max(0.5) * s.speed_mult;

            // Spawn on a row based on interval class
            let row = match interval {
                0..=2 => 0,  // unison/2nds: top
                3..=4 => 1,  // 3rds: upper-mid
                5 => 2,      // 4th: mid
                7 => 3,      // 5th: lower-mid
                8..=11 => 4, // 6ths+: low
                _ => 5,
            };

            if s.travelers.len() < MAX_TRAVELERS {
                s.travelers.push(Traveler {
                    x: from_x,
                    target_x: to_x,
                    speed,
                    hue: interval_hue,
                    life: 1.0,
                    row,
                });
            }
        }
        if a.note_midi > 0 {
            s.prev_note_midi = a.note_midi;
        }

        // ---- Update travelers ----
        for t in s.travelers.iter_mut() {
            let dir = if t.target_x > t.x { 1.0 } else { -1.0 };
            t.x += dir * t.speed;
            // Fade out as it approaches target or ages
            let remaining = (t.target_x - t.x).abs();
            let total_dist = (t.target_x - t.x + dir * t.speed).abs() + remaining;
            if total_dist > 0.1 {
                t.life = (remaining / total_dist).min(t.life);
            }
            t.life -= 0.015;
        }
        s.travelers
            .retain(|t| t.life > 0.0 && t.x >= -2.0 && t.x < (COLS + 2) as f32);

        // ---- Onset detection ----
        let kick_onset = a.kick > s.prev_kick.saturating_add(25);
        s.prev_kick = a.kick;
        s.kick_cooldown = s.kick_cooldown.saturating_sub(1);
        let _snare_onset = a.snare > s.prev_snare.saturating_add(30);
        s.prev_snare = a.snare;

        // ---- Sparkle particles ----
        let spawn_count = (high_f * 3.0) as usize;
        for _ in 0..spawn_count {
            if s.sparks.len() >= MAX_SPARKS {
                break;
            }
            let col_frac = (s.rand() % 1000) as f32 / 1000.0;
            let row = (s.rand() as usize) % ROWS;
            let max_life = 8.0 + (s.rand() % 20) as f32;
            let hue = (mid_f * 360.0 + s.palette_offset + (s.rand() % 60) as f32 - 30.0)
                .rem_euclid(360.0);
            s.sparks.push(Spark {
                col_frac,
                row,
                life: 0.0,
                max_life,
                hue,
            });
        }

        // ---- Chase color ----
        let chase_hue = if a.chord_root < 12 {
            (NOTE_HUES[a.chord_root as usize] + s.palette_offset + 120.0) % 360.0
        } else {
            s.chord_hue
        };
        let chase_color = hsv_to_rgb(chase_hue, 1.0, 1.0);
        let bass_u = ((a.bands[0] as u16 + a.bands[1] as u16) / 2) as usize;
        let chase_speed = (1 + bass_u / 32) as f32 * s.speed_mult;

        // ---- Clear ----
        strip.set_all([0, 0, 0, 0]);

        // ============================================
        // RENDER NOTE TRAVELERS (behind blocks, like shooting stars)
        // ============================================
        for t in s.travelers.iter() {
            let alpha = t.life * t.life; // quadratic fade
            let tail_len = 6.0;
            let dir = if t.target_x > t.x { -1.0 } else { 1.0 }; // trail behind

            for i in 0..((tail_len + 1.0) as usize) {
                let px = t.x + dir * i as f32;
                let col = px.round() as isize;
                if col < 0 || col >= COLS as isize {
                    continue;
                }

                let fade = (1.0 - i as f32 / tail_len) * alpha;
                if fade < 0.02 {
                    continue;
                }

                // Render on the traveler's row and adjacent rows
                for dr in -1i32..=1 {
                    let row = t.row as i32 + dr;
                    if row < 0 || row >= ROWS as i32 {
                        continue;
                    }
                    let row_fade = if dr == 0 { 1.0 } else { 0.3 };
                    let b = fade * row_fade;
                    let color = hsv_to_rgb(t.hue, 0.85, b);
                    let idx = row as usize * COLS + col as usize;
                    add_color(strip, idx, color);
                }
            }
        }

        // ============================================
        // RENDER BLOCKS (deterministic from fingerprint)
        // ============================================
        let active_count = s.active_count;
        let block_width = COLS / active_count;

        for slot in 0..active_count {
            let col_start = slot * block_width;
            let block_type = s.block_types[slot % 6];
            let block_leds = block_width * ROWS;

            match block_type {
                0 => {
                    // ---- PULSE ----
                    let phase = PULSE_PHASE[slot % 6];
                    let pulse_mod = (frame as f32 * 0.08 * s.speed_mult + phase).sin() * 0.5 + 0.5;
                    let kick_boost = kick_f * 0.5;
                    let brightness =
                        ((bass_f + kick_boost) * (0.4 + pulse_mod * 0.6)).max(energy * 0.15);
                    let hue = if a.chord_root < 12 {
                        (NOTE_HUES[a.chord_root as usize] + s.palette_offset + slot as f32 * 30.0)
                            % 360.0
                    } else {
                        (treble_f * 360.0 + s.palette_offset + slot as f32 * 30.0) % 360.0
                    };
                    let color = hsv_to_rgb(hue, 1.0, brightness);
                    fill_block(strip, col_start, block_width, color);
                }
                1 => {
                    // ---- HORIZONTAL CHASE ----
                    let cs = chase_speed as usize;
                    render_chase(
                        strip,
                        frame,
                        col_start,
                        block_width,
                        block_leds,
                        slot,
                        cs,
                        &chase_color,
                        false,
                    );
                }
                2 => {
                    // ---- SPARKLE ----
                    let hihat_bg = hihat_f * 0.08;
                    let bg = hsv_to_rgb(
                        s.chord_hue,
                        0.6,
                        (bass_f * 0.12).max(hihat_bg).max(energy * 0.05),
                    );
                    fill_block(strip, col_start, block_width, bg);
                    for spark in s.sparks.iter() {
                        let spark_col = (spark.col_frac * block_width as f32) as usize;
                        let spark_col = spark_col.min(block_width - 1);
                        let t = spark.life / spark.max_life;
                        let brightness = if t < 0.15 {
                            t / 0.15
                        } else {
                            (1.0 - (t - 0.15) / 0.85).max(0.0)
                        };
                        let brightness = brightness * brightness;
                        let color = hsv_to_rgb(spark.hue, 0.7, brightness);
                        let led = spark.row * COLS + col_start + spark_col;
                        strip.set(led, color);
                    }
                }
                3 => {
                    // ---- VERTICAL CHASE ----
                    let cs = chase_speed as usize;
                    render_chase(
                        strip,
                        frame,
                        col_start,
                        block_width,
                        block_leds,
                        slot,
                        cs,
                        &chase_color,
                        true,
                    );
                }
                4 => {
                    // ---- ENERGY BAR ----
                    let bar_energy = s.energy_smooth.max(vocals_f * 0.5).max(energy * 0.2);
                    let half = block_width / 2;
                    let bar_half = (bar_energy * half as f32) as usize;
                    let hue = if vocals_f > 0.2 {
                        (s.chord_hue + 30.0) % 360.0
                    } else {
                        (150.0 + mid_f * 60.0 + s.palette_offset) % 360.0
                    };

                    for row in 0..ROWS {
                        for col_off in 0..block_width {
                            let led = row * COLS + col_start + col_off;
                            let dist = if col_off >= half {
                                col_off - half
                            } else {
                                half - 1 - col_off
                            };
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
                    // ---- WAVE (unique to harmonic — standing wave tied to chord) ----
                    // Wavelength is determined by the chord root interval pattern.
                    let interval_sum: u16 = if s.chord_count >= 2 {
                        let mut sum = 0u16;
                        for i in 1..PROGRESSION_LEN {
                            if s.chord_history[i] < 12 && s.chord_history[i - 1] < 12 {
                                let diff = (s.chord_history[i] as i16
                                    - s.chord_history[i - 1] as i16)
                                    .unsigned_abs();
                                sum += diff;
                            }
                        }
                        sum
                    } else {
                        7
                    };
                    let wavelength = 4.0 + (interval_sum as f32 % 12.0) * 2.0;
                    let speed = 0.06 * s.speed_mult;

                    for row in 0..ROWS {
                        for col_off in 0..block_width {
                            let x = col_off as f32;
                            let y = row as f32;
                            let wave1 = ((x / wavelength + frame as f32 * speed).sin()
                                * (y / 3.0 + frame as f32 * speed * 0.7).cos())
                                * 0.5
                                + 0.5;
                            let wave2 =
                                ((x / (wavelength * 0.7) - frame as f32 * speed * 0.5).cos()) * 0.5
                                    + 0.5;
                            let val = ((wave1 * 0.6 + wave2 * 0.4) * energy).max(energy * 0.08);
                            let hue = (s.chord_hue + x * 2.0 + y * 15.0) % 360.0;
                            let led = row * COLS + col_start + col_off;
                            add_color(strip, led, hsv_to_rgb(hue, 0.9, val));
                        }
                    }
                }
                _ => {}
            }
        }

        // ============================================
        // KICK FLASH (whole strip brightness burst, subtle)
        // ============================================
        if kick_onset && s.kick_cooldown == 0 {
            s.kick_cooldown = 6;
            let flash_color = hsv_to_rgb((s.chord_hue + 180.0) % 360.0, 0.3, kick_f * 0.15);
            for i in 0..(COLS * ROWS) {
                add_color(strip, i, flash_color);
            }
        }

        // ---- Age / cull ----
        s.sparks.iter_mut().for_each(|sp| sp.life += 1.0);
        s.sparks.retain(|sp| sp.life < sp.max_life);
        s.kick_cooldown = s.kick_cooldown.saturating_sub(1);
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
    strip: &mut LedStrip,
    frame: usize,
    col_start: usize,
    block_width: usize,
    block_leds: usize,
    slot: usize,
    chase_speed: usize,
    chase_color: &[u8; 4],
    vertical: bool,
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
                add_color(
                    strip,
                    led_idx,
                    [
                        (chase_color[0] as f32 * fade) as u8,
                        (chase_color[1] as f32 * fade) as u8,
                        (chase_color[2] as f32 * fade) as u8,
                        0,
                    ],
                );
            }
        }
    }
}

fn lerp_hue(a: f32, b: f32, t: f32) -> f32 {
    let mut diff = b - a;
    if diff > 180.0 {
        diff -= 360.0;
    } else if diff < -180.0 {
        diff += 360.0;
    }
    (a + diff * t).rem_euclid(360.0)
}

fn add_color(strip: &mut LedStrip, idx: usize, color: [u8; 4]) {
    let leds = strip.controller_leds();
    let existing = leds[idx];
    strip.set(
        idx,
        [
            existing[0].max(color[0]),
            existing[1].max(color[1]),
            existing[2].max(color[2]),
            0,
        ],
    );
}
