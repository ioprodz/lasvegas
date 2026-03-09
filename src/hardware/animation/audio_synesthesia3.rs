use crate::command::AudioAnalysis;
use crate::hardware::led::{hsv_to_rgb, LedStrip};
use std::cell::RefCell;

const COLS: usize = 60;
const ROWS: usize = 6;
const NOTE_HUES: [f32; 12] = [
    0.0, 30.0, 55.0, 80.0, 120.0, 160.0,
    195.0, 220.0, 260.0, 285.0, 320.0, 345.0,
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
    BlockKind::Kick, BlockKind::Bass, BlockKind::Melody,
    BlockKind::Snare, BlockKind::Hihat, BlockKind::Vocal,
];

struct SnareRing { age: f32, intensity: f32 }
struct Spark { col_frac: f32, row: usize, life: f32, max_life: f32, hue: f32 }

struct Syn3State {
    frame: usize,
    chord_hue: f32,
    chord_hue_target: f32,
    energy: f32,
    beat_brightness: f32,
    // Melody dot tracking — drives layout changes
    dot_x: f32,          // current dot position (0..COLS)
    dot_y: f32,          // current dot position (0..ROWS)
    dot_prev_x: f32,
    dot_prev_y: f32,
    dot_velocity: f32,   // smoothed speed
    dot_trail: Vec<(f32, f32, f32, f32)>, // (x, y, hue, life)
    // Layout change from dot velocity
    active_count: usize,
    layout: [usize; 6],
    change_cooldown: usize,
    // Kick
    prev_kick: u8,
    kick_flash: [f32; 6],
    kick_chase_pos: [f32; 6],
    // Bass
    bass_phase: f32,
    // Snare
    prev_snare: u8,
    snare_rings: Vec<(usize, SnareRing)>,
    // Hihat
    sparks: Vec<(usize, Spark)>,
    // Vocal
    vocal_glow: f32,
    // RNG
    seed: u32,
}

impl Syn3State {
    fn rand(&mut self) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed
    }
}

thread_local! {
    static STATE: RefCell<Syn3State> = RefCell::new(Syn3State {
        frame: 0,
        chord_hue: 0.0,
        chord_hue_target: 0.0,
        energy: 0.0,
        beat_brightness: 0.0,
        dot_x: 30.0,
        dot_y: 2.5,
        dot_prev_x: 30.0,
        dot_prev_y: 2.5,
        dot_velocity: 0.0,
        dot_trail: Vec::new(),
        active_count: 4,
        layout: [0, 1, 2, 3, 4, 5],
        change_cooldown: 0,
        prev_kick: 0,
        kick_flash: [0.0; 6],
        kick_chase_pos: [0.0; 6],
        bass_phase: 0.0,
        prev_snare: 0,
        snare_rings: Vec::new(),
        sparks: Vec::new(),
        vocal_glow: 0.0,
        seed: 99991,
    });
}

pub fn audio_synesthesia3(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
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

        // ---- Energy ----
        let raw_energy: f32 = a.bands.iter().map(|&b| b as f32).sum::<f32>() / (255.0 * 8.0);
        if raw_energy > s.energy {
            s.energy += (raw_energy - s.energy) * 0.3;
        } else {
            s.energy += (raw_energy - s.energy) * 0.05;
        }
        let energy = s.energy.max(0.15);

        // ---- Chord hue ----
        if a.chord_root < 12 {
            s.chord_hue_target = NOTE_HUES[a.chord_root as usize];
        }
        s.chord_hue = lerp_hue(s.chord_hue, s.chord_hue_target, 0.08);

        // ---- Beat ----
        let beat_pulse = ((1.0 - beat_phase) * std::f32::consts::TAU).cos() * 0.5 + 0.5;
        s.beat_brightness += (beat_pulse - s.beat_brightness) * 0.3;

        // ---- Spectral centroid (for dot Y) ----
        let total: f32 = a.bands.iter().map(|&b| b as f32).sum::<f32>().max(1.0);
        let norm: [f32; 8] = std::array::from_fn(|i| a.bands[i] as f32 / total);
        let centroid: f32 = norm.iter().enumerate().map(|(i, &n)| i as f32 * n).sum();

        // ============================================
        // MELODY DOT — position from pitch + centroid
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
            // Idle drift
            let t = frame as f32 * 0.015;
            s.dot_x += (t.cos()) * 0.3;
            s.dot_y += ((t * 0.7).sin()) * 0.15;
            s.dot_x = s.dot_x.clamp(1.0, (COLS - 2) as f32);
            s.dot_y = s.dot_y.clamp(0.0, (ROWS - 1) as f32);
        }

        // Velocity = distance moved this frame
        let dx = s.dot_x - s.dot_prev_x;
        let dy = (s.dot_y - s.dot_prev_y) * 3.0; // weight Y more (fewer rows)
        let instant_vel = (dx * dx + dy * dy).sqrt();
        // Smooth velocity: fast attack, medium decay
        if instant_vel > s.dot_velocity {
            s.dot_velocity += (instant_vel - s.dot_velocity) * 0.5;
        } else {
            s.dot_velocity += (instant_vel - s.dot_velocity) * 0.15;
        }

        // Drop trail point
        let dot_hue = if a.note_midi > 0 {
            NOTE_HUES[a.note_midi as usize % 12]
        } else {
            (s.chord_hue + frame as f32 * 0.3) % 360.0
        };
        if s.dot_trail.len() < 50 {
            s.dot_trail.push((s.dot_x, s.dot_y, dot_hue, 0.0));
        }

        // ============================================
        // LAYOUT CHANGE — triggered by high dot velocity
        // ============================================
        s.change_cooldown = s.change_cooldown.saturating_sub(1);

        // Threshold: velocity > 2.0 means the pitch jumped significantly
        if s.dot_velocity > 2.0 && s.change_cooldown == 0 {
            s.change_cooldown = 30; // ~500ms cooldown

            // Derive layout from current dot position + velocity
            let px = (s.dot_x * 4.0) as u32;
            let py = (s.dot_y * 8.0) as u32;
            let pv = (s.dot_velocity * 10.0) as u32;
            let mut hash = px.wrapping_mul(7) ^ py.wrapping_mul(13) ^ pv.wrapping_mul(19);

            // Block count from velocity: faster = more blocks
            let vel_norm = (s.dot_velocity / 8.0).clamp(0.0, 1.0);
            s.active_count = ((vel_norm * 4.99) as usize + 2).min(6); // 2-6 blocks

            // Permutation from hash
            let mut perm = [0usize, 1, 2, 3, 4, 5];
            for i in (1..6).rev() {
                hash = hash.wrapping_mul(1664525).wrapping_add(1013904223);
                let j = (hash as usize) % (i + 1);
                perm.swap(i, j);
            }
            s.layout = perm;
        }

        // ---- Onset detection ----
        let kick_onset = a.kick > s.prev_kick.saturating_add(25);
        s.prev_kick = a.kick;
        let snare_onset = a.snare > s.prev_snare.saturating_add(30);
        s.prev_snare = a.snare;

        // ---- Bass phase ----
        s.bass_phase += 0.04 + bass_f * 0.15 + bass_line_f * 0.1;

        // ---- Vocal glow ----
        s.vocal_glow += (vocals_f - s.vocal_glow) * 0.12;

        // ---- Clear ----
        strip.set_all([0, 0, 0, 0]);

        // ============================================
        // RENDER MELODY DOT + TRAIL (always visible, on top of blocks)
        // ============================================
        // We render the trail first (behind), then blocks, then the dot head (on top)
        let trail_max_life = 40.0;
        for &(tx, ty, thue, life) in s.dot_trail.iter() {
            let fade = (1.0 - life / trail_max_life).max(0.0);
            let fade = fade * fade;
            let b = fade * 0.7;
            if b < 0.02 { continue; }
            let sat = 0.85;
            draw_glow(strip, tx, ty, thue, sat, b, 1.5);
        }

        // ============================================
        // RENDER BLOCKS
        // ============================================
        let active_count = s.active_count;
        let block_width = COLS / active_count;
        let layout = s.layout;

        for slot in 0..active_count {
            let col_start = slot * block_width;
            let kind = ALL_KINDS[layout[slot % 6] % 6];
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

                    if flash > 0.03 {
                        let chase_row = (ROWS as f32 - 1.0) - s.kick_chase_pos[slot];
                        for row in 0..ROWS {
                            let row_dist = (row as f32 - chase_row).abs();
                            let chase_bright = if row_dist < 1.2 {
                                (1.0 - row_dist / 1.2) * flash
                            } else { 0.0 };
                            let brightness = (chase_bright + flash * 0.3).min(1.0);
                            if brightness > 0.02 {
                                let color = hsv_to_rgb(hue, 0.85, brightness);
                                for col in col_start..(col_start + block_width) {
                                    add_color(strip, row * COLS + col, color);
                                }
                            }
                        }
                    }
                    s.kick_flash[slot] *= 0.85;

                    // Idle: beat-synced vertical pulse
                    let idle_strength = (1.0 - flash.min(1.0)) * energy;
                    if idle_strength > 0.02 {
                        let wave_pos = (beat_phase * ROWS as f32) % ROWS as f32;
                        for row in 0..ROWS {
                            let dist = (row as f32 - wave_pos).abs()
                                .min((row as f32 - wave_pos + ROWS as f32).abs())
                                .min((row as f32 - wave_pos - ROWS as f32).abs());
                            let b = (1.0 - dist / 2.0).max(0.0) * idle_strength * 0.5;
                            if b > 0.02 {
                                for col in col_start..(col_start + block_width) {
                                    add_color(strip, row * COLS + col, hsv_to_rgb(hue, 0.9, b));
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
                    let intensity = (bass_line_f.max(bass_f) * 0.8
                        + 0.2 * s.beat_brightness).max(energy * 0.4);
                    let wavelength = if a.note_midi > 0 {
                        120.0 / (a.note_midi as f32).max(30.0) * 15.0
                    } else { 8.0 };

                    for col_off in 0..block_width {
                        let col = col_start + col_off;
                        let phase = (col_off as f32 / wavelength) * std::f32::consts::TAU
                            + s.bass_phase;
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
                    // This block gets the dot trail rendered into it more brightly
                    let note_hue = if a.note_midi > 0 {
                        NOTE_HUES[a.note_midi as usize % 12]
                    } else {
                        (s.chord_hue + slot_hue_off) % 360.0
                    };

                    // Idle: horizontal scanner
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

                    // Bright head glow in this block's range
                    if a.note_midi > 0 {
                        let head_x = s.dot_x;
                        let head_y = s.dot_y;
                        // Only draw extra brightness if dot is within this block
                        if head_x >= col_start as f32 && head_x < (col_start + block_width) as f32 {
                            draw_glow(strip, head_x, head_y, note_hue, 0.7, 1.0, 2.5);
                        }
                    }
                }

                BlockKind::Snare => {
                    let snare_hue = (s.chord_hue + 60.0 + slot_hue_off) % 360.0;
                    let cx = block_width as f32 / 2.0;
                    let cy = ROWS as f32 / 2.0;

                    if snare_onset && s.snare_rings.len() < 8 {
                        s.snare_rings.push((slot, SnareRing {
                            age: 0.0, intensity: snare_f.max(0.5),
                        }));
                    }

                    let mut has_active = false;
                    for &(rs, ref ring) in s.snare_rings.iter() {
                        if rs != slot { continue; }
                        has_active = true;
                        let radius = ring.age * 0.6;
                        let fade = (1.0 - ring.age / 18.0).max(0.0) * ring.intensity;
                        let ring_w = 1.2 + ring.age * 0.1;
                        for row in 0..ROWS {
                            for col_off in 0..block_width {
                                let dx = col_off as f32 - cx;
                                let dy = row as f32 - cy;
                                let dist = (dx * dx + dy * dy).sqrt();
                                let rd = (dist - radius).abs();
                                if rd < ring_w {
                                    let b = (1.0 - rd / ring_w) * fade;
                                    if b > 0.03 {
                                        add_color(strip, row * COLS + col_start + col_off,
                                            hsv_to_rgb(snare_hue, 0.9, b));
                                    }
                                }
                            }
                        }
                    }

                    // Idle: breathing ring
                    if !has_active {
                        let idle_r = ((frame as f32 * 0.04).sin() * 0.5 + 0.5)
                            * (cx.min(cy) + 1.0);
                        let idle_b = energy * 0.5;
                        for row in 0..ROWS {
                            for col_off in 0..block_width {
                                let dx = col_off as f32 - cx;
                                let dy = row as f32 - cy;
                                let dist = (dx * dx + dy * dy).sqrt();
                                let rd = (dist - idle_r).abs();
                                if rd < 1.0 {
                                    let b = (1.0 - rd) * idle_b;
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
                    let spawn = ((hihat_f * 5.0) as usize).max(1);
                    let hue_base = (s.chord_hue + 80.0 + slot_hue_off) % 360.0;
                    for _ in 0..spawn {
                        if s.sparks.len() >= 50 { break; }
                        let col_frac = (s.rand() % 1000) as f32 / 1000.0;
                        let row = (s.rand() as usize) % ROWS;
                        let max_life = 5.0 + (s.rand() % 15) as f32;
                        let hue = (hue_base + (s.rand() % 60) as f32) % 360.0;
                        s.sparks.push((slot, Spark { col_frac, row, life: 0.0, max_life, hue }));
                    }

                    for &(sp_slot, ref spark) in s.sparks.iter() {
                        if sp_slot != slot { continue; }
                        let t = spark.life / spark.max_life;
                        let envelope = if t < 0.1 { t / 0.1 }
                            else { (1.0 - (t - 0.1) / 0.9).max(0.0) };
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
                    let glow = s.vocal_glow.max(energy * 0.3);
                    let pulse = 0.5 + s.beat_brightness * 0.5;
                    let brightness = glow * pulse;
                    if brightness > 0.02 {
                        let cx = block_width as f32 / 2.0;
                        let cy = ROWS as f32 / 2.0;
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
        s.snare_rings.iter_mut().for_each(|r| r.1.age += 1.0);
        s.snare_rings.retain(|r| r.1.age < 18.0);
        s.sparks.iter_mut().for_each(|sp| sp.1.life += 1.0);
        s.sparks.retain(|sp| sp.1.life < sp.1.max_life);
    });
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
            let leds = strip.controller_leds();
            let existing = leds[idx];
            strip.set(idx, [
                existing[0].max(color[0]),
                existing[1].max(color[1]),
                existing[2].max(color[2]),
                0,
            ]);
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
