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

// Mental model: we're drawing onto a virtual 60×60 grid; the LED strip is
// the 6 central rows (rows 27..32 of the virtual grid). Distances are in
// real pixel units (no aspect normalisation). A ring at r=10 looks like two
// vertical bars in the visible slit; as it expands to r=30 those bars
// sweep outward to the edges. That's the tunnel.
const VIRTUAL_RADIUS: f32 = 30.0; // max useful radius (strip half-width)

// History depth: each pixel reads back into this colour buffer at an index
// proportional to its distance from the centre.
const HIST_LEN: usize = 96;
// Frames per pixel of outward propagation. r=30 → age ~84 frames ≈ 2.8s.
const FRAMES_PER_PX: f32 = 2.8;

#[derive(Clone, Copy)]
struct ColorSample {
    hue: f32,
    sat: f32,
    amp: f32,
}

struct TunnelState {
    initialized: bool,
    frame: usize,
    seed: u32,

    history: Vec<ColorSample>,

    // Centre of the virtual 60×60 grid, expressed in strip coords.
    // Default: column 30, row 2.5 (between strip rows 2 and 3).
    cx: f32,
    cy: f32,
    target_cx: f32,
    target_cy: f32,
    note_steer_x: f32,
    note_steer_y: f32,
    shake_x: f32,
    shake_y: f32,

    // Spiral / rotation
    roll: f32,
    roll_rate: f32,
    roll_sign: f32,
    spiral: f32,

    // Audio smoothing — mirrored from Harmonic 4
    energy: f32,
    bass_smooth: f32,
    mid_smooth: f32,
    treble_smooth: f32,
    beat_brightness: f32,

    // Kick
    prev_kick: u8,
    kick_flash: f32,
    pulse_amp: f32,

    // Chord
    chord_hue: f32,
    chord_hue_target: f32,
    palette_offset: f32,
    speed_mult: f32,
    prev_chord_root: u8,
    chord_history: [u8; PROGRESSION_LEN],
    chord_count: usize,
    chord_stable_frames: usize,
    fingerprint: u64,

    prev_note_midi: u8,
}

impl TunnelState {
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
        self.palette_offset = ((fp >> 32) & 0xFF) as f32 / 255.0 * 330.0;
        self.speed_mult = 0.75 + ((fp >> 40) & 0xFF) as f32 / 255.0 * 1.0;
        self.roll_sign = if (fp & 1) == 0 { 1.0 } else { -1.0 };
        // Spiral amount: mostly small (concentric), occasionally large.
        let raw = ((fp >> 48) & 0xFF) as f32 / 255.0 - 0.5;
        self.spiral = raw * raw.abs() * 4.0; // -0.5 .. 0.5 biased toward 0
    }
}

thread_local! {
    static STATE: RefCell<TunnelState> = RefCell::new(TunnelState {
        initialized: false,
        frame: 0,
        seed: 0x5A5A5A5A,
        history: Vec::new(),
        cx: COLS as f32 / 2.0,
        cy: ROWS as f32 / 2.0 - 0.5, // virtual row 29.5 → between strip rows 2 and 3
        target_cx: COLS as f32 / 2.0,
        target_cy: ROWS as f32 / 2.0 - 0.5,
        note_steer_x: 0.0,
        note_steer_y: 0.0,
        shake_x: 0.0,
        shake_y: 0.0,
        roll: 0.0,
        roll_rate: 0.005,
        roll_sign: 1.0,
        spiral: 0.0,
        energy: 0.0,
        bass_smooth: 0.0,
        mid_smooth: 0.0,
        treble_smooth: 0.0,
        beat_brightness: 0.0,
        prev_kick: 0,
        kick_flash: 0.0,
        pulse_amp: 0.0,
        chord_hue: 200.0,
        chord_hue_target: 200.0,
        palette_offset: 0.0,
        speed_mult: 1.0,
        prev_chord_root: 255,
        chord_history: [255; PROGRESSION_LEN],
        chord_count: 0,
        chord_stable_frames: 0,
        fingerprint: 0,
        prev_note_midi: 0,
    });
}

/// The Tunnel — a circular node in the centre of a virtual 60×60 grid
/// (the LED strip is the 6 central rows of that grid). Colour rings emit
/// from the centre and expand outward to the edges.
///
/// The node's radius oscillates between ~1 and ~5 pixels with bass + beat +
/// idle breath. Each frame the current chord state is pushed into a colour
/// history buffer; each pixel reads back at an index proportional to its
/// distance-in-pixels from the moving centre — so chord changes become
/// colour bands travelling outward at constant speed.
///
/// Audio mapping (Harmonic V4–style):
/// - bass / kick / beat → node size + amplitude pulse pushed into history
/// - mid + vocals → ring rotation speed
/// - chord fingerprint → palette offset, roll direction, spiral amount
/// - note pitch → steering push on the vanishing point
pub fn audio_tunnel(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();
        s.frame = s.frame.wrapping_add(1);
        let frame = s.frame;
        let t = frame as f32;

        // ── Pre-fill history so the strip is alive on frame 1 ──
        if !s.initialized {
            s.initialized = true;
            s.history.clear();
            let seed_hue = s.chord_hue;
            for i in 0..HIST_LEN {
                let edge = 1.0 - (i as f32 / HIST_LEN as f32);
                s.history.push(ColorSample {
                    hue: seed_hue,
                    sat: 0.7,
                    amp: 0.25 * edge,
                });
            }
        }

        // ── Unpack audio ──
        let _hihat_f = a.hihat as f32 / 255.0;
        let vocals_f = a.vocals as f32 / 255.0;
        let beat_phase = a.beat_phase as f32 / 255.0;
        let bass_f = ((a.bands[0] as u16 + a.bands[1] as u16) / 2) as f32 / 255.0;
        let mid_f = ((a.bands[3] as u16 + a.bands[4] as u16) / 2) as f32 / 255.0;
        let high_f =
            ((a.bands[5] as u16 + a.bands[6] as u16 + a.bands[7] as u16) / 3) as f32 / 255.0;

        smooth(
            &mut s.energy,
            a.bands.iter().map(|&b| b as f32).sum::<f32>() / (255.0 * 8.0),
            0.3,
            0.05,
        );
        smooth(&mut s.bass_smooth, bass_f, 0.4, 0.08);
        smooth(&mut s.mid_smooth, mid_f, 0.35, 0.08);
        smooth(&mut s.treble_smooth, high_f, 0.3, 0.1);
        let energy = s.energy.max(0.15);

        // ── Chord hue ──
        if a.chord_root < 12 {
            s.chord_hue_target = (NOTE_HUES[a.chord_root as usize] + s.palette_offset) % 360.0;
        }
        s.chord_hue = lerp_hue(s.chord_hue, s.chord_hue_target, 0.07);

        // ── Beat (always-on breathing, even in silence) ──
        let beat_pulse = ((1.0 - beat_phase) * std::f32::consts::TAU).cos() * 0.5 + 0.5;
        smooth(&mut s.beat_brightness, beat_pulse, 0.4, 0.15);
        let idle_breath = ((t * 0.045).sin() * 0.5 + 0.5) * 0.5;

        // ── Kick → flash + emitted pulse + camera shake ──
        let kick_onset = a.kick > s.prev_kick.saturating_add(25);
        s.prev_kick = a.kick;
        if kick_onset {
            s.kick_flash = 1.0;
            s.pulse_amp = 1.0;
            let rx = s.randf();
            let ry = s.randf();
            s.shake_x = (rx - 0.5) * 2.5;
            s.shake_y = (ry - 0.5) * 0.7;
        }
        s.kick_flash *= 0.85;
        s.pulse_amp *= 0.82;
        s.shake_x *= 0.72;
        s.shake_y *= 0.72;

        // ── Chord progression fingerprint ──
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

        // ── Note onset → steering push (pitch register leans the centre) ──
        if a.note_midi > 0 && a.note_midi != s.prev_note_midi {
            let n = ((a.note_midi as f32 - 36.0) / 48.0).clamp(0.0, 1.0);
            s.note_steer_x = (n - 0.5) * 20.0;
            s.note_steer_y = (n - 0.5) * -1.5;
        }
        if a.note_midi > 0 {
            s.prev_note_midi = a.note_midi;
        }

        // ── Steering: Lissajous drift + note push, centre chases target ──
        let drift_x = (t * 0.0055).sin() * 9.0 + (t * 0.0113).cos() * 5.0;
        let drift_y = (t * 0.0083).cos() * 1.1 + (t * 0.0141).sin() * 0.6;
        s.target_cx = (COLS as f32 / 2.0) + s.note_steer_x * 0.55 + drift_x;
        s.target_cy = (ROWS as f32 / 2.0 - 0.5) + s.note_steer_y + drift_y;
        s.cx += (s.target_cx - s.cx) * 0.045;
        s.cy += (s.target_cy - s.cy) * 0.045;
        s.note_steer_x *= 0.985;
        s.note_steer_y *= 0.985;

        // ── Roll (rotates the spiral pattern slowly) ──
        let target_roll_rate = 0.003 + s.mid_smooth * 0.018 + vocals_f * 0.012;
        s.roll_rate += (target_roll_rate - s.roll_rate) * 0.05;
        s.roll += s.roll_rate * s.roll_sign;

        // ── Emit current state into history ──
        // The amplitude here is what travels outward as a visible wave front.
        let emit_amp = (0.30
            + bass_f * 0.45
            + s.beat_brightness * energy * 0.30
            + s.pulse_amp * 0.55
            + idle_breath * 0.15)
            .min(1.0);
        s.history.insert(
            0,
            ColorSample {
                hue: s.chord_hue,
                sat: (0.78 + s.treble_smooth * 0.15).min(0.98),
                amp: emit_amp,
            },
        );
        while s.history.len() > HIST_LEN {
            s.history.pop();
        }

        // ── Central node radius: oscillates between ~0.5 and ~2.5 pixels ──
        // 0.5 (silence baseline) → 2.5 (kick + bass + beat all peaking)
        let core_radius = 0.5
            + idle_breath * 0.7
            + s.beat_brightness * 0.45
            + s.bass_smooth * 1.1
            + s.pulse_amp * 0.75;
        let core_radius = core_radius.clamp(0.5, 2.5);

        // ══════════════════════════════════════════════
        // RENDER
        // ══════════════════════════════════════════════
        strip.set_all([0, 0, 0, 0]);

        let cx = s.cx + s.shake_x;
        let cy = s.cy + s.shake_y;

        // Wave-front parameters (rings on top of the colour history)
        // wavelength_px: smaller = denser rings. mid/treble compress it.
        let wavelength_px = (9.0 - s.mid_smooth * 3.5 - s.treble_smooth * 1.5).max(3.5);
        // Outward ring drift speed in cycles per frame
        let ring_drift = (0.045 + s.bass_smooth * 0.06) * s.speed_mult;
        // Spiral amount, scaled by energy so quiet sections stay concentric
        let spiral = s.spiral * (0.4 + s.mid_smooth * 1.2 + s.bass_smooth * 0.5);

        // Inverse of the propagation constant: pixels of distance → history age
        let r_to_age = FRAMES_PER_PX;

        // Soft-edge fade band beyond the core (0.8 px of glow)
        let core_glow_band = 0.8;

        let chord_hue = s.chord_hue;

        for row in 0..ROWS {
            for col in 0..COLS {
                let dx = col as f32 - cx;
                let dy = row as f32 - cy;
                let r_px = (dx * dx + dy * dy).sqrt();

                // ── Colour from history ──
                // r=0 reads the freshest sample, r=30 reads ~84 frames ago.
                let age = (r_px * r_to_age) as usize;
                let hist = if age < s.history.len() {
                    s.history[age]
                } else {
                    s.history[s.history.len() - 1]
                };

                // ── Ring modulation ──
                // phase in cycles: r_px / wavelength outward, drifting with t,
                // twisted into a spiral when the fingerprint says so.
                let theta = dy.atan2(dx);
                let phase = r_px / wavelength_px - t * ring_drift
                    + (theta + s.roll) * spiral / std::f32::consts::TAU;
                let ring = (phase * std::f32::consts::TAU).sin();
                let ring_amp = (ring * 0.5 + 0.5).powi(2);

                // ── Central node (1-5 px radius disk with soft edge) ──
                let core = if r_px <= core_radius {
                    1.0
                } else if r_px < core_radius + core_glow_band {
                    let f = (r_px - core_radius) / core_glow_band;
                    let inv = 1.0 - f;
                    inv * inv
                } else {
                    0.0
                };

                // ── Edge fade so far corners don't blow out ──
                let edge_fade = ((VIRTUAL_RADIUS + 2.0 - r_px) / 6.0).clamp(0.0, 1.0);

                // ── Combine layers ──
                // Body: colour-history gradient modulated by the ring wave
                let body_b = hist.amp * (0.55 + ring_amp * 0.55) * edge_fade;
                // Core: bright disk; brighter than the body so it reads as
                // the node, with a small additive shimmer for life
                let core_b = core * (0.55 + s.beat_brightness * 0.30 + s.bass_smooth * 0.25);

                let b = (body_b + core_b).min(1.0);
                if b < 0.02 {
                    continue;
                }

                // The core uses the freshest chord hue directly so it pops
                // even when the surrounding history is showing older colours.
                let hue = if core > 0.05 {
                    let mix = core.min(1.0);
                    let h_body = (hist.hue + theta.to_degrees() * 0.05 + r_px * 0.6)
                        .rem_euclid(360.0);
                    lerp_hue(h_body, chord_hue, mix)
                } else {
                    (hist.hue + theta.to_degrees() * 0.05 + r_px * 0.6).rem_euclid(360.0)
                };

                let sat = if core > 0.5 {
                    (hist.sat * 0.9).min(0.95)
                } else {
                    hist.sat
                };
                let color = hsv_to_rgb(hue, sat, b);
                strip.set(row * COLS + col, color);
            }
        }

        // ── Kick flash overlay ──
        if s.kick_flash > 0.05 {
            let flash_b = s.kick_flash * 0.22;
            let color = hsv_to_rgb((s.chord_hue + 180.0) % 360.0, 0.18, flash_b);
            for i in 0..NUM_LEDS {
                add_color(strip, i, color);
            }
        }
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
