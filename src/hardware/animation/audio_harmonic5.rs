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
const MAX_SWIMMERS: usize = 24;
const MAX_EMBERS: usize = 40;
const MAX_TRAVELERS: usize = 10;

// ── Particles ────────────────────────────────────────────────

/// A dot that orbits in Lissajous curves, leaving a fading trail
struct Swimmer {
    cx: f32,     // orbit center x (within block, 0..block_width)
    cy: f32,     // orbit center y (0..ROWS)
    rx: f32,     // x radius
    ry: f32,     // y radius
    freq_x: f32, // x oscillation frequency
    freq_y: f32, // y oscillation frequency
    phase: f32,  // phase offset
    hue: f32,
    life: f32,
    max_life: f32,
    trail: [(f32, f32); 8], // last 8 positions for trail
    trail_idx: usize,
}

/// Rising ember for fire effect
struct Ember {
    x: f32,
    y: f32,
    dx: f32, // horizontal wobble
    dy: f32, // rise speed (negative = up)
    hue: f32,
    life: f32,
    max_life: f32,
    size: f32,
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

struct Hm5State {
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
    melody_speed: f32, // driven by note change rate
    note_change_acc: f32,
    prev_note_midi: u8,
    travelers: Vec<WaveTraveler>,
    swimmers: Vec<Swimmer>,
    embers: Vec<Ember>,
    // Fire heat map: per-column heat values (used by fire blocks)
    fire_heat: [[f32; COLS]; ROWS],
    // Plasma phase accumulators
    plasma_phase1: f32,
    plasma_phase2: f32,
    prev_kick: u8,
    kick_flash: f32,
    seed: u32,
}

impl Hm5State {
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
    static STATE: RefCell<Hm5State> = RefCell::new(Hm5State {
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
        swimmers: Vec::new(),
        embers: Vec::new(),
        fire_heat: [[0.0; COLS]; ROWS],
        plasma_phase1: 0.0,
        plasma_phase2: 0.0,
        prev_kick: 0,
        kick_flash: 0.0,
        seed: 55555,
    });
}

/// Harmonic Memory V5 — Illusions & Organic FX.
/// Same chord-fingerprint sync as V4. Six new block types:
/// fire, vortex spiral, lava blobs, plasma, swimmer dots, strobe rings.
/// Melody change rate drives animation speed; chords drive colors.
pub fn audio_harmonic5(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();
        s.frame = s.frame.wrapping_add(1);
        let frame = s.frame;
        let t = frame as f32;

        // ── Unpack audio ──
        let hihat_f = a.hihat as f32 / 255.0;
        let _vocals_f = a.vocals as f32 / 255.0;
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

        // ── Melody speed: note changes drive animation speed ──
        s.note_change_acc *= 0.97; // decay
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
        s.kick_flash *= 0.82;

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

        // ── Note tracking & melody speed accumulator ──
        if a.note_midi > 0 && a.note_midi != s.prev_note_midi && s.prev_note_midi > 0 {
            let interval = (a.note_midi as i16 - s.prev_note_midi as i16).unsigned_abs();
            s.note_change_acc = (s.note_change_acc + 0.15 + interval as f32 * 0.02).min(1.5);

            // Spawn mirror travelers
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

        // ── Plasma phase advance (melody-driven speed) ──
        s.plasma_phase1 += 0.03 * mspeed;
        s.plasma_phase2 += 0.02 * mspeed;

        // ── Spawn swimmers ──
        let active_count = s.active_count;
        let block_width = COLS / active_count;
        if s.swimmers.len() < MAX_SWIMMERS && s.randf() < (energy * 0.2 + hihat_f * 0.15) {
            let r1 = s.randf();
            let r2 = s.randf();
            let r3 = s.randf();
            let r4 = s.randf();
            let r5 = s.randf();
            let r6 = s.randf();
            let r7 = s.randf();
            let r8 = s.randf();
            let hue = (s.chord_hue + r1 * 50.0 - 25.0).rem_euclid(360.0);
            // Pick a random block to spawn in
            let slot = (r2 * active_count as f32) as usize % active_count;
            let col_start = slot * block_width;
            let bw = block_width as f32;
            s.swimmers.push(Swimmer {
                cx: col_start as f32 + bw * 0.5 + (r3 - 0.5) * bw * 0.4,
                cy: ROWS as f32 * 0.5 + (r4 - 0.5) * 2.0,
                rx: 2.0 + r5 * (bw * 0.3).min(8.0),
                ry: 0.8 + r6 * 1.5,
                freq_x: 1.0 + r7 * 2.0,
                freq_y: 1.0 + r8 * 3.0,
                phase: r3 * std::f32::consts::TAU,
                hue,
                life: 0.0,
                max_life: 60.0 + r4 * 80.0,
                trail: [(-10.0, -10.0); 8],
                trail_idx: 0,
            });
        }

        // ── Spawn embers (for fire blocks) ──
        let ember_rate = s.bass_smooth * 0.6 + energy * 0.3;
        if s.embers.len() < MAX_EMBERS && s.randf() < ember_rate {
            let r1 = s.randf();
            let r2 = s.randf();
            let r3 = s.randf();
            let r4 = s.randf();
            let r5 = s.randf();
            // Fire hue: red-orange-yellow range, shifted by chord
            let base_fire_hue = s.chord_hue.rem_euclid(360.0);
            let fire_hue = (base_fire_hue + r1 * 40.0 - 10.0).rem_euclid(360.0);
            s.embers.push(Ember {
                x: r2 * COLS as f32,
                y: ROWS as f32 - 0.5, // start at bottom
                dx: (r3 - 0.5) * 0.3,
                dy: -(0.06 + r4 * 0.12), // rise up
                hue: fire_hue,
                life: 0.0,
                max_life: 20.0 + r5 * 30.0,
                size: 0.8 + r3 * 0.8,
            });
        }

        // ══════════════════════════════════════════════
        // RENDER
        // ══════════════════════════════════════════════
        strip.set_all([0, 0, 0, 0]);

        // ── Global: subtle beat breathing ──
        {
            let breath = s.beat_brightness * energy * 0.12;
            if breath > 0.01 {
                let color = hsv_to_rgb(s.chord_hue, 0.3, breath);
                for i in 0..NUM_LEDS {
                    add_color(strip, i, color);
                }
            }
        }

        // ── Note wave travelers ──
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
                    // ── FIRE ──
                    // Heat rises from bottom. Bass injects heat. Melody speed
                    // controls flicker rate. Reversed: fire falls from top.
                    let inject_row = if reversed { 0 } else { ROWS - 1 };
                    let rise_dir: isize = if reversed { 1 } else { -1 };

                    for col_off in 0..block_width {
                        let gc = col_start + col_off;
                        // Inject heat at base
                        let flicker =
                            fast_sin(t * 0.15 * mspeed + col_off as f32 * 0.7) * 0.3 + 0.7;
                        let heat_inject = (s.bass_smooth * 1.5 + energy * 0.5) * flicker;
                        s.fire_heat[inject_row][gc] =
                            (s.fire_heat[inject_row][gc] + heat_inject * 0.3).min(1.0);
                    }

                    // Propagate heat (shift rows)
                    if !reversed {
                        for row in 0..(ROWS - 1) {
                            for col_off in 0..block_width {
                                let gc = col_start + col_off;
                                // Pull heat from row below, with cooling
                                let below = s.fire_heat[row + 1][gc];
                                // Spread sideways slightly
                                let left = if gc > 0 {
                                    s.fire_heat[row + 1][gc - 1]
                                } else {
                                    below
                                };
                                let right = if gc < COLS - 1 {
                                    s.fire_heat[row + 1][gc + 1]
                                } else {
                                    below
                                };
                                let avg = below * 0.6 + left * 0.2 + right * 0.2;
                                let cool = 0.08 + (1.0 - energy) * 0.05;
                                s.fire_heat[row][gc] = (avg - cool).max(0.0);
                            }
                        }
                    } else {
                        for row in (1..ROWS).rev() {
                            for col_off in 0..block_width {
                                let gc = col_start + col_off;
                                let above = s.fire_heat[row - 1][gc];
                                let left = if gc > 0 {
                                    s.fire_heat[row - 1][gc - 1]
                                } else {
                                    above
                                };
                                let right = if gc < COLS - 1 {
                                    s.fire_heat[row - 1][gc + 1]
                                } else {
                                    above
                                };
                                let avg = above * 0.6 + left * 0.2 + right * 0.2;
                                let cool = 0.08 + (1.0 - energy) * 0.05;
                                s.fire_heat[row][gc] = (avg - cool).max(0.0);
                            }
                        }
                    }

                    // Render heat as fire colors
                    for row in 0..ROWS {
                        // Distance from heat source
                        let dist = if reversed {
                            row as f32 / (ROWS - 1) as f32
                        } else {
                            1.0 - row as f32 / (ROWS - 1) as f32
                        };
                        let _ = (rise_dir, dist);
                        for col_off in 0..block_width {
                            let gc = col_start + col_off;
                            let heat = s.fire_heat[row][gc];
                            if heat < 0.02 {
                                continue;
                            }
                            // Fire palette: black -> red -> orange -> yellow -> white
                            let (hue, sat, val) = fire_palette(heat, s.chord_hue);
                            let led = row * COLS + gc;
                            add_color(strip, led, hsv_to_rgb(hue, sat, val));
                        }
                    }
                }
                1 => {
                    // ── VORTEX SPIRAL ──
                    // Rotating spiral creates optical illusion of motion.
                    // Melody speed controls rotation. Direction reversal
                    // reverses spin.
                    let spin = t * 0.06 * mspeed * dir + slot_phase;
                    let arms = 2.0 + (s.bass_smooth * 2.0).floor(); // 2-4 spiral arms

                    for row in 0..ROWS {
                        for col_off in 0..block_width {
                            let x = if mirror {
                                block_width - 1 - col_off
                            } else {
                                col_off
                            };
                            // Normalized coords centered on block
                            let nx = (x as f32 - block_cx) / block_cx;
                            let ny = (row as f32 - row_cy) / row_cy.max(1.0);
                            // Scale y for aspect ratio (cols >> rows)
                            let ny_scaled = ny * (block_cx / row_cy.max(1.0)).min(3.0);

                            let dist = (nx * nx + ny_scaled * ny_scaled).sqrt();
                            let angle = ny_scaled.atan2(nx);

                            // Spiral: angle + distance creates arms
                            let spiral = (angle * arms + dist * 6.0 - spin).sin() * 0.5 + 0.5;
                            // Fade at edges
                            let edge_fade = (1.0 - dist * 0.7).max(0.0);
                            let val =
                                (spiral * edge_fade * (0.3 + energy * 1.2)).max(0.08 * edge_fade);
                            if val < 0.02 {
                                continue;
                            }
                            // Hue shifts along the spiral
                            let hue = (s.chord_hue
                                + angle.to_degrees() * 0.5
                                + dist * 40.0
                                + slot_phase * 30.0)
                                % 360.0;
                            let led = row * COLS + col_start + col_off;
                            add_color(strip, led, hsv_to_rgb(hue.rem_euclid(360.0), 0.9, val));
                        }
                    }
                }
                2 => {
                    // ── LAVA BLOBS ──
                    // Metaball-like organic blobs that drift and merge.
                    // 3 implicit blobs whose positions oscillate. Bass makes
                    // them bigger, melody speed makes them move faster.
                    let speed_f = 0.03 * mspeed;

                    // 3 blob centers
                    let blob_cx: [f32; 3] = [
                        block_cx + fast_sin(t * speed_f + slot_phase) * block_cx * 0.6,
                        block_cx + fast_sin(t * speed_f * 1.3 + slot_phase + 2.0) * block_cx * 0.5,
                        block_cx + fast_sin(t * speed_f * 0.7 + slot_phase + 4.5) * block_cx * 0.4,
                    ];
                    let blob_cy: [f32; 3] = [
                        row_cy + fast_sin(t * speed_f * 0.8 + slot_phase + 1.0) * row_cy * 0.7,
                        row_cy + fast_sin(t * speed_f * 1.1 + slot_phase + 3.5) * row_cy * 0.6,
                        row_cy + fast_sin(t * speed_f * 0.6 + slot_phase + 5.0) * row_cy * 0.8,
                    ];
                    let blob_r = 2.5 + s.bass_smooth * 3.0; // blob "radius"

                    for row in 0..ROWS {
                        for col_off in 0..block_width {
                            let x = if mirror {
                                block_width - 1 - col_off
                            } else {
                                col_off
                            };
                            let px = x as f32;
                            let py = row as f32;

                            // Metaball field: sum of 1/dist^2 for each blob
                            let mut field = 0.0f32;
                            for b in 0..3 {
                                let dx = px - blob_cx[b];
                                // Scale y for aspect ratio
                                let dy = (py - blob_cy[b]) * 2.5;
                                let dist_sq = dx * dx + dy * dy + 0.5;
                                field += (blob_r * blob_r) / dist_sq;
                            }

                            // Threshold: above 1.0 = inside blob
                            if field < 0.3 {
                                continue;
                            }
                            let inside = (field - 0.3).min(1.0);
                            // Edge glow at threshold boundary
                            let edge = if field > 0.8 && field < 1.2 {
                                1.0 - (field - 1.0).abs() * 5.0
                            } else {
                                0.0
                            }
                            .max(0.0);

                            let val = (inside * 0.6 + edge * 0.4) * (0.4 + energy * 1.0);
                            let val = val.max(inside * 0.15);
                            if val < 0.02 {
                                continue;
                            }
                            // Hue: shifts across field, chord-driven
                            let hue =
                                (s.chord_hue + field * 30.0 + slot_phase * 40.0).rem_euclid(360.0);
                            let sat = 0.75 + edge * 0.25;
                            let led = row * COLS + col_start + col_off;
                            add_color(strip, led, hsv_to_rgb(hue, sat, val));
                        }
                    }
                }
                3 => {
                    // ── PLASMA ──
                    // Classic plasma effect: overlapping sine waves create
                    // shifting color interference patterns. Melody speed drives
                    // animation rate. Direction reverses wave flow.
                    let p1 = s.plasma_phase1 * dir;
                    let p2 = s.plasma_phase2 * dir;

                    for row in 0..ROWS {
                        for col_off in 0..block_width {
                            let x = if mirror {
                                block_width - 1 - col_off
                            } else {
                                col_off
                            };
                            let nx = x as f32 / block_width as f32;
                            let ny = row as f32 / ROWS as f32;

                            let v1 = fast_sin(nx * 8.0 + p1);
                            let v2 = fast_sin(ny * 6.0 + p2 + slot_phase);
                            let v3 = fast_sin((nx + ny) * 5.0 - p1 * 0.7 + slot_phase * 0.5);
                            let v4 = fast_sin(
                                ((nx - 0.5) * (nx - 0.5) + (ny - 0.5) * (ny - 0.5)).sqrt() * 8.0
                                    + p2 * 1.3,
                            );

                            let plasma = (v1 + v2 + v3 + v4) * 0.25 + 0.5; // 0..1
                            let hue = (s.chord_hue + plasma * 120.0 + slot_phase * 25.0)
                                .rem_euclid(360.0);
                            let val = (plasma * (0.3 + energy * 1.3)).max(0.12);
                            let sat = 0.7 + (1.0 - plasma) * 0.3;
                            let led = row * COLS + col_start + col_off;
                            add_color(strip, led, hsv_to_rgb(hue, sat, val));
                        }
                    }
                }
                4 => {
                    // ── SWIMMER DOTS ──
                    // Lissajous-orbiting dots with fading trails.
                    // Each swimmer belongs to any block; we render only those
                    // whose center is within this block.
                    for sw in s.swimmers.iter() {
                        if sw.cx < col_start as f32 - 3.0
                            || sw.cx >= (col_start + block_width) as f32 + 3.0
                        {
                            continue;
                        }

                        let progress = sw.life / sw.max_life;
                        let alpha = if progress < 0.15 {
                            progress / 0.15
                        } else if progress > 0.75 {
                            (1.0 - progress) / 0.25
                        } else {
                            1.0
                        };
                        if alpha < 0.02 {
                            continue;
                        }

                        // Current position
                        let phase = sw.phase + sw.life * 0.08 * mspeed * dir;
                        let px = sw.cx + fast_sin(phase * sw.freq_x) * sw.rx;
                        let py = sw.cy + fast_sin(phase * sw.freq_y + 0.5) * sw.ry;

                        // Draw dot with glow
                        let radius = 1.5 + energy * 0.5;
                        render_glow(
                            strip,
                            px,
                            py,
                            radius,
                            alpha * (0.5 + energy * 1.0),
                            sw.hue,
                            0.85,
                        );

                        // Draw trail
                        for (ti, &(tx, ty)) in sw.trail.iter().enumerate() {
                            if tx < -5.0 {
                                continue;
                            }
                            let trail_alpha =
                                alpha * (1.0 - ti as f32 / sw.trail.len() as f32) * 0.4;
                            if trail_alpha < 0.02 {
                                continue;
                            }
                            render_glow(
                                strip,
                                tx,
                                ty,
                                0.8,
                                trail_alpha * (0.3 + energy * 0.5),
                                sw.hue,
                                0.6,
                            );
                        }
                    }
                }
                5 => {
                    // ── STROBE RINGS ──
                    // Concentric rings expand outward from center, creating
                    // depth/tunnel illusion. Beat triggers new rings.
                    // Reversed: rings contract inward.
                    let max_dist = block_cx + 2.0;
                    // Multiple ring phases
                    let ring_count = 3;
                    for ring_i in 0..ring_count {
                        let ring_phase =
                            t * 0.08 * mspeed * dir + ring_i as f32 * max_dist / ring_count as f32;
                        let ring_pos = ring_phase.rem_euclid(max_dist);
                        let ring_brightness = (1.0 - ring_pos / max_dist)
                            * (0.4 + energy * 1.0)
                            * (0.6 + s.beat_brightness * 0.6);

                        if ring_brightness < 0.03 {
                            continue;
                        }

                        for row in 0..ROWS {
                            for col_off in 0..block_width {
                                let x = if mirror {
                                    block_width - 1 - col_off
                                } else {
                                    col_off
                                };
                                let dx = x as f32 - block_cx;
                                let dy =
                                    (row as f32 - row_cy) * (block_cx / row_cy.max(1.0)).min(3.0);
                                let dist = (dx * dx + dy * dy).sqrt();

                                // Ring thickness
                                let ring_dist = (dist - ring_pos).abs();
                                if ring_dist > 2.0 {
                                    continue;
                                }
                                let ring_fade = (1.0 - ring_dist / 2.0).max(0.0);
                                let val = ring_fade * ring_brightness;
                                if val < 0.02 {
                                    continue;
                                }

                                let hue = (s.chord_hue
                                    + ring_i as f32 * 40.0
                                    + dist * 3.0
                                    + slot_phase * 25.0)
                                    .rem_euclid(360.0);
                                let led = row * COLS + col_start + col_off;
                                add_color(strip, led, hsv_to_rgb(hue, 0.85, val));
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // ── Embers overlay (rendered on top of fire blocks) ──
        for ember in s.embers.iter() {
            let progress = ember.life / ember.max_life;
            let alpha = if progress < 0.1 {
                progress / 0.1
            } else {
                (1.0 - progress).max(0.0)
            };
            if alpha < 0.02 {
                continue;
            }
            render_glow(
                strip,
                ember.x,
                ember.y,
                ember.size,
                alpha * (0.4 + energy * 0.8),
                ember.hue,
                0.9,
            );
        }

        // ── Kick flash overlay ──
        if s.kick_flash > 0.05 {
            let flash_b = s.kick_flash * 0.25;
            let color = hsv_to_rgb((s.chord_hue + 180.0) % 360.0, 0.2, flash_b);
            for i in 0..NUM_LEDS {
                add_color(strip, i, color);
            }
        }

        // ── Age / cull particles ──

        // Swimmers: update position and trail
        let mspeed_copy = mspeed;
        let block_reversed = s.block_reversed;
        for sw in s.swimmers.iter_mut() {
            sw.life += 1.0;
            // Record trail every 3 frames
            if sw.life as usize % 3 == 0 {
                let slot_idx =
                    (sw.cx / block_width as f32).clamp(0.0, (active_count - 1) as f32) as usize;
                let sw_dir: f32 = if block_reversed[slot_idx % 6] {
                    -1.0
                } else {
                    1.0
                };
                let phase = sw.phase + sw.life * 0.08 * mspeed_copy * sw_dir;
                let px = sw.cx + fast_sin(phase * sw.freq_x) * sw.rx;
                let py = sw.cy + fast_sin(phase * sw.freq_y + 0.5) * sw.ry;
                let idx = sw.trail_idx % sw.trail.len();
                sw.trail[idx] = (px, py);
                sw.trail_idx += 1;
            }
        }
        s.swimmers.retain(|sw| sw.life < sw.max_life);

        // Embers: rise/fall and wobble
        for ember in s.embers.iter_mut() {
            ember.life += 1.0;
            ember.y += ember.dy * mspeed;
            ember.x += ember.dx * fast_sin(ember.life * 0.2) * 0.5;
        }
        s.embers
            .retain(|e| e.life < e.max_life && e.y > -1.0 && e.y < ROWS as f32 + 1.0);
    });
}

// ── Helpers ──────────────────────────────────────────────────

/// Fast approximate sin (good enough for animation)
fn fast_sin(x: f32) -> f32 {
    x.sin()
}

/// Fire color palette: heat 0..1 → (hue, sat, val)
fn fire_palette(heat: f32, chord_hue: f32) -> (f32, f32, f32) {
    // Blend: low heat = deep red/chord, high heat = bright yellow/white
    let base_hue = chord_hue.rem_euclid(360.0);
    if heat < 0.25 {
        // Dark red glow
        let h = base_hue;
        (h, 1.0, heat * 2.0)
    } else if heat < 0.5 {
        // Red to orange
        let t = (heat - 0.25) * 4.0;
        let h = (base_hue + t * 30.0).rem_euclid(360.0);
        (h, 1.0 - t * 0.1, 0.5 + t * 0.3)
    } else if heat < 0.75 {
        // Orange to yellow
        let t = (heat - 0.5) * 4.0;
        let h = (base_hue + 30.0 + t * 20.0).rem_euclid(360.0);
        (h, 0.9 - t * 0.3, 0.8 + t * 0.15)
    } else {
        // Yellow to white-hot
        let t = (heat - 0.75) * 4.0;
        let h = (base_hue + 50.0).rem_euclid(360.0);
        (h, (0.6 - t * 0.5).max(0.1), (0.95 + t * 0.05).min(1.0))
    }
}

/// Render a soft glowing dot at (px, py) with given radius
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
            let dist_y = (row as f32 - py) * 2.0; // aspect ratio stretch
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
