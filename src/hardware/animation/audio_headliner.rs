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

// Cue rotation. Frame-based, not beat-based — predictable cadence even
// when the BPM detector loses lock.
const CUE_FRAMES: usize = 180; // ~6s at 30fps
const NUM_CUES: usize = 7;

const CUE_STROBE: usize = 0;
const CUE_FLOOD: usize = 1;
const CUE_BEAMS: usize = 2;
const CUE_POPS: usize = 3;
const CUE_CHASE: usize = 4;
const CUE_CIRCLES_BLINK: usize = 5;
const CUE_CIRCLES_SWAP: usize = 6;

const MAX_POPS: usize = 32;

// 10 evenly-spaced circle centres at columns 3, 9, 15, …, 57 (spacing 6)
const NUM_CIRCLES: usize = 10;
const CIRCLE_SPACING: f32 = COLS as f32 / NUM_CIRCLES as f32; // = 6.0
// 5 pairs of orbiting circles, midpoints at columns 6, 18, 30, 42, 54
const NUM_PAIRS: usize = 5;
const PAIR_SPACING: f32 = COLS as f32 / NUM_PAIRS as f32; // = 12.0

#[derive(Clone, Copy)]
struct Pop {
    col: u8,
    row: u8,
    life: f32, // 1.0 fresh, 0.0 dead
    hue: f32,
}

struct HeadlinerState {
    frame: usize,
    seed: u32,

    cue: usize,
    cue_order: [usize; NUM_CUES],
    cue_slot: usize,
    cue_frame: usize,

    // Audio
    energy: f32,
    bass_smooth: f32,
    mid_smooth: f32,
    treble_smooth: f32,
    beat_brightness: f32,

    // Onset trackers
    prev_kick: u8,
    kick_t: f32,
    kick_count: usize,
    prev_hihat: u8,
    hihat_t: f32,
    prev_snare: u8,
    snare_t: f32,

    // Cue-specific persistent state
    chase_a: f32,
    chase_b: f32,
    pops: Vec<Pop>,
    gradient_phase: f32,

    // Circles state
    circles_current_set: u8,   // 0 = even circles lit, 1 = odd circles lit
    circles_transition: f32,   // 0.0 just after a kick, ramps to 1.0
    circles_theta: f32,        // orbit angle for the SWAP cue

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
}

impl HeadlinerState {
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
        self.speed_mult = 0.85 + ((fp >> 40) & 0xFF) as f32 / 255.0 * 0.5;
        // Shuffle cue order so each chord progression yields a different setlist
        let mut order = [0, 1, 2, 3, 4, 5, 6];
        for i in 0..NUM_CUES {
            let j = ((fp >> (i * 4)) & 0x0F) as usize % NUM_CUES;
            order.swap(i, j);
        }
        self.cue_order = order;
    }
}

thread_local! {
    static STATE: RefCell<HeadlinerState> = RefCell::new(HeadlinerState {
        frame: 0,
        seed: 0xDEADBEEF,
        cue: CUE_STROBE,
        cue_order: [
            CUE_STROBE,
            CUE_FLOOD,
            CUE_BEAMS,
            CUE_POPS,
            CUE_CHASE,
            CUE_CIRCLES_BLINK,
            CUE_CIRCLES_SWAP,
        ],
        cue_slot: 0,
        cue_frame: 0,
        energy: 0.0,
        bass_smooth: 0.0,
        mid_smooth: 0.0,
        treble_smooth: 0.0,
        beat_brightness: 0.0,
        prev_kick: 0,
        kick_t: 0.0,
        kick_count: 0,
        prev_hihat: 0,
        hihat_t: 0.0,
        prev_snare: 0,
        snare_t: 0.0,
        chase_a: 0.0,
        chase_b: (COLS - 1) as f32,
        pops: Vec::new(),
        gradient_phase: 0.0,
        circles_current_set: 0,
        circles_transition: 1.0,
        circles_theta: 0.0,
        chord_hue: 0.0,
        chord_hue_target: 0.0,
        palette_offset: 0.0,
        speed_mult: 1.0,
        prev_chord_root: 255,
        chord_history: [255; PROGRESSION_LEN],
        chord_count: 0,
        chord_stable_frames: 0,
        fingerprint: 0,
    });
}

/// Headliner — a programmed stage-light show.
///
/// Three always-on layers run underneath every cue:
///   • Background wash (very dim, chord-tinted, breathing)
///   • Kick white-flash overlay (full-strip snap on every kick)
///   • Hihat micro-strobe (sub-beat colour accent on every hihat)
///
/// On top, a movement layer cycles through 6 cues every ~6 seconds (frame-
/// based, predictable). Cue order is shuffled per chord-progression
/// fingerprint so each progression yields a different setlist.
pub fn audio_headliner(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();
        s.frame = s.frame.wrapping_add(1);
        s.cue_frame = s.cue_frame.wrapping_add(1);
        let f = s.frame;
        let _t = f as f32;

        // ── Unpack audio ──
        let bass_f = ((a.bands[0] as u16 + a.bands[1] as u16) / 2) as f32 / 255.0;
        let mid_f = ((a.bands[3] as u16 + a.bands[4] as u16) / 2) as f32 / 255.0;
        let high_f =
            ((a.bands[5] as u16 + a.bands[6] as u16 + a.bands[7] as u16) / 3) as f32 / 255.0;
        let beat_phase = a.beat_phase as f32 / 255.0;

        smooth(
            &mut s.energy,
            a.bands.iter().map(|&b| b as f32).sum::<f32>() / (255.0 * 8.0),
            0.3,
            0.05,
        );
        smooth(&mut s.bass_smooth, bass_f, 0.4, 0.08);
        smooth(&mut s.mid_smooth, mid_f, 0.35, 0.08);
        smooth(&mut s.treble_smooth, high_f, 0.3, 0.1);
        let _energy = s.energy.max(0.10);

        let beat_pulse = ((1.0 - beat_phase) * std::f32::consts::TAU).cos() * 0.5 + 0.5;
        smooth(&mut s.beat_brightness, beat_pulse, 0.4, 0.15);

        // ── Onset detection ──
        let kick_onset = a.kick > s.prev_kick.saturating_add(25);
        s.prev_kick = a.kick;
        if kick_onset {
            s.kick_t = 1.0;
            s.kick_count += 1;
        }
        s.kick_t *= 0.60; // fast decay → real "snap"

        let hihat_onset = a.hihat > s.prev_hihat.saturating_add(35);
        s.prev_hihat = a.hihat;
        if hihat_onset {
            s.hihat_t = 1.0;
        }
        s.hihat_t *= 0.45;

        let snare_onset = a.snare > s.prev_snare.saturating_add(30);
        s.prev_snare = a.snare;
        if snare_onset {
            s.snare_t = 1.0;
        }
        s.snare_t *= 0.55;

        // ── Chord tracking ──
        if a.chord_root < 12 {
            s.chord_hue_target = (NOTE_HUES[a.chord_root as usize] + s.palette_offset) % 360.0;
        }
        s.chord_hue = lerp_hue(s.chord_hue, s.chord_hue_target, 0.10);

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

        // ── Cue advance (frame-based, predictable) ──
        if s.cue_frame >= CUE_FRAMES {
            s.cue_frame = 0;
            s.cue_slot = (s.cue_slot + 1) % NUM_CUES;
            s.cue = s.cue_order[s.cue_slot];
            // Reset per-cue state
            s.chase_a = 0.0;
            s.chase_b = (COLS - 1) as f32;
            s.gradient_phase = 0.0;
            s.circles_transition = 1.0;
            s.circles_theta = 0.0;
        }

        let chord_hue = s.chord_hue;

        // Clear to black — the canvas every cue paints onto.
        strip.set_all([0, 0, 0, 0]);

        // ════════════════════════════════════════════════════
        // LAYER 1: Background wash (skip for the circles cues so they
        // stand alone on black). Never below ~12% so real LEDs reliably
        // light up — anything below that just looks off.
        // ════════════════════════════════════════════════════
        let want_bg = !matches!(s.cue, CUE_CIRCLES_BLINK | CUE_CIRCLES_SWAP);
        if want_bg {
            let bg_b = (0.12 + s.bass_smooth * 0.10 + s.beat_brightness * 0.04).min(0.28);
            let bg = hsv_to_rgb(chord_hue, 0.90, bg_b);
            for i in 0..NUM_LEDS {
                strip.set(i, bg);
            }
        }

        // ════════════════════════════════════════════════════
        // LAYER 2: Cue-specific movement
        // ════════════════════════════════════════════════════
        match s.cue {
            // ─────────────────────────────────────────
            // STROBE — frame-locked rapid flash. Real strobe rate.
            // ─────────────────────────────────────────
            CUE_STROBE => {
                // 15 Hz strobe at 30fps: 1 frame on, 1 frame off.
                // We modulate intensity with energy so quiet sections still
                // have a slow flicker (every 4 frames) and loud sections
                // hammer (every 2 frames).
                let period = if s.bass_smooth > 0.35 { 2 } else if s.energy > 0.25 { 3 } else { 5 };
                let on = (f % period) < (period / 2).max(1);
                if on {
                    // Colour cycles every 4 strobe cycles for variety.
                    let cycle = (f / period / 2) as f32;
                    let hue = (chord_hue + cycle * 47.0) % 360.0;
                    let c = hsv_to_rgb(hue, 0.85, 0.85);
                    for i in 0..NUM_LEDS {
                        strip.set(i, c);
                    }
                }
            }

            // ─────────────────────────────────────────
            // FLOOD — flowing rainbow gradient + kick pulses
            // ─────────────────────────────────────────
            CUE_FLOOD => {
                s.gradient_phase += 0.6 + s.mid_smooth * 1.4;
                let span = 90.0; // 90° spread across the strip
                let pulse = 0.25 + s.bass_smooth * 0.45 + s.beat_brightness * 0.20;
                for col in 0..COLS {
                    let hue = (chord_hue + col as f32 * span / COLS as f32 + s.gradient_phase)
                        % 360.0;
                    let c = hsv_to_rgb(hue, 0.85, pulse.min(1.0));
                    for row in 0..ROWS {
                        strip.set(row * COLS + col, c);
                    }
                }
            }

            // ─────────────────────────────────────────
            // BEAMS — two beams sweeping in opposing sinusoids; bright at intersection
            // ─────────────────────────────────────────
            CUE_BEAMS => {
                let cf = s.cue_frame as f32;
                let speed_a = 0.045 * s.speed_mult * (1.0 + s.mid_smooth * 0.6);
                let speed_b = 0.038 * s.speed_mult * (1.0 + s.mid_smooth * 0.6);
                let pos_a = COLS as f32 * 0.5 + (cf * speed_a).sin() * (COLS as f32 * 0.45);
                let pos_b = COLS as f32 * 0.5 + (cf * speed_b + 2.1).cos() * (COLS as f32 * 0.45);
                let hue_a = chord_hue;
                let hue_b = (chord_hue + 150.0) % 360.0;
                let half_width = 3.0;

                draw_beam(strip, pos_a, hue_a, half_width, 0.85);
                draw_beam(strip, pos_b, hue_b, half_width, 0.85);

                // Bright intersection point
                let dist = (pos_a - pos_b).abs();
                if dist < 5.0 {
                    let mix = 1.0 - dist / 5.0;
                    let intersect_x = (pos_a + pos_b) * 0.5;
                    draw_beam(strip, intersect_x, chord_hue, 2.5, mix * 1.0);
                    // Plus a tiny white snap at the centre of intersection
                    if mix > 0.5 {
                        let cx = intersect_x as i32;
                        for d in -1..=1 {
                            let col = cx + d;
                            if col < 0 || col >= COLS as i32 {
                                continue;
                            }
                            let c = hsv_to_rgb(chord_hue, 0.0, mix);
                            for row in 0..ROWS {
                                strip.set(row * COLS + col as usize, c);
                            }
                        }
                    }
                }
            }

            // ─────────────────────────────────────────
            // POPS — random bright pixels appear on every kick/snare, fade fast
            // ─────────────────────────────────────────
            CUE_POPS => {
                // Spawn on kicks (big burst) and snares (smaller burst)
                let mut n_spawn = 0;
                if kick_onset {
                    n_spawn += 8 + (s.bass_smooth * 8.0) as usize;
                }
                if snare_onset {
                    n_spawn += 4;
                }
                if hihat_onset {
                    n_spawn += 2;
                }
                for _ in 0..n_spawn {
                    if s.pops.len() >= MAX_POPS {
                        break;
                    }
                    let rc = s.randf();
                    let rr = s.randf();
                    let rh = s.randf();
                    s.pops.push(Pop {
                        col: (rc * COLS as f32) as u8,
                        row: (rr * ROWS as f32) as u8,
                        life: 1.0,
                        hue: (chord_hue + rh * 90.0 - 45.0).rem_euclid(360.0),
                    });
                }
                // Render + decay
                for pop in s.pops.iter_mut() {
                    let b = pop.life.powi(2) * 0.95;
                    let c = hsv_to_rgb(pop.hue, 0.80, b);
                    let idx = pop.row as usize * COLS + pop.col as usize;
                    if idx < NUM_LEDS {
                        strip.set(idx, c);
                    }
                    pop.life *= 0.78;
                }
                s.pops.retain(|p| p.life > 0.05);
            }

            // ─────────────────────────────────────────
            // CHASE — two bars criss-crossing, white snap at intersection
            // ─────────────────────────────────────────
            CUE_CHASE => {
                let speed = (1.0 + s.bass_smooth * 1.2 + s.mid_smooth * 0.8) * s.speed_mult;
                s.chase_a += speed;
                s.chase_b -= speed;
                if s.chase_a >= COLS as f32 {
                    s.chase_a = 0.0;
                }
                if s.chase_b < 0.0 {
                    s.chase_b = (COLS - 1) as f32;
                }
                let hue_a = chord_hue;
                let hue_b = (chord_hue + 120.0) % 360.0;

                // Bars with short trails
                for k in 0..6 {
                    let kf = k as f32;
                    let fade = (1.0 - kf / 6.0).max(0.0);
                    let b = fade * fade * 0.95;
                    let xa = (s.chase_a - kf).rem_euclid(COLS as f32);
                    let xb = (s.chase_b + kf).rem_euclid(COLS as f32);
                    let ca = hsv_to_rgb(hue_a, 0.85, b);
                    let cb = hsv_to_rgb(hue_b, 0.85, b);
                    let xa_i = xa as usize;
                    let xb_i = xb as usize;
                    for row in 0..ROWS {
                        // Bar A
                        let base = strip.controller_leds()[row * COLS + xa_i];
                        strip.set(
                            row * COLS + xa_i,
                            [
                                base[0].max(ca[0]),
                                base[1].max(ca[1]),
                                base[2].max(ca[2]),
                                0,
                            ],
                        );
                        // Bar B
                        let base = strip.controller_leds()[row * COLS + xb_i];
                        strip.set(
                            row * COLS + xb_i,
                            [
                                base[0].max(cb[0]),
                                base[1].max(cb[1]),
                                base[2].max(cb[2]),
                                0,
                            ],
                        );
                    }
                }
                // Intersection snap
                let dist = (s.chase_a - s.chase_b).abs();
                if dist < 2.0 {
                    let cx = ((s.chase_a + s.chase_b) * 0.5) as usize;
                    let c = hsv_to_rgb(chord_hue, 0.0, 1.0);
                    for d in 0..2 {
                        let col = cx + d;
                        if col >= COLS {
                            continue;
                        }
                        for row in 0..ROWS {
                            strip.set(row * COLS + col, c);
                        }
                    }
                }
            }

            // ─────────────────────────────────────────
            // CIRCLES_BLINK — 10 circles, alternating evens/odds, swap on every kick.
            // Smooth pop-in / pop-out transition over ~10 frames.
            // ─────────────────────────────────────────
            CUE_CIRCLES_BLINK => {
                if kick_onset {
                    s.circles_current_set ^= 1;
                    s.circles_transition = 0.0;
                }
                s.circles_transition = (s.circles_transition + 0.10).min(1.0);
                // Smooth-step the transition curve
                let tau = s.circles_transition;
                let smooth_t = tau * tau * (3.0 - 2.0 * tau);
                let new_scale = smooth_t;
                let old_scale = 1.0 - smooth_t;

                // Circles fill the 6×6 box exactly, no spacing between them.
                let max_radius = 3.0;
                let cy = ROWS as f32 / 2.0 - 0.5; // 2.5

                for i in 0..NUM_CIRCLES {
                    let in_current = (i % 2) as u8 == s.circles_current_set;
                    let scale = if in_current { new_scale } else { old_scale };
                    if scale < 0.04 {
                        continue;
                    }
                    let cx = (i as f32 + 0.5) * CIRCLE_SPACING; // 3, 9, 15, …, 57
                    // Spread chord palette across all 10 circles
                    let hue = (chord_hue + i as f32 * 36.0).rem_euclid(360.0);
                    let radius = max_radius * scale;
                    let brightness =
                        (0.70 + s.beat_brightness * 0.20 + s.kick_t * 0.10) * scale;
                    draw_circle(strip, cx, cy, radius, hue, 0.88, brightness);
                }
            }

            // ─────────────────────────────────────────
            // CIRCLES_SWAP — 5 pairs orbit around their fixed midpoints.
            // The point where each pair "touches" stays centred; the two
            // circles orbit it, dipping above/below the visible strip on
            // their way around (we see the central 6 rows of a 60×60 grid).
            // ─────────────────────────────────────────
            CUE_CIRCLES_SWAP => {
                let speed = (0.040 + s.mid_smooth * 0.075 + s.bass_smooth * 0.025)
                    * s.speed_mult;
                s.circles_theta += speed;
                let theta = s.circles_theta;
                let cos_t = theta.cos();
                let sin_t = theta.sin();
                // Half the distance between two circles in a pair.
                // Pair circles sit 6 cols apart, so half_dist = 3 px.
                let half_dist = 3.0_f32;
                let cy = ROWS as f32 / 2.0 - 0.5;
                // Circles fill the 6×6 box exactly; touching at the midpoint.
                let radius = 3.0_f32;
                let brightness = 0.78 + s.beat_brightness * 0.15 + s.kick_t * 0.10;

                for pair in 0..NUM_PAIRS {
                    let mid_x = (pair as f32 + 0.5) * PAIR_SPACING; // 6, 18, 30, 42, 54
                    let dx = half_dist * cos_t;
                    let dy = half_dist * sin_t;
                    let x_a = mid_x + dx;
                    let y_a = cy + dy;
                    let x_b = mid_x - dx;
                    let y_b = cy - dy;
                    let hue_a = (chord_hue + pair as f32 * 72.0).rem_euclid(360.0);
                    let hue_b = (hue_a + 180.0).rem_euclid(360.0);
                    draw_circle(strip, x_a, y_a, radius, hue_a, 0.85, brightness);
                    draw_circle(strip, x_b, y_b, radius, hue_b, 0.85, brightness);
                }
            }

            _ => {}
        }

        // ════════════════════════════════════════════════════
        // LAYER 3: Hihat micro-strobe (additive accent)
        // ════════════════════════════════════════════════════
        if s.hihat_t > 0.12 {
            let strobe_hue = (chord_hue + 60.0) % 360.0;
            let strobe_b = s.hihat_t * 0.45;
            let c = hsv_to_rgb(strobe_hue, 0.70, strobe_b);
            for i in 0..NUM_LEDS {
                add_color(strip, i, c);
            }
        }

        // ════════════════════════════════════════════════════
        // LAYER 4: Kick white-flash overlay (this is THE strobe)
        // Fires on every detected kick — completely reliable, never off-beat.
        // ════════════════════════════════════════════════════
        if s.kick_t > 0.15 {
            let flash_b = s.kick_t;
            let leds = strip.controller_leds();
            for i in 0..NUM_LEDS {
                let e = leds[i];
                let r = (e[0] as f32 * (1.0 - flash_b) + 255.0 * flash_b) as u8;
                let g = (e[1] as f32 * (1.0 - flash_b) + 255.0 * flash_b) as u8;
                let bl = (e[2] as f32 * (1.0 - flash_b) + 255.0 * flash_b) as u8;
                leds[i] = [r, g, bl, 0];
            }
        }
    });
}

/// Draw a soft-edge disc in real pixel units, max-blended into the strip.
/// cy can be outside [0, ROWS) — pixels outside the grid are simply clipped,
/// so circles can dip in and out of the visible 6-row window (treating the
/// strip as the central slice of a 60×60 virtual grid).
fn draw_circle(
    strip: &mut LedStrip,
    cx: f32,
    cy: f32,
    radius: f32,
    hue: f32,
    sat: f32,
    brightness: f32,
) {
    if radius < 0.15 || brightness < 0.02 {
        return;
    }
    let x_min = ((cx - radius).floor() as i32).max(0);
    let x_max = ((cx + radius).ceil() as i32).min(COLS as i32 - 1);
    let y_min = ((cy - radius).floor() as i32).max(0);
    let y_max = ((cy + radius).ceil() as i32).min(ROWS as i32 - 1);
    for row in y_min..=y_max {
        for col in x_min..=x_max {
            let dx = col as f32 - cx;
            let dy = row as f32 - cy;
            let r = (dx * dx + dy * dy).sqrt();
            if r > radius {
                continue;
            }
            // Solid core out to 70% radius, then quick linear taper to the edge.
            // Adjacent circles at radius=3 with centres 6 apart touch crisply.
            let core_r = radius * 0.7;
            let fade = if r <= core_r {
                1.0
            } else {
                (1.0 - (r - core_r) / (radius - core_r)).max(0.0)
            };
            let b = fade * brightness;
            if b < 0.03 {
                continue;
            }
            let c = hsv_to_rgb(hue, sat, b.min(1.0));
            let idx = row as usize * COLS + col as usize;
            let base = strip.controller_leds()[idx];
            strip.set(
                idx,
                [
                    base[0].max(c[0]),
                    base[1].max(c[1]),
                    base[2].max(c[2]),
                    0,
                ],
            );
        }
    }
}

fn draw_beam(strip: &mut LedStrip, x: f32, hue: f32, half_width: f32, peak_b: f32) {
    let x_min = ((x - half_width).floor() as i32).max(0);
    let x_max = ((x + half_width).ceil() as i32).min(COLS as i32 - 1);
    for col in x_min..=x_max {
        let dx = (col as f32 - x).abs();
        let fade = (1.0 - dx / half_width).max(0.0);
        let b = fade * fade * peak_b;
        if b < 0.04 {
            continue;
        }
        let c = hsv_to_rgb(hue, 0.85, b.min(1.0));
        for row in 0..ROWS {
            let idx = row * COLS + col as usize;
            let base = strip.controller_leds()[idx];
            strip.set(
                idx,
                [
                    base[0].max(c[0]),
                    base[1].max(c[1]),
                    base[2].max(c[2]),
                    0,
                ],
            );
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
