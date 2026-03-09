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
const MAX_ORBITERS: usize = 36;
const MAX_BUBBLES: usize = 30;
const MAX_TRAVELERS: usize = 10;

// Pastel: high value, low-mid saturation
const PASTEL_SAT: f32 = 0.35;
const PASTEL_SAT_HI: f32 = 0.50;

// ── Particles ────────────────────────────────────────────────

/// A dot that orbits around the perimeter of a block
struct Orbiter {
    slot: usize, // which block this belongs to
    angle: f32,  // current angle on perimeter (radians)
    speed: f32,  // radians per frame
    hue: f32,
    size: f32, // glow radius
    life: f32,
    max_life: f32,
}

/// Soft bubble that floats upward with gentle wobble
struct Bubble {
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
    radius: f32,
    hue: f32,
    life: f32,
    max_life: f32,
    wobble_phase: f32,
}

/// Note transition wave traveler
struct WaveTraveler {
    x: f32,
    dx: f32,
    hue: f32,
    life: f32,
    freq: f32,
}

// ── State ────────────────────────────────────────────────────

struct Hm6State {
    frame: usize,
    chord_history: [u8; PROGRESSION_LEN],
    chord_count: usize,
    prev_chord_root: u8,
    chord_stable_frames: usize,
    fingerprint: u64,
    active_count: usize,
    block_types: [usize; 6],
    mirrored: [bool; 6],
    block_reversed: [bool; 6],
    palette_offset: f32,
    speed_mult: f32,
    chord_hue: f32,
    chord_hue_target: f32,
    energy: f32,
    bass_smooth: f32,
    mid_smooth: f32,
    treble_smooth: f32,
    beat_brightness: f32,
    melody_speed: f32,
    note_change_acc: f32,
    prev_note_midi: u8,
    travelers: Vec<WaveTraveler>,
    orbiters: Vec<Orbiter>,
    bubbles: Vec<Bubble>,
    prev_kick: u8,
    kick_flash: f32,
    bpm_smooth: f32,
    seed: u32,
}

impl Hm6State {
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

        let raw = (fp & 0xFF) as usize % 4;
        self.active_count = [2, 3, 4, 5][raw];

        for i in 0..6 {
            let shift = 8 + i * 4;
            self.block_types[i] = ((fp >> shift) & 0x0F) as usize % 6;
        }

        let n = self.active_count;
        for i in 0..6 {
            self.mirrored[i] = i >= (n + 1) / 2;
        }
        for i in 0..n / 2 {
            self.block_types[n - 1 - i] = self.block_types[i];
        }

        for i in 0..6 {
            self.block_reversed[i] = ((fp >> (48 + i)) & 1) == 1;
        }
        for i in 0..n / 2 {
            self.block_reversed[n - 1 - i] = !self.block_reversed[i];
        }

        self.palette_offset = ((fp >> 32) & 0xFF) as f32 / 255.0 * 330.0;
        self.speed_mult = 0.7 + ((fp >> 40) & 0xFF) as f32 / 255.0 * 1.0;
    }
}

thread_local! {
    static STATE: RefCell<Hm6State> = RefCell::new(Hm6State {
        frame: 0,
        chord_history: [255; PROGRESSION_LEN],
        chord_count: 0,
        prev_chord_root: 255,
        chord_stable_frames: 0,
        fingerprint: 0,
        active_count: 3,
        block_types: [0, 1, 2, 3, 4, 5],
        mirrored: [false; 6],
        block_reversed: [false, true, false, true, false, true],
        palette_offset: 0.0,
        speed_mult: 1.0,
        chord_hue: 0.0,
        chord_hue_target: 0.0,
        energy: 0.0,
        bass_smooth: 0.0,
        mid_smooth: 0.0,
        treble_smooth: 0.0,
        beat_brightness: 0.0,
        melody_speed: 0.5,
        note_change_acc: 0.0,
        prev_note_midi: 0,
        travelers: Vec::new(),
        orbiters: Vec::new(),
        bubbles: Vec::new(),
        prev_kick: 0,
        kick_flash: 0.0,
        bpm_smooth: 120.0,
        seed: 66666,
    });
}

/// Harmonic Memory V6 — Pastel Dreams.
/// Same chord-fingerprint sync. All pastel palette. Six block types:
/// orbiting particles, harmonic waves, bubble float, ribbon weave,
/// petal bloom, cloud drift.
pub fn audio_harmonic6(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();
        s.frame = s.frame.wrapping_add(1);
        let frame = s.frame;
        let t = frame as f32;

        // ── Unpack audio ──
        let hihat_f = a.hihat as f32 / 255.0;
        let beat_phase = a.beat_phase as f32 / 255.0;
        let bass_f = ((a.bands[0] as u16 + a.bands[1] as u16) / 2) as f32 / 255.0;
        let mid_f = ((a.bands[3] as u16 + a.bands[4] as u16) / 2) as f32 / 255.0;
        let high_f =
            ((a.bands[5] as u16 + a.bands[6] as u16 + a.bands[7] as u16) / 3) as f32 / 255.0;
        let bpm = a.bpm as f32;

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

        // ── BPM smoothing ──
        if bpm > 30.0 && bpm < 250.0 {
            smooth(&mut s.bpm_smooth, bpm, 0.1, 0.02);
        }
        let bpm_factor = (s.bpm_smooth / 120.0).clamp(0.5, 2.0);

        // ── Melody speed ──
        s.note_change_acc *= 0.97;
        smooth(
            &mut s.melody_speed,
            0.4 + s.note_change_acc * 2.0,
            0.2,
            0.05,
        );
        let mspeed = s.melody_speed.clamp(0.3, 2.5);

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

        // ── Note tracking ──
        if a.note_midi > 0 && a.note_midi != s.prev_note_midi && s.prev_note_midi > 0 {
            let interval = (a.note_midi as i16 - s.prev_note_midi as i16).unsigned_abs();
            s.note_change_acc = (s.note_change_acc + 0.15 + interval as f32 * 0.02).min(1.5);

            let from_x =
                ((s.prev_note_midi as f32 - 36.0) / 48.0).clamp(0.0, 1.0) * (COLS - 1) as f32;
            let to_x = ((a.note_midi as f32 - 36.0) / 48.0).clamp(0.0, 1.0) * (COLS - 1) as f32;
            let dir = if to_x > from_x { 1.0 } else { -1.0 };
            let speed = ((to_x - from_x).abs() / 25.0).max(0.4) * s.speed_mult * dir;
            let freq = 0.8 + (interval % 12) as f32 * 0.3;
            let hue = NOTE_HUES[(interval % 12) as usize];

            if s.travelers.len() < MAX_TRAVELERS {
                s.travelers.push(WaveTraveler {
                    x: from_x,
                    dx: speed,
                    hue,
                    life: 1.0,
                    freq,
                });
            }
            if s.travelers.len() < MAX_TRAVELERS {
                s.travelers.push(WaveTraveler {
                    x: (COLS - 1) as f32 - from_x,
                    dx: -speed,
                    hue: (hue + 180.0) % 360.0,
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

        // ── Spawn orbiters ──
        let active_count = s.active_count;
        let block_width = COLS / active_count;
        if s.orbiters.len() < MAX_ORBITERS && s.randf() < (energy * 0.25 + hihat_f * 0.2) {
            let r1 = s.randf();
            let r2 = s.randf();
            let r3 = s.randf();
            let r4 = s.randf();
            let slot = (r1 * active_count as f32) as usize % active_count;
            let reversed = s.block_reversed[slot % 6];
            let base_speed = 0.03 + r2 * 0.04;
            s.orbiters.push(Orbiter {
                slot,
                angle: r3 * std::f32::consts::TAU,
                speed: base_speed * mspeed * if reversed { -1.0 } else { 1.0 },
                hue: (s.chord_hue + r4 * 50.0 - 25.0).rem_euclid(360.0),
                size: 1.2 + r1 * 0.8,
                life: 0.0,
                max_life: 50.0 + r2 * 70.0,
            });
        }

        // ── Spawn bubbles ──
        if s.bubbles.len() < MAX_BUBBLES && s.randf() < (s.treble_smooth * 0.3 + energy * 0.15) {
            let r1 = s.randf();
            let r2 = s.randf();
            let r3 = s.randf();
            let r4 = s.randf();
            let r5 = s.randf();
            let hue = (s.chord_hue + r1 * 60.0 - 30.0).rem_euclid(360.0);
            s.bubbles.push(Bubble {
                x: r2 * COLS as f32,
                y: ROWS as f32 + 0.5,
                dx: (r3 - 0.5) * 0.15,
                dy: -(0.04 + r4 * 0.06),
                radius: 1.0 + r5 * 1.2,
                hue,
                life: 0.0,
                max_life: 60.0 + r1 * 60.0,
                wobble_phase: r3 * std::f32::consts::TAU,
            });
        }

        // ══════════════════════════════════════════════
        // RENDER
        // ══════════════════════════════════════════════
        strip.set_all([0, 0, 0, 0]);

        // ── Global: pastel beat breathing ──
        {
            let breath = s.beat_brightness * energy * 0.12;
            if breath > 0.01 {
                let color = hsv_to_rgb(s.chord_hue, 0.2, breath);
                for i in 0..NUM_LEDS {
                    add_color(strip, i, color);
                }
            }
        }

        // ── Note wave travelers (pastel) ──
        for tr in s.travelers.iter() {
            let alpha = tr.life * tr.life;
            if alpha < 0.02 {
                continue;
            }
            let half_width = 3.5;
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
                    add_color(strip, idx, hsv_to_rgb(tr.hue, PASTEL_SAT, b));
                }
            }
        }

        // ── Render blocks ──
        let block_cx = block_width as f32 / 2.0;
        let row_cy = (ROWS - 1) as f32 / 2.0;

        for slot in 0..active_count {
            let col_start = slot * block_width;
            let block_type = s.block_types[slot % 6];
            let mirror = s.mirrored[slot];
            let reversed = s.block_reversed[slot % 6];
            let dir: f32 = if reversed { -1.0 } else { 1.0 };
            let slot_phase = slot as f32 * 1.5;

            match block_type {
                0 => {
                    // ── ORBITING PARTICLES ──
                    // Soft pastel dots circling around the block perimeter.
                    // Speed driven by melody; direction from fingerprint.
                    // Rendered via the orbiters list (filtered per-slot).
                    for orb in s.orbiters.iter() {
                        if orb.slot != slot {
                            continue;
                        }
                        let progress = orb.life / orb.max_life;
                        let alpha = if progress < 0.15 {
                            progress / 0.15
                        } else if progress > 0.7 {
                            (1.0 - progress) / 0.3
                        } else {
                            1.0
                        };
                        if alpha < 0.02 {
                            continue;
                        }

                        // Map angle to perimeter position
                        let (px, py) =
                            angle_to_block_perimeter(orb.angle, block_width as f32, ROWS as f32);
                        let gx = col_start as f32 + px;
                        let brightness = alpha * (0.5 + energy * 0.8);
                        render_glow(strip, gx, py, orb.size, brightness, orb.hue, PASTEL_SAT);

                        // Soft trail: 3 ghost positions behind
                        for ti in 1..=3 {
                            let trail_angle = orb.angle - orb.speed * ti as f32 * 4.0;
                            let (tx, ty) = angle_to_block_perimeter(
                                trail_angle,
                                block_width as f32,
                                ROWS as f32,
                            );
                            let tgx = col_start as f32 + tx;
                            let trail_b = brightness * (1.0 - ti as f32 * 0.3) * 0.5;
                            if trail_b > 0.02 {
                                render_glow(
                                    strip,
                                    tgx,
                                    ty,
                                    orb.size * 0.7,
                                    trail_b,
                                    orb.hue,
                                    PASTEL_SAT,
                                );
                            }
                        }
                    }

                    // Soft pastel background glow for this block
                    let bg_val = 0.06 + energy * 0.04;
                    for row in 0..ROWS {
                        for col_off in 0..block_width {
                            let led = row * COLS + col_start + col_off;
                            add_color(
                                strip,
                                led,
                                hsv_to_rgb(
                                    (s.chord_hue + slot_phase * 20.0).rem_euclid(360.0),
                                    0.2,
                                    bg_val,
                                ),
                            );
                        }
                    }
                }
                1 => {
                    // ── HARMONIC WAVES ──
                    // Overlapping sine waves. Wavelength from BPM (slow tempo =
                    // long waves). Wave angle/tilt from mid freq. Speed from
                    // melody. Direction reversed per block.
                    let wavelength = 3.0 + (bpm_factor * 6.0);
                    let tilt = s.mid_smooth * 0.5 * dir; // wave angle
                    let speed = 0.06 * mspeed * dir;

                    for row in 0..ROWS {
                        for col_off in 0..block_width {
                            let x = if mirror {
                                block_width - 1 - col_off
                            } else {
                                col_off
                            };
                            let nx = x as f32;
                            let ny = row as f32;

                            // Tilted coordinate
                            let tilted = nx + ny * tilt;

                            let wave1 = (tilted / wavelength + t * speed + slot_phase).sin();
                            let wave2 = (tilted / (wavelength * 1.7) - t * speed * 0.6
                                + ny * 0.3
                                + slot_phase * 0.5)
                                .sin();
                            let wave3 = (ny / 2.5 + t * speed * 0.4 + slot_phase * 1.3).sin() * 0.5;

                            let combined = (wave1 * 0.5 + wave2 * 0.3 + wave3 * 0.2) * 0.5 + 0.5;
                            let val = (combined * (0.3 + energy * 1.0)).max(0.10);

                            // Pastel hue shifts gently per row
                            let hue =
                                (s.chord_hue + row as f32 * 15.0 + nx * 0.5 + slot_phase * 20.0)
                                    .rem_euclid(360.0);
                            let led = row * COLS + col_start + col_off;
                            add_color(strip, led, hsv_to_rgb(hue, PASTEL_SAT_HI, val));
                        }
                    }
                }
                2 => {
                    // ── BUBBLE FLOAT ──
                    // Soft pastel bubbles rise through the block with gentle
                    // horizontal wobble. Treble spawns more; bass makes them bigger.
                    for bub in s.bubbles.iter() {
                        let local_x = bub.x - col_start as f32;
                        if local_x < -(bub.radius + 1.0)
                            || local_x >= block_width as f32 + bub.radius + 1.0
                        {
                            continue;
                        }

                        let progress = bub.life / bub.max_life;
                        let alpha = if progress < 0.1 {
                            progress / 0.1
                        } else if progress > 0.8 {
                            (1.0 - progress) / 0.2
                        } else {
                            1.0
                        };
                        if alpha < 0.02 {
                            continue;
                        }

                        let r = bub.radius * (1.0 + s.bass_smooth * 0.4);
                        let brightness = alpha * (0.4 + energy * 0.7);

                        // Bubble: bright ring, softer center (like a soap bubble)
                        let r_ceil = r.ceil() as isize + 1;
                        let cx = bub.x.round() as isize;
                        let cy = bub.y.round() as isize;
                        for dy in -r_ceil..=r_ceil {
                            let row = cy + dy;
                            if row < 0 || row >= ROWS as isize {
                                continue;
                            }
                            for dx in -r_ceil..=r_ceil {
                                let col = cx + dx;
                                if col < 0 || col >= COLS as isize {
                                    continue;
                                }
                                let dist_x = col as f32 - bub.x;
                                let dist_y = (row as f32 - bub.y) * 2.0;
                                let dist = (dist_x * dist_x + dist_y * dist_y).sqrt();
                                if dist > r {
                                    continue;
                                }
                                let norm_dist = dist / r;
                                // Ring brightness: brighter at edge
                                let ring = if norm_dist > 0.6 {
                                    (norm_dist - 0.6) / 0.4
                                } else {
                                    0.3
                                };
                                let b = brightness * ring;
                                if b < 0.01 {
                                    continue;
                                }
                                let led = row as usize * COLS + col as usize;
                                add_color(strip, led, hsv_to_rgb(bub.hue, PASTEL_SAT, b));
                            }
                        }
                    }
                }
                3 => {
                    // ── RIBBON WEAVE ──
                    // Thin horizontal ribbons that undulate vertically, each row
                    // a different pastel shade. BPM controls weave frequency;
                    // mid freq controls amplitude. Creates a fabric-like look.
                    let weave_freq = 2.0 + bpm_factor * 3.0;
                    let amplitude = 0.3 + s.mid_smooth * 0.7;
                    let speed = 0.04 * mspeed * dir;

                    for row in 0..ROWS {
                        let ribbon_hue =
                            (s.chord_hue + row as f32 * 55.0 + slot_phase * 25.0).rem_euclid(360.0);
                        // Each ribbon has a vertical offset that oscillates
                        let y_offset =
                            (t * speed + row as f32 * 1.2 + slot_phase).sin() * amplitude;

                        for col_off in 0..block_width {
                            let x = if mirror {
                                block_width - 1 - col_off
                            } else {
                                col_off
                            };
                            let nx = x as f32 / block_width as f32;

                            // Weave: sine modulation creates the undulation
                            let weave = (nx * weave_freq * std::f32::consts::TAU
                                + t * speed * 3.0
                                + row as f32 * 0.8)
                                .sin();
                            let ribbon_y = row as f32 + y_offset + weave * 0.4;

                            // How close is this pixel to the ribbon center?
                            let dist_to_ribbon = (row as f32 - ribbon_y).abs();
                            let ribbon_brightness = (1.0 - dist_to_ribbon * 1.5).max(0.0);

                            if ribbon_brightness < 0.02 {
                                continue;
                            }

                            // Cross-weave: alternate ribbons go over/under
                            let cross =
                                ((nx * weave_freq + row as f32 * 0.5).sin() * 0.5 + 0.5) * 0.3;
                            let val = (ribbon_brightness * (0.3 + energy * 0.9)
                                + cross * energy * 0.2)
                                .max(0.08);
                            let led = row * COLS + col_start + col_off;
                            add_color(strip, led, hsv_to_rgb(ribbon_hue, PASTEL_SAT_HI, val));
                        }
                    }
                }
                4 => {
                    // ── PETAL BLOOM ──
                    // Flower-like pattern: "petals" radiate from block center,
                    // rotating with beat. Number of petals from bass.
                    // Opens/closes with energy.
                    let petals = 3.0 + (s.bass_smooth * 3.0).floor();
                    let rotation = t * 0.04 * mspeed * dir + slot_phase;
                    let bloom = 0.3 + energy * 0.7; // how "open" the flower is

                    for row in 0..ROWS {
                        for col_off in 0..block_width {
                            let x = if mirror {
                                block_width - 1 - col_off
                            } else {
                                col_off
                            };
                            let nx = (x as f32 - block_cx) / block_cx;
                            let ny = (row as f32 - row_cy) / row_cy.max(1.0);
                            let ny_scaled = ny * (block_cx / row_cy.max(1.0)).min(3.0);

                            let dist = (nx * nx + ny_scaled * ny_scaled).sqrt();
                            let angle = ny_scaled.atan2(nx);

                            // Petal shape: cosine of angle * petal count
                            let petal = ((angle + rotation) * petals).cos();
                            let petal_shape = (petal * 0.5 + 0.5) * bloom;

                            // Inside the petal?
                            let petal_radius = petal_shape * (block_cx * 0.8).min(8.0);
                            let in_petal = (1.0 - dist / petal_radius.max(0.1)).max(0.0);

                            if in_petal < 0.02 && dist > 1.5 {
                                // Soft background
                                let bg = 0.06 * (1.0 - dist * 0.3).max(0.0);
                                if bg > 0.01 {
                                    let led = row * COLS + col_start + col_off;
                                    add_color(strip, led, hsv_to_rgb(s.chord_hue, 0.15, bg));
                                }
                                continue;
                            }

                            // Center glow
                            let center_glow = (1.0 - dist * 0.8).max(0.0);
                            let val = (in_petal * 0.7 + center_glow * 0.4)
                                * (0.4 + energy * 0.8)
                                * (0.7 + s.beat_brightness * 0.3);
                            if val < 0.02 {
                                continue;
                            }

                            // Each petal gets a slightly different hue
                            let petal_idx = ((angle + rotation) * petals / std::f32::consts::TAU)
                                .rem_euclid(petals);
                            let hue =
                                (s.chord_hue + petal_idx * (360.0 / petals) + slot_phase * 20.0)
                                    .rem_euclid(360.0);
                            let led = row * COLS + col_start + col_off;
                            add_color(strip, led, hsv_to_rgb(hue, PASTEL_SAT_HI, val));
                        }
                    }
                }
                5 => {
                    // ── CLOUD DRIFT ──
                    // Soft gaussian-ish blobs that drift slowly across the block,
                    // overlapping to create dreamy pastel color mixing.
                    // 4 implicit clouds per block.
                    let speed_f = 0.02 * mspeed;

                    for row in 0..ROWS {
                        for col_off in 0..block_width {
                            let x = if mirror {
                                block_width - 1 - col_off
                            } else {
                                col_off
                            };
                            let px = x as f32;
                            let py = row as f32;

                            let mut total_val = 0.0f32;
                            let mut total_hue_x = 0.0f32;
                            let mut total_hue_y = 0.0f32;

                            // 4 drifting clouds
                            for ci in 0..4 {
                                let ci_f = ci as f32;
                                let cloud_x = block_cx
                                    + (t * speed_f * dir + ci_f * 1.7 + slot_phase).sin()
                                        * block_cx
                                        * 0.7;
                                let cloud_y = row_cy
                                    + (t * speed_f * 0.7 * dir + ci_f * 2.3 + slot_phase + 1.0)
                                        .sin()
                                        * row_cy
                                        * 0.8;
                                let cloud_r = 3.0 + (ci_f * 0.5) + s.bass_smooth * 2.0;

                                let dx = px - cloud_x;
                                let dy = (py - cloud_y) * 2.5; // aspect
                                let dist_sq = dx * dx + dy * dy;
                                let r_sq = cloud_r * cloud_r;

                                if dist_sq < r_sq * 2.0 {
                                    // Gaussian-ish falloff
                                    let falloff = (-dist_sq / (r_sq * 0.8)).exp();
                                    let cloud_brightness = falloff * (0.25 + energy * 0.6);
                                    total_val += cloud_brightness;

                                    // Weighted hue accumulation (for blending)
                                    let cloud_hue = (s.chord_hue + ci_f * 70.0 + slot_phase * 15.0)
                                        .rem_euclid(360.0);
                                    let rad = cloud_hue.to_radians();
                                    total_hue_x += rad.cos() * cloud_brightness;
                                    total_hue_y += rad.sin() * cloud_brightness;
                                }
                            }

                            if total_val < 0.02 {
                                continue;
                            }
                            total_val = total_val.min(1.0);

                            // Recover blended hue from weighted average
                            let blended_hue = total_hue_y
                                .atan2(total_hue_x)
                                .to_degrees()
                                .rem_euclid(360.0);

                            let led = row * COLS + col_start + col_off;
                            add_color(strip, led, hsv_to_rgb(blended_hue, PASTEL_SAT, total_val));
                        }
                    }
                }
                _ => {}
            }
        }

        // ── Kick flash (pastel white) ──
        if s.kick_flash > 0.05 {
            let flash_b = s.kick_flash * 0.2;
            let color = hsv_to_rgb(s.chord_hue, 0.1, flash_b);
            for i in 0..NUM_LEDS {
                add_color(strip, i, color);
            }
        }

        // ── Age / cull particles ──

        // Orbiters
        for orb in s.orbiters.iter_mut() {
            orb.life += 1.0;
            orb.angle += orb.speed;
        }
        s.orbiters.retain(|o| o.life < o.max_life);

        // Bubbles
        let mspeed_copy = mspeed;
        for bub in s.bubbles.iter_mut() {
            bub.life += 1.0;
            bub.y += bub.dy * mspeed_copy;
            bub.x += bub.dx + (bub.wobble_phase + bub.life * 0.08).sin() * 0.08;
        }
        s.bubbles
            .retain(|b| b.life < b.max_life && b.y > -2.0 && b.y < ROWS as f32 + 2.0);
    });
}

// ── Helpers ──────────────────────────────────────────────────

/// Map an angle (radians) to a position on the perimeter of a rectangle
/// (0,0) to (width, height). Returns (x, y) local to the block.
fn angle_to_block_perimeter(angle: f32, width: f32, height: f32) -> (f32, f32) {
    // Normalize angle to 0..TAU
    let a = angle.rem_euclid(std::f32::consts::TAU);
    let perimeter = 2.0 * (width + height);
    let dist = a / std::f32::consts::TAU * perimeter;

    if dist < width {
        // Top edge: left to right
        (dist, 0.0)
    } else if dist < width + height {
        // Right edge: top to bottom
        (width - 0.5, dist - width)
    } else if dist < 2.0 * width + height {
        // Bottom edge: right to left
        (2.0 * width + height - dist, height - 0.5)
    } else {
        // Left edge: bottom to top
        (0.0, perimeter - dist)
    }
}

/// Render a soft glowing dot
fn render_glow(
    strip: &mut LedStrip,
    px: f32,
    py: f32,
    radius: f32,
    brightness: f32,
    hue: f32,
    sat: f32,
) {
    if brightness < 0.02 {
        return;
    }
    let r_ceil = radius.ceil() as isize + 1;
    let cx = px.round() as isize;
    let cy = py.round() as isize;

    for dy in -r_ceil..=r_ceil {
        let row = cy + dy;
        if row < 0 || row >= ROWS as isize {
            continue;
        }
        for dx in -r_ceil..=r_ceil {
            let col = cx + dx;
            if col < 0 || col >= COLS as isize {
                continue;
            }
            let dist_x = col as f32 - px;
            let dist_y = (row as f32 - py) * 2.0;
            let dist = (dist_x * dist_x + dist_y * dist_y).sqrt();
            if dist > radius {
                continue;
            }
            let falloff = 1.0 - dist / radius;
            let b = brightness * falloff * falloff;
            if b < 0.01 {
                continue;
            }
            let led = row as usize * COLS + col as usize;
            add_color(strip, led, hsv_to_rgb(hue, sat, b));
        }
    }
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
