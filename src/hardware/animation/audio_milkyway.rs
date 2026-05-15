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

const NUM_STARS: usize = 32;
const MAX_SHOOTING: usize = 3;

// Galactic plane sits between rows 2 and 3 (virtual y = 29.5 on a 60×60).
const GALACTIC_PLANE: f32 = ROWS as f32 / 2.0 - 0.5;

// Disc envelope: wide horizontal gaussian, narrow vertical gaussian — the
// band we see crossing the sky.
const DISC_SIGMA_X: f32 = 16.0;
const DISC_SIGMA_Y: f32 = 1.35;

// Bulge: squashed ellipse (dy multiplied so it stays in the central band).
const BULGE_RADIUS_BASE: f32 = 4.5;
const BULGE_DY_SQUASH: f32 = 2.4;

// Dust lane (thin dark stripe exactly on the plane).
const LANE_SIGMA: f32 = 0.55;

#[derive(Clone, Copy)]
struct Star {
    x_galaxy: f32, // x in the galaxy frame; rendered position = (x + pan) mod COLS
    y: f32,
    hue: f32,
    sat: f32,
    base_b: f32,
    twinkle_phase: f32,
    twinkle_speed: f32,
}

struct ShootingStar {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    life: f32,
    hue: f32,
    brightness: f32,
}

struct GalaxyState {
    initialized: bool,
    frame: usize,
    seed: u32,

    stars: Vec<Star>,
    shooting: Vec<ShootingStar>,

    // Camera
    cy: f32,
    target_cy: f32,
    shake_x: f32,
    shake_y: f32,

    // Pan = apparent galactic rotation (stars scroll across)
    pan: f32,
    pan_speed: f32,
    pan_direction: f32,

    // Audio smoothing
    energy: f32,
    bass_smooth: f32,
    mid_smooth: f32,
    treble_smooth: f32,
    beat_brightness: f32,

    // Kick / supernova
    prev_kick: u8,
    kick_flash: f32,
    nova_idx: Option<usize>,
    nova_t: f32,

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

impl GalaxyState {
    fn rand(&mut self) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed
    }
    fn randf(&mut self) -> f32 {
        (self.rand() % 10000) as f32 / 10000.0
    }
    fn randn(&mut self) -> f32 {
        // Standard-normal via Box–Muller. Used to cluster stars around the plane.
        let u1 = self.randf().max(1e-6);
        let u2 = self.randf();
        (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos()
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
        // Palette offset stays subtle (±30°) so the galaxy keeps its blue/orange identity.
        self.palette_offset = ((fp >> 32) & 0xFF) as f32 / 255.0 * 60.0 - 30.0;
        self.speed_mult = 0.7 + ((fp >> 40) & 0xFF) as f32 / 255.0 * 0.9;
        self.pan_direction = if (fp & 1) == 0 { 1.0 } else { -1.0 };
    }
}

thread_local! {
    static STATE: RefCell<GalaxyState> = RefCell::new(GalaxyState {
        initialized: false,
        frame: 0,
        seed: 0xC0FFEE42,
        stars: Vec::new(),
        shooting: Vec::new(),
        cy: GALACTIC_PLANE,
        target_cy: GALACTIC_PLANE,
        shake_x: 0.0,
        shake_y: 0.0,
        pan: 0.0,
        pan_speed: 0.0,
        pan_direction: 1.0,
        energy: 0.0,
        bass_smooth: 0.0,
        mid_smooth: 0.0,
        treble_smooth: 0.0,
        beat_brightness: 0.0,
        prev_kick: 0,
        kick_flash: 0.0,
        nova_idx: None,
        nova_t: 0.0,
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

/// Milky Way — edge-on view of a galaxy crossing the strip.
///
/// The 60×6 strip is the 6 central rows of a virtual 60×60 view. A warm
/// yellow-orange bulge sits at the centre, a cool blueish disc fades
/// outward, and a thin reddish dust lane bisects the plane between rows 2
/// and 3. ~42 stars are sprinkled around the plane (gaussian dispersion)
/// and slowly pan across as if the galaxy were rotating.
///
/// Audio mapping (Harmonic V4–style):
/// - bass + beat → bulge size and brightness
/// - kick → supernova flash on a random star + camera shake
/// - mid + vocals → pan speed
/// - treble + hihat → star twinkle intensity
/// - chord fingerprint → palette offset, pan direction, speed multiplier
/// - note onset → shooting star (pitch sets vertical position and colour)
pub fn audio_milkyway(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();
        s.frame = s.frame.wrapping_add(1);
        let t = s.frame as f32;

        // ── Init stars on first call ──
        if !s.initialized {
            s.initialized = true;
            for _ in 0..NUM_STARS {
                let r_x = s.randf();
                let z = s.randn();
                let r_kind = s.randf();
                let r_a = s.randf();
                let r_b = s.randf();
                let r_phase = s.randf();

                let x_galaxy = r_x * COLS as f32;
                let y = (GALACTIC_PLANE + z * 0.95).clamp(0.0, (ROWS - 1) as f32);

                // Stellar population mix: fewer pure whites, more coloured
                // stars so the strip doesn't read as a wash of bright pixels.
                let (hue, sat) = if r_kind < 0.30 {
                    // Cool-white (still tinted blue, never pure white)
                    (210.0 + r_a * 40.0, 0.30 + r_a * 0.20)
                } else if r_kind < 0.65 {
                    // Blue
                    (200.0 + r_a * 25.0, 0.55 + r_a * 0.20)
                } else if r_kind < 0.88 {
                    // Yellow (sun-like)
                    (40.0 + r_a * 15.0, 0.65 + r_a * 0.20)
                } else {
                    // Red giant
                    (5.0 + r_a * 15.0, 0.80)
                };
                let is_giant = r_kind >= 0.88 || (r_kind >= 0.78 && r_b > 0.75);
                let base_b = if is_giant {
                    0.35 + r_b * 0.30
                } else {
                    0.08 + r_b * 0.22
                };

                s.stars.push(Star {
                    x_galaxy,
                    y,
                    hue,
                    sat,
                    base_b,
                    twinkle_phase: r_phase * std::f32::consts::TAU,
                    twinkle_speed: 0.04 + r_b * 0.12,
                });
            }
        }

        // ── Unpack audio ──
        let hihat_f = a.hihat as f32 / 255.0;
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

        // ── Chord hue (slowly tints the palette) ──
        if a.chord_root < 12 {
            s.chord_hue_target = (NOTE_HUES[a.chord_root as usize] + s.palette_offset)
                .rem_euclid(360.0);
        }
        s.chord_hue = lerp_hue(s.chord_hue, s.chord_hue_target, 0.05);

        // ── Beat + idle breath (always-on heartbeat) ──
        let beat_pulse = ((1.0 - beat_phase) * std::f32::consts::TAU).cos() * 0.5 + 0.5;
        smooth(&mut s.beat_brightness, beat_pulse, 0.4, 0.15);
        let idle_breath = ((t * 0.04).sin() * 0.5 + 0.5) * 0.3;

        // ── Kick → flash + shake + supernova ──
        let kick_onset = a.kick > s.prev_kick.saturating_add(25);
        s.prev_kick = a.kick;
        if kick_onset {
            s.kick_flash = 1.0;
            let rx = s.randf();
            let ry = s.randf();
            s.shake_x = (rx - 0.5) * 1.5;
            s.shake_y = (ry - 0.5) * 0.35;
            if !s.stars.is_empty() {
                let idx = (s.rand() as usize) % s.stars.len();
                s.nova_idx = Some(idx);
                s.nova_t = 1.0;
            }
        }
        s.kick_flash *= 0.85;
        s.nova_t *= 0.88;
        s.shake_x *= 0.7;
        s.shake_y *= 0.7;
        if s.nova_t < 0.05 {
            s.nova_idx = None;
        }

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

        // ── Note onset → shooting star (pitch sets y and colour) ──
        if a.note_midi > 0 && a.note_midi != s.prev_note_midi && s.prev_note_midi > 0 {
            if s.shooting.len() < MAX_SHOOTING {
                let n = ((a.note_midi as f32 - 36.0) / 48.0).clamp(0.0, 1.0);
                let r_dir = s.randf();
                let r_speed = s.randf();
                let r_y = s.randf();
                let dir = if r_dir > 0.5 { 1.0 } else { -1.0 };
                let start_x = if dir > 0.0 { -3.0 } else { COLS as f32 + 3.0 };
                let speed = 1.5 + r_speed * 1.6;
                let y = (GALACTIC_PLANE + (n - 0.5) * 4.5 + (r_y - 0.5) * 0.6)
                    .clamp(0.3, (ROWS - 1) as f32 - 0.3);
                let vy = (r_y - 0.5) * 0.08;
                s.shooting.push(ShootingStar {
                    x: start_x,
                    y,
                    vx: speed * dir,
                    vy,
                    life: 0.0,
                    hue: 200.0 + n * 60.0,
                    brightness: 0.7 + r_speed * 0.3,
                });
            }
        }
        if a.note_midi > 0 {
            s.prev_note_midi = a.note_midi;
        }

        // ── Subtle vertical wobble (camera drift) ──
        let drift_y = (t * 0.007).sin() * 0.35 + (t * 0.0103).cos() * 0.20;
        s.target_cy = GALACTIC_PLANE + drift_y;
        s.cy += (s.target_cy - s.cy) * 0.04;

        // ── Pan (apparent galactic rotation) ──
        let target_pan_speed = (0.05 + s.mid_smooth * 0.18 + vocals_f * 0.06 + s.bass_smooth * 0.04)
            * s.speed_mult
            * s.pan_direction;
        s.pan_speed += (target_pan_speed - s.pan_speed) * 0.04;
        s.pan = (s.pan + s.pan_speed).rem_euclid(COLS as f32);

        // ── Advance + cull shooting stars ──
        for ss in s.shooting.iter_mut() {
            ss.x += ss.vx;
            ss.y += ss.vy;
            ss.life += 1.0;
        }
        s.shooting.retain(|ss| {
            ss.life < 80.0
                && ss.x > -8.0
                && ss.x < (COLS as f32 + 8.0)
                && ss.y > -2.0
                && ss.y < (ROWS as f32 + 2.0)
        });

        // ══════════════════════════════════════════════
        // RENDER
        // ══════════════════════════════════════════════
        strip.set_all([0, 0, 0, 0]);

        let cx = COLS as f32 / 2.0 + s.shake_x;
        let cy = s.cy + s.shake_y;

        // Bulge size / intensity
        let bulge_r = BULGE_RADIUS_BASE
            + s.bass_smooth * 2.5
            + s.beat_brightness * 1.0
            + idle_breath * 0.6;
        let bulge_intensity = 0.38 + s.bass_smooth * 0.25 + s.beat_brightness * 0.12;

        // Palette: cool blue disc + warm orange bulge + reddish dust lane.
        // chord_hue subtly tints all three (±30° via palette_offset).
        let chord_shift = (s.chord_hue - 200.0) * 0.15;
        let disc_hue = (220.0 + chord_shift).rem_euclid(360.0);
        let bulge_hue = (45.0 + chord_shift).rem_euclid(360.0);
        let lane_hue = (15.0 + chord_shift).rem_euclid(360.0);

        let inv_2_sigx2 = 1.0 / (2.0 * DISC_SIGMA_X * DISC_SIGMA_X);
        let inv_2_sigy2 = 1.0 / (2.0 * DISC_SIGMA_Y * DISC_SIGMA_Y);
        let inv_2_lane2 = 1.0 / (2.0 * LANE_SIGMA * LANE_SIGMA);

        // ── Disc + bulge + dust lane (per-pixel background) ──
        for row in 0..ROWS {
            for col in 0..COLS {
                let dx = col as f32 - cx;
                let dy = row as f32 - cy;

                // Disc envelope
                let gx = (-(dx * dx) * inv_2_sigx2).exp();
                let gy = (-(dy * dy) * inv_2_sigy2).exp();
                let disc_b = gx * gy;

                // Dust lane darkens the central bright region along the plane
                let lane_factor = (-(dy * dy) * inv_2_lane2).exp();
                let lane_dark = 1.0 - lane_factor * gx * 0.75;

                let body_b = disc_b * lane_dark * 0.32;
                if body_b > 0.02 {
                    let c = hsv_to_rgb(disc_hue, 0.55, body_b.min(1.0));
                    add_color(strip, row * COLS + col, c);
                }

                // Bulge (vertically squashed)
                let dy_b = dy * BULGE_DY_SQUASH;
                let r_bulge = (dx * dx + dy_b * dy_b).sqrt();
                if r_bulge < bulge_r {
                    let bf = (1.0 - r_bulge / bulge_r).powi(2);
                    let bb = bf * bulge_intensity;
                    let c = hsv_to_rgb(bulge_hue, 0.7, bb.min(1.0));
                    add_color(strip, row * COLS + col, c);
                }

                // Subtle reddish glow inside the dust lane (where there's disc to tint)
                if lane_factor > 0.3 && disc_b > 0.08 {
                    let lt = (lane_factor - 0.3) / 0.7 * disc_b * 0.20;
                    let c = hsv_to_rgb(lane_hue, 0.7, lt);
                    add_color(strip, row * COLS + col, c);
                }
            }
        }

        // ── Stars ──
        for (i, star) in s.stars.iter().enumerate() {
            let screen_x = (star.x_galaxy + s.pan).rem_euclid(COLS as f32);
            let x = screen_x + s.shake_x;
            let y = star.y + s.shake_y;

            let tw = (star.twinkle_phase + t * star.twinkle_speed).sin() * 0.35 + 0.65;
            let mut b = star.base_b * tw * (0.5 + energy * 0.4);
            b *= 1.0 + s.treble_smooth * 0.3 + hihat_f * 0.25;

            // Supernova flash on this star, if selected
            if Some(i) == s.nova_idx {
                b += s.nova_t * 0.85;
            }

            if b < 0.02 {
                continue;
            }
            splat(strip, x, y, star.hue, star.sat, b.min(1.0));
        }

        // ── Shooting stars ──
        for ss in s.shooting.iter() {
            let life_frac = ss.life / 80.0;
            let alpha = if life_frac < 0.12 {
                life_frac / 0.12
            } else if life_frac > 0.70 {
                ((1.0 - life_frac) / 0.30).max(0.0)
            } else {
                1.0
            };
            let b = ss.brightness * alpha;
            if b < 0.04 {
                continue;
            }

            splat(strip, ss.x, ss.y, ss.hue, 0.25, b);
            // Trail behind the head
            for k in 1..=5 {
                let kf = k as f32;
                let tx = ss.x - ss.vx * 0.45 * kf;
                let ty = ss.y - ss.vy * 0.45 * kf;
                let fade = (1.0 - kf / 6.0).max(0.0);
                splat(
                    strip,
                    tx,
                    ty,
                    ss.hue,
                    (0.30 + kf * 0.05).min(0.6),
                    b * fade * fade * 0.65,
                );
            }
        }

        // ── Kick flash overlay (very subtle, chord-tinted) ──
        if s.kick_flash > 0.05 {
            let flash_b = s.kick_flash * 0.08;
            let c = hsv_to_rgb(disc_hue, 0.40, flash_b);
            for i in 0..NUM_LEDS {
                add_color(strip, i, c);
            }
        }
    });
}

fn splat(strip: &mut LedStrip, x: f32, y: f32, hue: f32, sat: f32, b: f32) {
    if b < 0.01 {
        return;
    }
    let bx = x.floor();
    let by = y.floor();
    let fx = x - bx;
    let fy = y - by;
    let bxi = bx as i32;
    let byi = by as i32;
    for (dy_off, wy) in [(0i32, 1.0 - fy), (1i32, fy)] {
        let row = byi + dy_off;
        if row < 0 || row >= ROWS as i32 {
            continue;
        }
        for (dx_off, wx) in [(0i32, 1.0 - fx), (1i32, fx)] {
            let col = bxi + dx_off;
            if col < 0 || col >= COLS as i32 {
                continue;
            }
            let w = wx * wy;
            if w < 0.02 {
                continue;
            }
            let bb = (w * b).min(1.0);
            if bb < 0.01 {
                continue;
            }
            let c = hsv_to_rgb(hue, sat, bb);
            add_color(strip, row as usize * COLS + col as usize, c);
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
