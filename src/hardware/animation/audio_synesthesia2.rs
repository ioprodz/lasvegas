use crate::command::AudioAnalysis;
use crate::hardware::led::{hsv_to_rgb, LedStrip};
use std::cell::RefCell;

const COLS: usize = 60;
const ROWS: usize = 6;
const NOTE_HUES: [f32; 12] = [
    0.0,   // C  — red
    30.0,  // C# — orange
    55.0,  // D  — gold
    80.0,  // D# — yellow-green
    120.0, // E  — green
    160.0, // F  — teal
    195.0, // F# — cyan
    220.0, // G  — blue
    260.0, // G# — indigo
    285.0, // A  — violet
    320.0, // A# — magenta
    345.0, // B  — rose
];

#[derive(Clone, Copy)]
enum BlockKind {
    Kick,
    Bass,
    Melody,
    Snare,
    Hihat,
    Vocal,
}

const ALL_KINDS: [BlockKind; 6] = [
    BlockKind::Kick,
    BlockKind::Bass,
    BlockKind::Melody,
    BlockKind::Snare,
    BlockKind::Hihat,
    BlockKind::Vocal,
];

struct SnareRing {
    age: f32,
    intensity: f32,
}

struct Spark {
    col_frac: f32,
    row: usize,
    life: f32,
    max_life: f32,
    hue: f32,
}

struct Syn2State {
    frame: usize,
    // Chord tracking
    chord_hue: f32,
    chord_hue_target: f32,
    // Overall energy (smoothed total band level)
    energy: f32,
    // Block layout
    active_count: usize,
    layout: [usize; 6],
    // Section change detection
    slow_centroid: f32,
    slow_spread: f32,
    slow_bass_ratio: f32,
    section_cooldown: usize,
    // Kick
    prev_kick: u8,
    kick_flash: [f32; 6],
    kick_chase_pos: [f32; 6],
    // Bass
    bass_phase: f32,
    // Melody
    melody_col: f32,
    melody_col_target: f32,
    melody_trail: Vec<(f32, f32, f32)>, // (col_frac, row_frac, life)
    // Snare
    prev_snare: u8,
    snare_rings: Vec<(usize, SnareRing)>,
    // Hihat
    sparks: Vec<(usize, Spark)>,
    // Vocal
    vocal_glow: f32,
    // Beat
    beat_brightness: f32,
    // RNG
    seed: u32,
}

impl Syn2State {
    fn rand(&mut self) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed
    }
}

thread_local! {
    static STATE: RefCell<Syn2State> = RefCell::new(Syn2State {
        frame: 0,
        chord_hue: 0.0,
        chord_hue_target: 0.0,
        energy: 0.0,
        active_count: 6,
        layout: [0, 1, 2, 3, 4, 5],
        slow_centroid: 3.5,
        slow_spread: 0.5,
        slow_bass_ratio: 0.3,
        section_cooldown: 0,
        prev_kick: 0,
        kick_flash: [0.0; 6],
        kick_chase_pos: [0.0; 6],
        bass_phase: 0.0,
        melody_col: 0.5,
        melody_col_target: 0.5,
        melody_trail: Vec::new(),
        prev_snare: 0,
        snare_rings: Vec::new(),
        sparks: Vec::new(),
        vocal_glow: 0.0,
        beat_brightness: 0.0,
        seed: 42069,
    });
}

pub fn audio_synesthesia2(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();
        s.frame = s.frame.wrapping_add(1);
        let frame = s.frame;

        let snare_f = a.snare as f32 / 255.0;
        let hihat_f = a.hihat as f32 / 255.0;
        let vocals_f = a.vocals as f32 / 255.0;
        let bass_line_f = a.bass_line as f32 / 255.0;
        let beat_phase = a.beat_phase as f32 / 255.0;
        let bass_f = ((a.bands[0] as u16 + a.bands[1] as u16) / 2) as f32 / 255.0;

        // ---- Overall energy (smoothed) ----
        let raw_energy: f32 = a.bands.iter().map(|&b| b as f32).sum::<f32>() / (255.0 * 8.0);
        if raw_energy > s.energy {
            s.energy += (raw_energy - s.energy) * 0.3; // fast attack
        } else {
            s.energy += (raw_energy - s.energy) * 0.05; // slow decay
        }
        // Minimum floor so idle animations always have something
        let energy = s.energy.max(0.15);

        // ---- Chord hue tracking ----
        if a.chord_root < 12 {
            s.chord_hue_target = NOTE_HUES[a.chord_root as usize];
        }
        s.chord_hue = lerp_hue(s.chord_hue, s.chord_hue_target, 0.08);

        // ---- Beat breathing ----
        let beat_pulse = ((1.0 - beat_phase) * std::f32::consts::TAU).cos() * 0.5 + 0.5;
        s.beat_brightness += (beat_pulse - s.beat_brightness) * 0.3;

        // ---- Section detection ----
        let total: f32 = a.bands.iter().map(|&b| b as f32).sum::<f32>().max(1.0);
        let norm: [f32; 8] = std::array::from_fn(|i| a.bands[i] as f32 / total);
        let centroid: f32 = norm.iter().enumerate().map(|(i, &n)| i as f32 * n).sum();
        let variance: f32 = norm.iter().enumerate()
            .map(|(i, &n)| { let d = i as f32 - centroid; d * d * n }).sum();
        let spread = variance.sqrt();
        let bass_ratio = norm[0] + norm[1];

        s.slow_centroid += (centroid - s.slow_centroid) * 0.005;
        s.slow_spread += (spread - s.slow_spread) * 0.005;
        s.slow_bass_ratio += (bass_ratio - s.slow_bass_ratio) * 0.005;
        s.section_cooldown = s.section_cooldown.saturating_sub(1);

        let divergence = ((centroid - s.slow_centroid) / 7.0).abs()
            + ((spread - s.slow_spread) / 3.0).abs()
            + (bass_ratio - s.slow_bass_ratio).abs();

        if s.section_cooldown == 0 && divergence > 0.35 && total > 50.0 {
            s.slow_centroid = centroid;
            s.slow_spread = spread;
            s.slow_bass_ratio = bass_ratio;
            s.section_cooldown = 45;

            let complexity = (centroid / 7.0) * 0.4
                + (spread / 3.0).min(1.0) * 0.3
                + (1.0 - bass_ratio) * 0.3;
            s.active_count = ((complexity * 5.99).clamp(0.0, 5.99) as usize) + 1;

            let c = (centroid * 4.0) as u32;
            let sp = (spread * 4.0) as u32;
            let b = (bass_ratio * 4.0) as u32;
            let mut h = c.wrapping_mul(7) ^ sp.wrapping_mul(13) ^ b.wrapping_mul(19);
            let mut perm = [0usize, 1, 2, 3, 4, 5];
            for i in (1..6).rev() {
                h = h.wrapping_mul(1664525).wrapping_add(1013904223);
                let j = (h as usize) % (i + 1);
                perm.swap(i, j);
            }
            s.layout = perm;
        }

        // ---- Onset detection ----
        let kick_onset = a.kick > s.prev_kick.saturating_add(25);
        s.prev_kick = a.kick;
        let snare_onset = a.snare > s.prev_snare.saturating_add(30);
        s.prev_snare = a.snare;

        // ---- Melody tracking ----
        if a.note_midi > 0 {
            let note_frac = ((a.note_midi as f32 - 36.0) / 48.0).clamp(0.0, 1.0);
            s.melody_col_target = note_frac;
        }
        s.melody_col += (s.melody_col_target - s.melody_col) * 0.2;

        if a.note_midi > 0 {
            let row_frac = centroid / 7.0;
            if s.melody_trail.len() < 40 {
                s.melody_trail.push((s.melody_col, row_frac, 0.0));
            }
        }

        // ---- Bass phase (always advance slowly, faster with bass) ----
        s.bass_phase += 0.04 + bass_f * 0.15 + bass_line_f * 0.1;

        // ---- Vocal glow ----
        s.vocal_glow += (vocals_f - s.vocal_glow) * 0.12;

        // ---- Clear ----
        strip.set_all([0, 0, 0, 0]);

        let active_count = s.active_count;
        let block_width = COLS / active_count;
        let layout = s.layout;

        // ---- Render each block ----
        for slot in 0..active_count {
            let col_start = slot * block_width;
            let kind = ALL_KINDS[layout[slot % 6] % 6];
            // Per-slot hue offset so idle patterns differ between blocks
            let slot_hue_off = slot as f32 * 55.0;

            match kind {
                BlockKind::Kick => {
                    let hue = (s.chord_hue + 180.0 + slot_hue_off) % 360.0;

                    if kick_onset {
                        s.kick_flash[slot] = 1.0;
                        s.kick_chase_pos[slot] = 0.0;
                    }
                    let flash = s.kick_flash[slot];
                    s.kick_chase_pos[slot] += 0.4;

                    // Active: chase traveling upward + flash
                    if flash > 0.03 {
                        let chase_row = (ROWS as f32 - 1.0) - s.kick_chase_pos[slot];
                        for row in 0..ROWS {
                            let row_dist = (row as f32 - chase_row).abs();
                            let chase_bright = if row_dist < 1.2 {
                                (1.0 - row_dist / 1.2) * flash
                            } else {
                                0.0
                            };
                            let flash_bright = flash * 0.3;
                            let brightness = (chase_bright + flash_bright).min(1.0);
                            if brightness > 0.02 {
                                let color = hsv_to_rgb(hue, 0.85, brightness);
                                for col in col_start..(col_start + block_width) {
                                    add_color(strip, row * COLS + col, color);
                                }
                            }
                        }
                    }
                    s.kick_flash[slot] *= 0.85;

                    // Idle: gentle vertical pulse wave synced to beat
                    let idle_strength = (1.0 - flash.min(1.0)) * energy;
                    if idle_strength > 0.02 {
                        let wave_pos = (beat_phase * ROWS as f32) % ROWS as f32;
                        for row in 0..ROWS {
                            let dist = ((row as f32 - wave_pos).abs()).min(
                                (row as f32 - wave_pos + ROWS as f32).abs()
                            ).min(
                                (row as f32 - wave_pos - ROWS as f32).abs()
                            );
                            let b = (1.0 - dist / 2.0).max(0.0) * idle_strength * 0.5;
                            if b > 0.02 {
                                let color = hsv_to_rgb(hue, 0.9, b);
                                for col in col_start..(col_start + block_width) {
                                    add_color(strip, row * COLS + col, color);
                                }
                            }
                        }
                    }
                }

                BlockKind::Bass => {
                    let bass_hue = if a.chord_root < 12 {
                        (NOTE_HUES[a.chord_root as usize] + slot_hue_off) % 360.0
                    } else {
                        (s.chord_hue + slot_hue_off) % 360.0
                    };
                    // Always show wave; intensity from bass or energy floor
                    let intensity = (bass_line_f.max(bass_f) * 0.8
                        + 0.2 * s.beat_brightness)
                        .max(energy * 0.4);

                    let wavelength = if a.note_midi > 0 {
                        120.0 / (a.note_midi as f32).max(30.0) * 15.0
                    } else {
                        8.0
                    };

                    for col_off in 0..block_width {
                        let col = col_start + col_off;
                        let phase = (col_off as f32 / wavelength) * std::f32::consts::TAU + s.bass_phase;
                        let wave = phase.sin();
                        let wave_row = (wave * 0.5 + 0.5) * (ROWS - 1) as f32;

                        for row in 0..ROWS {
                            let dist = (row as f32 - wave_row).abs();
                            if dist < 1.5 {
                                let b = (1.0 - dist / 1.5) * intensity;
                                if b > 0.03 {
                                    add_color(strip, row * COLS + col,
                                        hsv_to_rgb(bass_hue, 0.9, b));
                                }
                            }
                        }
                    }
                }

                BlockKind::Melody => {
                    let note_hue = if a.note_midi > 0 {
                        NOTE_HUES[a.note_midi as usize % 12]
                    } else {
                        (s.chord_hue + slot_hue_off) % 360.0
                    };

                    // Render trail dots
                    for &(trail_col_frac, trail_row_frac, life) in s.melody_trail.iter() {
                        let max_life = 30.0;
                        let fade = (1.0 - life / max_life).max(0.0);
                        let fade = fade * fade;
                        let tc = (trail_col_frac * (block_width - 1) as f32) as usize;
                        let tr = (trail_row_frac * (ROWS - 1) as f32).clamp(0.0, (ROWS - 1) as f32) as usize;

                        for row in 0..ROWS {
                            let dr = (row as isize - tr as isize).unsigned_abs();
                            if dr > 1 { continue; }
                            for dc in 0..=4 {
                                let col_signed = tc as isize + dc as isize - 2;
                                if col_signed < 0 || col_signed >= block_width as isize { continue; }
                                let col = col_start + col_signed as usize;
                                let dist = ((dc as f32 - 2.0).abs() / 2.0).max(dr as f32);
                                let b = (1.0 - dist) * fade;
                                if b > 0.02 {
                                    add_color(strip, row * COLS + col,
                                        hsv_to_rgb(note_hue, 0.8, b));
                                }
                            }
                        }
                    }

                    // Bright head
                    if a.note_midi > 0 {
                        let head_col = (s.melody_col * (block_width - 1) as f32) as usize;
                        let head_row = (centroid / 7.0 * (ROWS - 1) as f32)
                            .clamp(0.0, (ROWS - 1) as f32) as usize;
                        for row in 0..ROWS {
                            let dr = (row as isize - head_row as isize).unsigned_abs();
                            if dr > 1 { continue; }
                            for dc in 0..=2 {
                                let c = head_col as isize + dc as isize - 1;
                                if c < 0 || c >= block_width as isize { continue; }
                                let col = col_start + c as usize;
                                let b = if dr == 0 { 1.0 } else { 0.5 };
                                strip.set(row * COLS + col, hsv_to_rgb(note_hue, 0.7, b));
                            }
                        }
                    }

                    // Idle: slow horizontal scanner when no melody
                    if a.note_midi == 0 {
                        let scan_pos = ((frame as f32 * 0.03).sin() * 0.5 + 0.5)
                            * (block_width - 1) as f32;
                        let idle_hue = (s.chord_hue + slot_hue_off + frame as f32 * 0.5) % 360.0;
                        for row in 0..ROWS {
                            for col_off in 0..block_width {
                                let dist = (col_off as f32 - scan_pos).abs();
                                if dist < 3.0 {
                                    let b = (1.0 - dist / 3.0) * energy * 0.6;
                                    if b > 0.02 {
                                        add_color(strip, row * COLS + col_start + col_off,
                                            hsv_to_rgb(idle_hue, 0.9, b));
                                    }
                                }
                            }
                        }
                    }
                }

                BlockKind::Snare => {
                    let snare_hue = (s.chord_hue + 60.0 + slot_hue_off) % 360.0;
                    let cx = block_width as f32 / 2.0;
                    let cy = ROWS as f32 / 2.0;

                    if snare_onset {
                        if s.snare_rings.len() < 8 {
                            s.snare_rings.push((slot, SnareRing {
                                age: 0.0,
                                intensity: snare_f.max(0.5),
                            }));
                        }
                    }

                    // Active rings
                    let mut has_active_ring = false;
                    for &(ring_slot, ref ring) in s.snare_rings.iter() {
                        if ring_slot != slot { continue; }
                        has_active_ring = true;
                        let radius = ring.age * 0.6;
                        let fade = (1.0 - ring.age / 18.0).max(0.0) * ring.intensity;
                        let ring_w = 1.2 + ring.age * 0.1;

                        for row in 0..ROWS {
                            for col_off in 0..block_width {
                                let dx = col_off as f32 - cx;
                                let dy = row as f32 - cy;
                                let dist = (dx * dx + dy * dy).sqrt();
                                let ring_dist = (dist - radius).abs();
                                if ring_dist < ring_w {
                                    let b = (1.0 - ring_dist / ring_w) * fade;
                                    if b > 0.03 {
                                        add_color(strip, row * COLS + col_start + col_off,
                                            hsv_to_rgb(snare_hue, 0.9, b));
                                    }
                                }
                            }
                        }
                    }

                    // Idle: slow breathing concentric ring
                    if !has_active_ring {
                        let idle_radius = ((frame as f32 * 0.04).sin() * 0.5 + 0.5)
                            * (cx.min(cy) + 1.0);
                        let idle_b = energy * 0.5;
                        for row in 0..ROWS {
                            for col_off in 0..block_width {
                                let dx = col_off as f32 - cx;
                                let dy = row as f32 - cy;
                                let dist = (dx * dx + dy * dy).sqrt();
                                let ring_dist = (dist - idle_radius).abs();
                                if ring_dist < 1.0 {
                                    let b = (1.0 - ring_dist) * idle_b;
                                    if b > 0.02 {
                                        add_color(strip, row * COLS + col_start + col_off,
                                            hsv_to_rgb(snare_hue, 0.85, b));
                                    }
                                }
                            }
                        }
                    }
                }

                BlockKind::Hihat => {
                    // Always spawn at least 1 spark per frame for idle shimmer
                    let spawn = ((hihat_f * 5.0) as usize).max(1);
                    let hihat_hue_base = (s.chord_hue + 80.0 + slot_hue_off) % 360.0;
                    for _ in 0..spawn {
                        if s.sparks.len() >= 50 { break; }
                        let col_frac = (s.rand() % 1000) as f32 / 1000.0;
                        let row = (s.rand() as usize) % ROWS;
                        let max_life = 5.0 + (s.rand() % 15) as f32;
                        let hue = (hihat_hue_base + (s.rand() % 60) as f32) % 360.0;
                        // Brightness scales: high when hihat active, dimmer for idle
                        s.sparks.push((slot, Spark { col_frac, row, life: 0.0, max_life, hue }));
                    }

                    for &(sp_slot, ref spark) in s.sparks.iter() {
                        if sp_slot != slot { continue; }
                        let t = spark.life / spark.max_life;
                        let envelope = if t < 0.1 {
                            t / 0.1
                        } else {
                            (1.0 - (t - 0.1) / 0.9).max(0.0)
                        };
                        // Scale by hihat level, with energy floor
                        let brightness = envelope * hihat_f.max(energy * 0.4);
                        if brightness < 0.03 { continue; }
                        let col = col_start + ((spark.col_frac * (block_width - 1) as f32) as usize)
                            .min(block_width - 1);
                        strip.set(spark.row * COLS + col,
                            hsv_to_rgb(spark.hue, 0.8, brightness));
                    }
                }

                BlockKind::Vocal => {
                    let vocal_hue = (s.chord_hue + 30.0 + slot_hue_off) % 360.0;
                    // Always glow at least at energy floor level
                    let glow = s.vocal_glow.max(energy * 0.3);
                    let pulse = 0.5 + s.beat_brightness * 0.5;
                    let brightness = glow * pulse;

                    if brightness > 0.02 {
                        let cx = block_width as f32 / 2.0;
                        let cy = ROWS as f32 / 2.0;
                        // Slowly shift hue for living feel
                        let hue = (vocal_hue + (frame as f32 * 0.3).sin() * 15.0) % 360.0;

                        for row in 0..ROWS {
                            for col_off in 0..block_width {
                                let dx = (col_off as f32 - cx) / cx;
                                let dy = (row as f32 - cy) / cy;
                                let dist = (dx * dx + dy * dy).sqrt().min(1.0);
                                let b = (1.0 - dist * 0.6) * brightness;
                                if b > 0.02 {
                                    add_color(strip, row * COLS + col_start + col_off,
                                        hsv_to_rgb(hue, 0.7, b));
                                }
                            }
                        }
                    }
                }
            }
        }

        // ---- Age / cull particles ----
        s.melody_trail.iter_mut().for_each(|t| t.2 += 1.0);
        s.melody_trail.retain(|t| t.2 < 30.0);
        s.snare_rings.iter_mut().for_each(|r| r.1.age += 1.0);
        s.snare_rings.retain(|r| r.1.age < 18.0);
        s.sparks.iter_mut().for_each(|sp| sp.1.life += 1.0);
        s.sparks.retain(|sp| sp.1.life < sp.1.max_life);
    });
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
