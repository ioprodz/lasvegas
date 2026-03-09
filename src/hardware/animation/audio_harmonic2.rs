use crate::command::AudioAnalysis;
use crate::hardware::led::{hsv_to_rgb, LedStrip};
use std::cell::RefCell;

const COLS: usize = 60;
const ROWS: usize = 6;
const NUM_LEDS: usize = COLS * ROWS;
const NOTE_HUES: [f32; 12] = [
    0.0, 30.0, 55.0, 80.0, 120.0, 160.0, 195.0, 220.0, 260.0, 285.0, 320.0, 345.0,
];
const PROGRESSION_LEN: usize = 4;
const MAX_FIREFLIES: usize = 30;
const MAX_TRAVELERS: usize = 10;

// ── Particles ────────────────────────────────────────────────

struct Firefly {
    x: f32,
    y: f32,
    dx: f32,    // horizontal drift speed
    phase: f32, // vertical sine phase
    hue: f32,
    life: f32,
    max_life: f32,
}

/// Note transition: a full-height sine wave traveling horizontally.
struct WaveTraveler {
    x: f32,
    dx: f32, // speed (signed = direction)
    hue: f32,
    life: f32, // 0..1
    freq: f32, // vertical sine frequency
}

// ── Drop for rain animation ──

struct RainDrop {
    col_frac: f32, // 0..1 within block
    y: f32,        // current row (fractional)
    speed: f32,    // rows per frame
    hue: f32,
    brightness: f32,
}

// ── State ────────────────────────────────────────────────────

struct Hm2State {
    frame: usize,
    // Chord progression (same as V1)
    chord_history: [u8; PROGRESSION_LEN],
    chord_count: usize,
    prev_chord_root: u8,
    chord_stable_frames: usize,
    // Fingerprint & config
    fingerprint: u64,
    active_count: usize,
    block_types: [usize; 6],
    mirrored: [bool; 6], // whether each block renders mirrored
    palette_offset: f32,
    speed_mult: f32,
    // Audio smoothing
    chord_hue: f32,
    chord_hue_target: f32,
    energy: f32,
    bass_smooth: f32,
    mid_smooth: f32,
    treble_smooth: f32,
    beat_brightness: f32,
    // Note tracking
    prev_note_midi: u8,
    travelers: Vec<WaveTraveler>,
    // Per-block particles
    fireflies: Vec<Firefly>,
    rain_drops: Vec<RainDrop>,
    // Onset
    prev_kick: u8,
    kick_flash: f32,
    // RNG
    seed: u32,
}

impl Hm2State {
    fn rand(&mut self) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed
    }
    fn randf(&mut self) -> f32 {
        (self.rand() % 10000) as f32 / 10000.0
    }

    fn compute_fingerprint(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for i in 0..PROGRESSION_LEN {
            h ^= self.chord_history[i] as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }

    fn apply_fingerprint(&mut self) {
        let fp = self.fingerprint;

        // Block count: 2-5 (even numbers look more symmetric on 60 cols)
        let raw = (fp & 0xFF) as usize % 4; // 0,1,2,3
        self.active_count = [2, 3, 4, 5][raw]; // try odd too for variety

        // Block types: 6 types
        for i in 0..6 {
            let shift = 8 + i * 4;
            self.block_types[i] = ((fp >> shift) & 0x0F) as usize % 6;
        }

        // Mirror pattern: symmetric pairs.
        // For N blocks, mirror so left and right halves complement.
        let n = self.active_count;
        for i in 0..6 {
            self.mirrored[i] = i >= (n + 1) / 2; // right half is mirrored
        }
        // Also mirror block types for symmetry: slot[n-1-i] = slot[i]
        for i in 0..n / 2 {
            self.block_types[n - 1 - i] = self.block_types[i];
        }

        // Palette offset
        self.palette_offset = ((fp >> 32) & 0xFF) as f32 / 255.0 * 330.0;

        // Speed
        self.speed_mult = 0.7 + ((fp >> 40) & 0xFF) as f32 / 255.0 * 1.0;
    }
}

thread_local! {
    static STATE: RefCell<Hm2State> = RefCell::new(Hm2State {
        frame: 0,
        chord_history: [255; PROGRESSION_LEN],
        chord_count: 0,
        prev_chord_root: 255,
        chord_stable_frames: 0,
        fingerprint: 0,
        active_count: 3,
        block_types: [0, 1, 2, 3, 4, 5],
        mirrored: [false; 6],
        palette_offset: 0.0,
        speed_mult: 1.0,
        chord_hue: 0.0,
        chord_hue_target: 0.0,
        energy: 0.0,
        bass_smooth: 0.0,
        mid_smooth: 0.0,
        treble_smooth: 0.0,
        beat_brightness: 0.0,
        prev_note_midi: 0,
        travelers: Vec::new(),
        fireflies: Vec::new(),
        rain_drops: Vec::new(),
        prev_kick: 0,
        kick_flash: 0.0,
        seed: 88888,
    });
}

/// Harmonic Memory V2 — symmetric, visually cohesive block animations
/// designed for the 6×60 grid. Same chord progression = same layout.
/// Block types mirror across the strip for visual balance.
pub fn audio_harmonic2(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();
        s.frame = s.frame.wrapping_add(1);
        let frame = s.frame;
        let t = frame as f32;

        // ── Unpack audio ──
        let _kick_f = a.kick as f32 / 255.0;
        let hihat_f = a.hihat as f32 / 255.0;
        let vocals_f = a.vocals as f32 / 255.0;
        let beat_phase = a.beat_phase as f32 / 255.0;
        let bass_f = ((a.bands[0] as u16 + a.bands[1] as u16) / 2) as f32 / 255.0;
        let mid_f = ((a.bands[3] as u16 + a.bands[4] as u16) / 2) as f32 / 255.0;
        let high_f =
            ((a.bands[5] as u16 + a.bands[6] as u16 + a.bands[7] as u16) / 3) as f32 / 255.0;

        // ── Smooth audio ──
        smooth(
            &mut s.energy,
            a.bands.iter().map(|&b| b as f32).sum::<f32>() / (255.0 * 8.0),
            0.3,
            0.05,
        );
        smooth(&mut s.bass_smooth, bass_f, 0.4, 0.08);
        smooth(&mut s.mid_smooth, mid_f, 0.35, 0.08);
        smooth(&mut s.treble_smooth, high_f, 0.3, 0.1);
        let energy = s.energy.max(0.3);

        // ── Chord hue ──
        if a.chord_root < 12 {
            s.chord_hue_target = (NOTE_HUES[a.chord_root as usize] + s.palette_offset) % 360.0;
        }
        s.chord_hue = lerp_hue(s.chord_hue, s.chord_hue_target, 0.08);

        // ── Beat ──
        let beat_pulse = ((1.0 - beat_phase) * std::f32::consts::TAU).cos() * 0.5 + 0.5;
        smooth(&mut s.beat_brightness, beat_pulse, 0.4, 0.15);

        // ── Kick flash ──
        let kick_onset = a.kick > s.prev_kick.saturating_add(25);
        s.prev_kick = a.kick;
        if kick_onset {
            s.kick_flash = 1.0;
        }
        s.kick_flash *= 0.85;

        // ── Chord progression tracking ──
        s.chord_stable_frames += 1;
        if a.chord_root < 12 && a.chord_root != s.prev_chord_root && s.chord_stable_frames > 8 {
            s.chord_stable_frames = 0;
            s.prev_chord_root = a.chord_root;
            for i in 0..(PROGRESSION_LEN - 1) {
                s.chord_history[i] = s.chord_history[i + 1];
            }
            s.chord_history[PROGRESSION_LEN - 1] = a.chord_root;
            s.chord_count = (s.chord_count + 1).min(PROGRESSION_LEN);
            if s.chord_count >= 2 {
                let new_fp = s.compute_fingerprint();
                if new_fp != s.fingerprint {
                    s.fingerprint = new_fp;
                    s.apply_fingerprint();
                }
            }
        }

        // ── Note wave travelers ──
        if a.note_midi > 0 && a.note_midi != s.prev_note_midi && s.prev_note_midi > 0 {
            let from_x =
                ((s.prev_note_midi as f32 - 36.0) / 48.0).clamp(0.0, 1.0) * (COLS - 1) as f32;
            let to_x = ((a.note_midi as f32 - 36.0) / 48.0).clamp(0.0, 1.0) * (COLS - 1) as f32;
            let interval =
                ((a.note_midi as i16 - s.prev_note_midi as i16).unsigned_abs() % 12) as usize;
            let dir = if to_x > from_x { 1.0 } else { -1.0 };
            let speed = ((to_x - from_x).abs() / 25.0).max(0.4) * s.speed_mult * dir;
            // Vertical sine frequency from interval: larger interval = tighter wave
            let freq = 0.8 + interval as f32 * 0.3;

            if s.travelers.len() < MAX_TRAVELERS {
                s.travelers.push(WaveTraveler {
                    x: from_x,
                    dx: speed,
                    hue: NOTE_HUES[interval],
                    life: 1.0,
                    freq,
                });
            }
            // Spawn mirror traveler from opposite end
            if s.travelers.len() < MAX_TRAVELERS {
                s.travelers.push(WaveTraveler {
                    x: (COLS - 1) as f32 - from_x,
                    dx: -speed,
                    hue: (NOTE_HUES[interval] + 180.0) % 360.0,
                    life: 0.8,
                    freq,
                });
            }
        }
        if a.note_midi > 0 {
            s.prev_note_midi = a.note_midi;
        }

        for tr in s.travelers.iter_mut() {
            tr.x += tr.dx;
            tr.life -= 0.018;
        }
        s.travelers
            .retain(|tr| tr.life > 0.0 && tr.x > -5.0 && tr.x < (COLS + 5) as f32);

        // ── Spawn fireflies ──
        if s.fireflies.len() < MAX_FIREFLIES && s.randf() < (energy * 0.3 + hihat_f * 0.2) {
            let r1 = s.randf();
            let r2 = s.randf();
            let r3 = s.randf();
            let r4 = s.randf();
            let r5 = s.randf();
            let r6 = s.randf();
            let hue = (s.chord_hue + r1 * 40.0 - 20.0).rem_euclid(360.0);
            s.fireflies.push(Firefly {
                x: r2 * COLS as f32,
                y: r3 * ROWS as f32,
                dx: (r4 - 0.5) * 0.4 * s.speed_mult,
                phase: r5 * std::f32::consts::TAU,
                hue,
                life: 0.0,
                max_life: 30.0 + r6 * 40.0,
            });
        }

        // ── Spawn rain drops ──
        let rain_rate = high_f * 0.5 + s.treble_smooth * 0.3;
        if s.randf() < rain_rate && s.rain_drops.len() < 60 {
            let r1 = s.randf();
            let r2 = s.randf();
            let r3 = s.randf();
            let r4 = s.randf();
            let hue = (s.chord_hue + 60.0 + r1 * 30.0).rem_euclid(360.0);
            s.rain_drops.push(RainDrop {
                col_frac: r2,
                y: -0.5,
                speed: 0.08 + r3 * 0.15,
                hue,
                brightness: 0.5 + r4 * 0.5,
            });
        }

        // ══════════════════════════════════════════════
        // RENDER
        // ══════════════════════════════════════════════
        strip.set_all([0, 0, 0, 0]);

        // ── Global: subtle beat breathing on all LEDs ──
        {
            let breath = s.beat_brightness * energy * 0.15;
            if breath > 0.01 {
                let color = hsv_to_rgb(s.chord_hue, 0.3, breath);
                for i in 0..NUM_LEDS {
                    add_color(strip, i, color);
                }
            }
        }

        // ── Note wave travelers (full-height sine waves) ──
        for tr in s.travelers.iter() {
            let alpha = tr.life * tr.life;
            if alpha < 0.02 {
                continue;
            }
            let half_width = 3.0;
            let x_start = (tr.x - half_width).max(0.0) as usize;
            let x_end = ((tr.x + half_width).min((COLS - 1) as f32) as usize) + 1;

            for col in x_start..x_end {
                let dx = col as f32 - tr.x;
                let x_fade = 1.0 - (dx.abs() / half_width);
                if x_fade <= 0.0 {
                    continue;
                }

                for row in 0..ROWS {
                    let wave = ((row as f32 * tr.freq + t * 0.1).sin() * 0.5 + 0.5) * x_fade;
                    let b = wave * alpha;
                    if b < 0.02 {
                        continue;
                    }
                    let idx = row * COLS + col;
                    add_color(strip, idx, hsv_to_rgb(tr.hue, 0.8, b));
                }
            }
        }

        // ── Render blocks ──
        let active_count = s.active_count;
        let block_width = COLS / active_count;
        let block_cx = block_width as f32 / 2.0;
        let row_cy = (ROWS - 1) as f32 / 2.0;

        for slot in 0..active_count {
            let col_start = slot * block_width;
            let block_type = s.block_types[slot % 6];
            let mirror = s.mirrored[slot];
            let slot_phase = slot as f32 * 1.2;

            match block_type {
                0 => {
                    // ── AURORA CURTAIN ──
                    // Each row is a sine wave with different phase, creating a
                    // flowing curtain. Adjacent rows are offset for depth.
                    let speed = 0.04 * s.speed_mult;
                    let wave_stretch = 6.0 + s.bass_smooth * 8.0; // wider waves with bass

                    for row in 0..ROWS {
                        let row_phase = row as f32 * 0.7 + slot_phase;
                        for col_off in 0..block_width {
                            let x = if mirror {
                                block_width - 1 - col_off
                            } else {
                                col_off
                            };
                            let wave1 =
                                (x as f32 / wave_stretch + t * speed + row_phase).sin() * 0.5 + 0.5;
                            let wave2 = (x as f32 / (wave_stretch * 1.6) - t * speed * 0.6
                                + row_phase * 0.5)
                                .sin()
                                * 0.5
                                + 0.5;
                            let combined = wave1 * 0.6 + wave2 * 0.4;
                            let val = (combined * (0.3 + energy * 1.5)).max(0.15);
                            // Hue shifts per row for depth
                            let hue = (s.chord_hue + row as f32 * 20.0 + x as f32 * 0.8) % 360.0;
                            let led = row * COLS + col_start + col_off;
                            add_color(strip, led, hsv_to_rgb(hue, 0.85, val));
                        }
                    }
                }
                1 => {
                    // ── DIAMOND BREATHE ──
                    // Concentric diamonds expanding from block center, pulsing with beat.
                    // Horizontally stretched to match aspect ratio.
                    let phase_offset = slot_phase * 2.0;
                    let expand = (t * 0.07 * s.speed_mult + phase_offset).sin() * 0.5 + 0.5;
                    let max_diamond = block_cx + 2.0;

                    for row in 0..ROWS {
                        for col_off in 0..block_width {
                            let x = if mirror {
                                block_width - 1 - col_off
                            } else {
                                col_off
                            };
                            // Diamond distance: |dx| + |dy| scaled for aspect ratio
                            let dx = (x as f32 - block_cx).abs();
                            let dy = (row as f32 - row_cy).abs() * (block_cx / row_cy).min(4.0);
                            let diamond_dist = dx + dy;

                            // Multiple concentric diamonds
                            let ring_size = max_diamond * (0.3 + expand * 0.7);
                            let ring1 = ((diamond_dist - ring_size).abs() / 2.5).min(1.0);
                            let ring2 = ((diamond_dist - ring_size * 0.5).abs() / 2.0).min(1.0);
                            let val = ((1.0 - ring1) * 0.7 + (1.0 - ring2) * 0.3)
                                * (0.4 + energy * 0.8)
                                * (0.6 + s.beat_brightness * 0.4);
                            if val < 0.02 {
                                continue;
                            }
                            let hue =
                                (s.chord_hue + diamond_dist * 4.0 + slot_phase * 30.0) % 360.0;
                            let led = row * COLS + col_start + col_off;
                            add_color(strip, led, hsv_to_rgb(hue, 0.9, val));
                        }
                    }
                }
                2 => {
                    // ── RAIN ──
                    // Vertical drops falling through rows. High freq controls density.
                    // Each drop has a soft glow that spans ±1 column.
                    for drop in s.rain_drops.iter() {
                        let drop_col = (drop.col_frac * block_width as f32) as isize;
                        let drop_row = drop.y;
                        if drop_row < -1.0 || drop_row >= ROWS as f32 + 1.0 {
                            continue;
                        }

                        for dr in -1i32..=2 {
                            let row = drop_row as i32 + dr;
                            if row < 0 || row >= ROWS as i32 {
                                continue;
                            }
                            let y_dist = (row as f32 - drop_row).abs();
                            let y_fade = (1.0 - y_dist * 0.6).max(0.0);

                            for dc in -1i32..=1 {
                                let col = drop_col + dc as isize;
                                if col < 0 || col >= block_width as isize {
                                    continue;
                                }
                                let x_fade = if dc == 0 { 1.0 } else { 0.25 };
                                let b = y_fade * x_fade * drop.brightness * (0.5 + energy);
                                if b < 0.02 {
                                    continue;
                                }
                                let led = row as usize * COLS + col_start + col as usize;
                                add_color(strip, led, hsv_to_rgb(drop.hue, 0.7, b));
                            }
                        }
                    }
                }
                3 => {
                    // ── GRADIENT SWEEP ──
                    // A smooth hue gradient sweeps horizontally, direction oscillates.
                    // Creates flowing color washes. Mid-freq controls saturation.
                    let sweep_pos = (t * 0.05 * s.speed_mult + slot_phase).sin();
                    let sat = 0.6 + s.mid_smooth * 0.4;

                    for row in 0..ROWS {
                        for col_off in 0..block_width {
                            let x = if mirror {
                                block_width - 1 - col_off
                            } else {
                                col_off
                            };
                            let x_norm = x as f32 / block_width as f32;
                            // Gradient position shifts with sweep
                            let grad_pos =
                                (x_norm + sweep_pos * 0.5 + row as f32 * 0.08).rem_euclid(1.0);
                            let hue = (s.chord_hue + grad_pos * 120.0) % 360.0;
                            // Brightness: center of gradient is brighter
                            let center_dist = (grad_pos - 0.5).abs() * 2.0;
                            let val = ((1.0 - center_dist * 0.5) * (0.3 + energy * 1.2)).max(0.15);
                            let led = row * COLS + col_start + col_off;
                            add_color(strip, led, hsv_to_rgb(hue, sat, val));
                        }
                    }
                }
                4 => {
                    // ── MIRROR BARS ──
                    // Two energy bars growing from edges toward center.
                    // Bass drives left bar, vocals drive right bar.
                    // Where they overlap, brightness doubles.
                    let left_energy = s.bass_smooth.max(energy * 0.4).max(0.25);
                    let right_energy = (vocals_f * 0.6 + s.mid_smooth * 0.4)
                        .max(energy * 0.4)
                        .max(0.25);
                    let half = block_width / 2;
                    let left_extent = (left_energy * half as f32) as usize;
                    let right_extent = (right_energy * half as f32) as usize;
                    let hue_l = s.chord_hue;
                    let hue_r = (s.chord_hue + 60.0) % 360.0;

                    for row in 0..ROWS {
                        for col_off in 0..block_width {
                            let x = if mirror {
                                block_width - 1 - col_off
                            } else {
                                col_off
                            };
                            let led = row * COLS + col_start + col_off;

                            // Left bar: grows from col 0
                            if x < left_extent {
                                let fade = 1.0 - x as f32 / left_extent.max(1) as f32;
                                let b = fade * fade * (0.4 + energy);
                                add_color(strip, led, hsv_to_rgb(hue_l, 0.9, b));
                            }
                            // Right bar: grows from col block_width-1
                            let from_right = block_width - 1 - x;
                            if from_right < right_extent {
                                let fade = 1.0 - from_right as f32 / right_extent.max(1) as f32;
                                let b = fade * fade * (0.4 + energy);
                                add_color(strip, led, hsv_to_rgb(hue_r, 0.9, b));
                            }
                        }
                    }
                }
                5 => {
                    // ── FIREFLY DRIFT ──
                    // Slow organic glowing dots that drift horizontally with
                    // gentle sine-wave vertical wobble. Each firefly has its own hue.
                    for ff in s.fireflies.iter() {
                        // Map firefly global position to block-local
                        let local_x = ff.x - col_start as f32;
                        if local_x < -2.0 || local_x >= (block_width + 2) as f32 {
                            continue;
                        }

                        let progress = ff.life / ff.max_life;
                        let brightness = if progress < 0.2 {
                            progress / 0.2
                        } else if progress > 0.7 {
                            (1.0 - progress) / 0.3
                        } else {
                            1.0
                        };
                        let brightness = brightness * brightness * (0.5 + energy * 1.5);
                        if brightness < 0.02 {
                            continue;
                        }

                        // Glow radius
                        let radius = 1.8;
                        let cx = local_x.round() as isize;
                        let cy = ff.y.round() as isize;
                        for dy in -2isize..=2 {
                            let row = cy + dy;
                            if row < 0 || row >= ROWS as isize {
                                continue;
                            }
                            for dx in -2isize..=2 {
                                let col = cx + dx;
                                if col < 0 || col >= block_width as isize {
                                    continue;
                                }
                                let dist_x = col as f32 - local_x;
                                let dist_y = (row as f32 - ff.y) * 2.0; // stretch for aspect
                                let dist = (dist_x * dist_x + dist_y * dist_y).sqrt();
                                if dist > radius {
                                    continue;
                                }
                                let falloff = 1.0 - dist / radius;
                                let b = brightness * falloff * falloff;
                                if b < 0.01 {
                                    continue;
                                }
                                let led = row as usize * COLS + col_start + col as usize;
                                add_color(strip, led, hsv_to_rgb(ff.hue, 0.75, b));
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // ── Kick flash overlay ──
        if s.kick_flash > 0.05 {
            let flash_b = s.kick_flash * 0.3;
            let color = hsv_to_rgb((s.chord_hue + 180.0) % 360.0, 0.2, flash_b);
            for i in 0..NUM_LEDS {
                add_color(strip, i, color);
            }
        }

        // ── Age / cull particles ──
        for ff in s.fireflies.iter_mut() {
            ff.life += 1.0;
            ff.x += ff.dx;
            ff.y += (ff.phase + t * 0.03).sin() * 0.06;
            ff.y = ff.y.clamp(0.0, (ROWS - 1) as f32);
        }
        s.fireflies
            .retain(|ff| ff.life < ff.max_life && ff.x > -3.0 && ff.x < (COLS + 3) as f32);

        for drop in s.rain_drops.iter_mut() {
            drop.y += drop.speed;
        }
        s.rain_drops.retain(|d| d.y < ROWS as f32 + 1.0);
    });
}

fn smooth(current: &mut f32, target: f32, attack: f32, decay: f32) {
    if target > *current {
        *current += (target - *current) * attack;
    } else {
        *current += (target - *current) * decay;
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
