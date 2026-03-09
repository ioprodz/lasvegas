use crate::command::AudioAnalysis;
use crate::hardware::led::{hsv_to_rgb, LedStrip};
use std::cell::RefCell;

const COLS: usize = 60;
const ROWS: usize = 6;
const NOTE_HUES: [f32; 12] = [
    0.0, 30.0, 55.0, 80.0, 120.0, 160.0, 195.0, 220.0, 260.0, 285.0, 320.0, 345.0,
];
const MAX_ORBS: usize = 40;

// ── Orb: a soft glowing circle that drifts in smooth curves ──

struct Orb {
    x: f32,
    y: f32,
    // Lissajous orbit parameters (smooth, predictable motion)
    cx: f32,     // orbit center x
    cy: f32,     // orbit center y
    rx: f32,     // orbit radius x
    ry: f32,     // orbit radius y
    freq_x: f32, // x frequency
    freq_y: f32, // y frequency
    phase: f32,  // phase offset
    hue: f32,
    radius: f32, // glow radius
    born: f32,   // frame born (for lifetime calc)
    lifespan: f32,
}

// ── State ────────────────────────────────────────────────────

struct DriftState {
    frame: usize,
    // Audio
    chord_hue: f32,
    chord_hue_target: f32,
    energy: f32,
    bass_smooth: f32,
    mid_smooth: f32,
    treble_smooth: f32,
    beat_brightness: f32,
    row_energy: [f32; 6],
    // Flow (same tidal background)
    flow_x: f32,
    flow_y: f32,
    // Melody
    melody_pos: f32,
    melody_target: f32,
    melody_glow: f32,
    prev_note_midi: u8,
    // Orbs
    orbs: Vec<Orb>,
    // Kick
    prev_kick: u8,
    beat_ripple_x: f32,
    beat_ripple_age: f32,
    // BPM
    bpm_smooth: f32,
    // RNG
    seed: u32,
}

impl DriftState {
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

thread_local! {
    static STATE: RefCell<DriftState> = RefCell::new(DriftState {
        frame: 0,
        chord_hue: 180.0,
        chord_hue_target: 180.0,
        energy: 0.3,
        bass_smooth: 0.0,
        mid_smooth: 0.0,
        treble_smooth: 0.0,
        beat_brightness: 0.0,
        row_energy: [0.0; 6],
        flow_x: 0.0,
        flow_y: 0.0,
        melody_pos: 0.5,
        melody_target: 0.5,
        melody_glow: 0.0,
        prev_note_midi: 0,
        orbs: Vec::new(),
        prev_kick: 0,
        beat_ripple_x: 30.0,
        beat_ripple_age: 99.0,
        bpm_smooth: 120.0,
        seed: 43210,
    });
}

/// Harmonic Memory V8 — "Drift"
/// Tidal flowing background with soft glowing orbs that drift in smooth
/// Lissajous curves across the full strip. No blocks, no flicker.
/// Orbs spawn on note changes and float gracefully, fading over time.
/// Everything is buttery smooth.
pub fn audio_harmonic8(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();
        s.frame = s.frame.wrapping_add(1);
        let t = s.frame as f32;

        // ── Unpack audio ──
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
            0.25,
            0.04,
        );
        smooth(&mut s.bass_smooth, bass_f, 0.35, 0.06);
        smooth(&mut s.mid_smooth, mid_f, 0.3, 0.06);
        smooth(&mut s.treble_smooth, high_f, 0.25, 0.08);
        let energy = s.energy.max(0.25);

        // Per-row energy
        let band_targets: [f32; 6] = [
            high_f,
            ((a.bands[5] as u16 + a.bands[6] as u16) / 2) as f32 / 255.0,
            mid_f,
            ((a.bands[2] as u16 + a.bands[3] as u16) / 2) as f32 / 255.0,
            bass_f,
            a.bands[0] as f32 / 255.0,
        ];
        for i in 0..6 {
            smooth(&mut s.row_energy[i], band_targets[i], 0.3, 0.06);
        }

        // ── BPM ──
        if bpm > 30.0 && bpm < 250.0 {
            smooth(&mut s.bpm_smooth, bpm, 0.1, 0.02);
        }
        let tempo_speed = s.bpm_smooth / 120.0;

        // ── Beat ──
        let beat_pulse = ((1.0 - beat_phase) * std::f32::consts::TAU).cos() * 0.5 + 0.5;
        smooth(&mut s.beat_brightness, beat_pulse, 0.35, 0.12);

        // ── Chord hue ──
        if a.chord_root < 12 {
            s.chord_hue_target = NOTE_HUES[a.chord_root as usize];
        }
        s.chord_hue = lerp_hue(s.chord_hue, s.chord_hue_target, 0.05);

        // ── Flow ──
        let flow_speed = 0.3 + tempo_speed * 0.5 + energy * 0.3;
        s.flow_x += flow_speed * 0.04;
        s.flow_y += flow_speed * 0.015;

        // ── Melody tracking ──
        if a.note_midi > 0 && a.note_midi != s.prev_note_midi {
            s.melody_target = ((a.note_midi as f32 - 36.0) / 48.0).clamp(0.0, 1.0);
            s.melody_glow = 1.0;

            // Spawn an orb at the melody position
            if s.orbs.len() < MAX_ORBS {
                let r1 = s.randf();
                let r2 = s.randf();
                let r3 = s.randf();
                let r4 = s.randf();
                let r5 = s.randf();
                let spawn_x = s.melody_target * (COLS - 1) as f32;
                let spawn_y = ROWS as f32 * 0.5 + (r1 - 0.5) * 2.0;
                let note_hue = if a.chord_root < 12 {
                    (NOTE_HUES[a.note_midi as usize % 12] + r2 * 20.0 - 10.0).rem_euclid(360.0)
                } else {
                    (s.chord_hue + r2 * 30.0 - 15.0).rem_euclid(360.0)
                };
                s.orbs.push(Orb {
                    x: spawn_x,
                    y: spawn_y,
                    cx: spawn_x,
                    cy: spawn_y,
                    rx: 4.0 + r3 * 8.0,
                    ry: 0.8 + r4 * 1.5,
                    freq_x: 0.8 + r5 * 1.5,
                    freq_y: 1.0 + r1 * 2.0,
                    phase: r2 * std::f32::consts::TAU,
                    hue: note_hue,
                    radius: 2.0 + r3 * 1.5,
                    born: t,
                    lifespan: 120.0 + r4 * 180.0,
                });
            }
        }
        if a.note_midi > 0 {
            s.prev_note_midi = a.note_midi;
        }
        smooth(&mut s.melody_pos, s.melody_target, 0.08, 0.08);
        s.melody_glow *= 0.97;

        // ── Kick: spawn burst of orbs + ripple ──
        let kick_onset = a.kick > s.prev_kick.saturating_add(25);
        s.prev_kick = a.kick;
        if kick_onset {
            s.beat_ripple_x = s.melody_pos * COLS as f32;
            s.beat_ripple_age = 0.0;

            // Spawn 2-3 orbs on kick
            let kick_spawns = 2 + (energy * 2.0) as usize;
            for _ in 0..kick_spawns {
                if s.orbs.len() >= MAX_ORBS {
                    break;
                }
                let r1 = s.randf();
                let r2 = s.randf();
                let r3 = s.randf();
                let r4 = s.randf();
                let r5 = s.randf();
                let r6 = s.randf();
                let spawn_x = r1 * COLS as f32;
                let spawn_y = r2 * ROWS as f32;
                s.orbs.push(Orb {
                    x: spawn_x,
                    y: spawn_y,
                    cx: spawn_x,
                    cy: spawn_y,
                    rx: 5.0 + r3 * 10.0,
                    ry: 0.6 + r4 * 1.8,
                    freq_x: 0.6 + r5 * 1.2,
                    freq_y: 0.8 + r6 * 1.5,
                    phase: r1 * std::f32::consts::TAU,
                    hue: (s.chord_hue + r2 * 50.0 - 25.0).rem_euclid(360.0),
                    radius: 2.5 + r3 * 1.5,
                    born: t,
                    lifespan: 80.0 + r4 * 150.0,
                });
            }
        }
        s.beat_ripple_age += 0.8 * tempo_speed;

        // ── Ambient orb spawning (keep some always alive) ──
        if s.orbs.len() < 8 {
            let r1 = s.randf();
            let r2 = s.randf();
            let r3 = s.randf();
            let r4 = s.randf();
            let r5 = s.randf();
            let r6 = s.randf();
            s.orbs.push(Orb {
                x: r1 * COLS as f32,
                y: r2 * ROWS as f32,
                cx: r1 * COLS as f32,
                cy: r2 * ROWS as f32,
                rx: 6.0 + r3 * 12.0,
                ry: 0.8 + r4 * 1.5,
                freq_x: 0.5 + r5 * 1.0,
                freq_y: 0.7 + r6 * 1.2,
                phase: r3 * std::f32::consts::TAU,
                hue: (s.chord_hue + r4 * 40.0 - 20.0).rem_euclid(360.0),
                radius: 2.5 + r5 * 2.0,
                born: t,
                lifespan: 200.0 + r1 * 200.0,
            });
        }

        // ── Update orb positions (smooth Lissajous drift) ──
        let orb_speed = 0.012 * tempo_speed;
        for orb in s.orbs.iter_mut() {
            let age = t - orb.born;
            let phase = orb.phase + age * orb_speed;
            orb.x = orb.cx + (phase * orb.freq_x).sin() * orb.rx;
            orb.y = orb.cy + (phase * orb.freq_y + 0.7).sin() * orb.ry;
            // Gently drift the center with the flow
            orb.cx += flow_speed * 0.003;
            // Wrap horizontally
            if orb.cx > COLS as f32 + 10.0 {
                orb.cx -= COLS as f32 + 20.0;
            }
        }
        // Cull expired orbs
        s.orbs.retain(|o| (t - o.born) < o.lifespan);

        // ══════════════════════════════════════════════
        // RENDER
        // ══════════════════════════════════════════════

        // ── Pass 1: Tidal background (same as V7) ──
        let melody_col = s.melody_pos * (COLS - 1) as f32;

        for row in 0..ROWS {
            let row_f = row as f32;
            let row_norm = row_f / (ROWS - 1) as f32;
            let row_e = s.row_energy[row].max(0.15);
            let row_flow = s.flow_x * (1.3 - row_norm * 0.6);
            let row_shift = (s.flow_y + row_f * 0.4).sin() * s.bass_smooth * 0.3;

            for col in 0..COLS {
                let col_f = col as f32;
                let col_norm = col_f / (COLS - 1) as f32;

                // Flowing hue gradient
                let hue_base = s.chord_hue
                    + col_f * 1.2
                    + row_f * 15.0
                    + (col_f * 0.08 + row_flow).sin() * 20.0;

                // Layered waves
                let wave1 = (col_f / 12.0 + row_flow * 0.15 + row_shift).sin() * 0.5 + 0.5;
                let wave2 = (col_f / 7.0 - row_flow * 0.1 + row_f * 0.6).sin() * 0.5 + 0.5;
                let wave3 =
                    (col_norm * std::f32::consts::PI + t * 0.008 * tempo_speed).sin() * 0.5 + 0.5;
                let wave_blend = wave1 * 0.45 + wave2 * 0.3 + wave3 * 0.25;

                let row_brightness = 0.12 + row_e * 0.5 + wave_blend * (0.15 + energy * 0.4);
                let beat_mod = 1.0 + s.beat_brightness * 0.15;

                // Melody glow
                let melody_dist = (col_f - melody_col).abs();
                let melody_width = 6.0 + energy * 4.0;
                let melody_brightness = s.melody_glow
                    * (-(melody_dist * melody_dist) / (melody_width * melody_width)).exp()
                    * 0.25;

                // Kick ripple
                let ripple_brightness = if s.beat_ripple_age < 30.0 {
                    let ripple_dist = (col_f - s.beat_ripple_x).abs();
                    let ripple_radius = s.beat_ripple_age * 1.5;
                    let ring_dist = (ripple_dist - ripple_radius).abs();
                    let ring = (1.0 - ring_dist / 4.0).max(0.0);
                    let fade = (1.0 - s.beat_ripple_age / 30.0).max(0.0);
                    ring * fade * 0.2
                } else {
                    0.0
                };

                // Dimmer background to let orbs pop
                let val = (row_brightness * beat_mod * 0.7 + melody_brightness + ripple_brightness)
                    .clamp(0.04, 0.85);
                let hue = (hue_base + melody_brightness * 40.0).rem_euclid(360.0);
                let sat = (0.5 - melody_brightness * 0.3).clamp(0.2, 0.55);

                let led = row * COLS + col;
                strip.set(led, hsv_to_rgb(hue, sat, val));
            }
        }

        // ── Pass 2: Orbs (additive on top of background) ──
        for orb in s.orbs.iter() {
            let age = t - orb.born;
            let progress = age / orb.lifespan;

            // Smooth fade in/out
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

            let brightness = alpha * (0.4 + energy * 0.6);
            let r = orb.radius * (1.0 + s.bass_smooth * 0.2);
            let r_ceil = r.ceil() as isize + 1;
            let cx = orb.x.round() as isize;
            let cy = orb.y.round() as isize;

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
                    let dist_x = col as f32 - orb.x;
                    let dist_y = (row as f32 - orb.y) * 2.5; // aspect ratio
                    let dist = (dist_x * dist_x + dist_y * dist_y).sqrt();
                    if dist > r {
                        continue;
                    }
                    // Smooth gaussian-ish falloff (no hard edge)
                    let falloff = (-(dist * dist) / (r * r * 0.4)).exp();
                    let b = brightness * falloff;
                    if b < 0.01 {
                        continue;
                    }
                    let led = row as usize * COLS + col as usize;
                    add_color(strip, led, hsv_to_rgb(orb.hue, 0.4, b));
                }
            }
        }
    });
}

// ── Helpers ──────────────────────────────────────────────────

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
