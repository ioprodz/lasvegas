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

// Fixed bubble count — bubbles live in numbered slots; the target position
// is a deterministic geometric function of (slot, pattern, time). Bubbles
// lerp toward the target each frame so transitions and pattern switches
// are smooth.
const NUM_SLOTS: usize = 6;
const BUBBLE_RADIUS: f32 = 3.5;
const CY: f32 = ROWS as f32 * 0.5 - 0.5; // 2.5

// Two modes alternating bar-by-bar:
//   CIRCLE — pair members orbit their midpoint (the "pattern switch").
//            Step π/2 per beat → full rotation in one bar.
//   SWING  — the whole pair slides left/right as a rigid unit ("back,
//            forth, back, forth"). Step π per beat → 4 swap events per bar.
// Pattern switches every 4 beats (1 bar), with an 8s watchdog fallback.
const NUM_PATTERNS: usize = 2;
const PATTERN_CIRCLE: usize = 0;
const PATTERN_SWING: usize = 1;
const PATTERN_WATCHDOG_FRAMES: usize = 240;

const BEATS_PER_BAR: usize = 4;
const NUM_PAIRS: usize = NUM_SLOTS / 2; // 3 pairs of 2 bubbles each

fn pattern_step_size(pattern: usize) -> f32 {
    match pattern {
        PATTERN_CIRCLE => std::f32::consts::PI * 0.5, // 4 beats = full rotation
        PATTERN_SWING => std::f32::consts::PI,         // 4 beats = "back forth back forth"
        _ => std::f32::consts::PI * 0.5,
    }
}

#[derive(Clone, Copy)]
enum BubbleState {
    Floating,
    Exploding(f32),
}

#[derive(Clone, Copy)]
struct Bubble {
    slot: u32,
    x: f32,
    y: f32,
    radius: f32,
    hue: f32,
    sat: f32,
    state: BubbleState,
    born_progress: f32,
}

struct State {
    initialized: bool,
    frame: usize,
    seed: u32,

    bubbles: Vec<Bubble>,

    // Pattern
    pattern: usize,
    pattern_t: f32,
    pattern_watchdog: usize,
    prev_beat_phase: u8,
    beat_count: usize,

    // Pitch → speed
    prev_note_midi: u8,
    pitch_var: f32,

    // Audio
    energy: f32,
    bass_smooth: f32,
    mid_smooth: f32,
    beat_brightness: f32,

    // Strobe
    prev_kick: u8,
    prev_snare: u8,
    kick_t: f32,
    strobe_hue: f32,

    // Two-state bubble colour: flips on every beat. color_t lerps smoothly
    // between 0 (state A) and 1 (state B) over the time between beats.
    color_state: bool,
    color_t: f32,

    // Chord
    chord_hue: f32,
    chord_hue_target: f32,
    palette_offset: f32,
    prev_chord_root: u8,
    chord_history: [u8; PROGRESSION_LEN],
    chord_count: usize,
    chord_stable_frames: usize,
    fingerprint: u64,
}

impl State {
    fn rand(&mut self) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed
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
    }

    fn spawn_at_slot(&mut self, slot: u32, x: f32, y: f32, chord_hue: f32) {
        // Each slot is painted with a fixed offset on the colour wheel —
        // 360°/NUM_SLOTS apart — over the current chord hue. Uniform palette.
        let hue =
            (chord_hue + slot as f32 * (360.0 / NUM_SLOTS as f32)).rem_euclid(360.0);
        self.bubbles.push(Bubble {
            slot,
            x,
            y,
            radius: BUBBLE_RADIUS,
            hue,
            sat: 0.88,
            state: BubbleState::Floating,
            born_progress: 0.0,
        });
    }

    /// Ensure every slot has a Floating bubble. If a slot is currently empty
    /// (eg. its previous bubble just exploded), spawn a replacement. We
    /// inherit the exploded bubble's position so the new one fades in
    /// from where the old one was — smooth handoff, no jump.
    fn ensure_slots_filled(&mut self, chord_hue: f32, pattern: usize, t: f32) {
        for slot in 0..NUM_SLOTS as u32 {
            let has_floating = self
                .bubbles
                .iter()
                .any(|b| b.slot == slot && matches!(b.state, BubbleState::Floating));
            if has_floating {
                continue;
            }
            let inherit = self
                .bubbles
                .iter()
                .find(|b| b.slot == slot && matches!(b.state, BubbleState::Exploding(_)))
                .map(|b| (b.x, b.y));
            let (sx, sy) = inherit.unwrap_or_else(|| pattern_target(pattern, slot, t));
            self.spawn_at_slot(slot, sx, sy, chord_hue);
        }
    }

    fn explode_random_bubble(&mut self) {
        if self.bubbles.is_empty() {
            return;
        }
        for _ in 0..8 {
            let idx = (self.rand() as usize) % self.bubbles.len();
            let b = self.bubbles[idx];
            if matches!(b.state, BubbleState::Floating) && b.born_progress > 0.55 {
                self.bubbles[idx].state = BubbleState::Exploding(0.0);
                return;
            }
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State {
        initialized: false,
        frame: 0,
        seed: 0xB0BB1E5_u32,
        bubbles: Vec::new(),
        pattern: PATTERN_CIRCLE,
        pattern_t: 0.0,
        pattern_watchdog: 0,
        prev_beat_phase: 0,
        beat_count: 0,
        prev_note_midi: 0,
        pitch_var: 0.0,
        energy: 0.0,
        bass_smooth: 0.0,
        mid_smooth: 0.0,
        beat_brightness: 0.0,
        prev_kick: 0,
        prev_snare: 0,
        kick_t: 0.0,
        strobe_hue: 0.0,
        color_state: false,
        color_t: 0.0,
        chord_hue: 200.0,
        chord_hue_target: 200.0,
        palette_offset: 0.0,
        prev_chord_root: 255,
        chord_history: [255; PROGRESSION_LEN],
        chord_count: 0,
        chord_stable_frames: 0,
        fingerprint: 0,
    });
}

/// Geometric target position for a given slot under a given pattern at time t.
/// 6 bubbles = 3 pairs; each pair shares a fixed midpoint at x = 10, 30, 50
/// on the centre row.
fn pattern_target(pattern: usize, slot: u32, t: f32) -> (f32, f32) {
    let pair = (slot / 2) as f32;
    let side = (slot % 2) as f32; // 0 = first member, 1 = second
    let n_pairs = NUM_PAIRS as f32;
    let midpoint_x = (pair + 0.5) * (COLS as f32 / n_pairs); // 10, 30, 50
    let midpoint_y = CY;

    match pattern {
        // Rotating 12-sided polygon (flat ellipse: wide x, narrow y).
        // ── CIRCLE ── pair members orbit their shared midpoint at
        //    radius 4 (y squashed so vertical reach stays inside the strip).
        //    side 0 sits at angle t; side 1 sits at angle t + π (opposite).
        //    Step π/2 per beat → 4 beats = 1 full revolution.
        PATTERN_CIRCLE => {
            let angle = t + side * std::f32::consts::PI;
            let r = 4.0;
            let aspect_y = 0.55;
            (
                midpoint_x + r * angle.cos(),
                midpoint_y + r * angle.sin() * aspect_y,
            )
        }
        // ── SWING ── the WHOLE pair slides left/right as a rigid unit.
        //    Internal pair structure (members ±5 from pair-centre) is kept;
        //    the pair-centre itself shifts by 4·cos(t). Step π per beat
        //    → 4 swap-direction events per bar = "back forth back forth".
        PATTERN_SWING => {
            let pair_shift = 4.0 * t.cos();
            let inner = if side == 0.0 { 5.0 } else { -5.0 };
            (midpoint_x + pair_shift + inner, midpoint_y)
        }
        _ => (midpoint_x, midpoint_y),
    }
}

/// Strobubble V2 — 6 bubbles in 3 pairs at fixed midpoints (x = 10, 30, 50)
/// alternating between two motions every bar:
///   • CIRCLE — pair members orbit their midpoint (the "pattern switch":
///     bubbles circle around each other). 4 beats = 1 full revolution.
///   • SWING  — the WHOLE pair slides left/right as a rigid unit
///     ("back, forth, back, forth"). 4 swap-direction events per bar.
/// Plus carried over from V1: kick/snare strobe with rotating colour,
/// chord palette, explosion-on-kick, two-state colour transition flipping
/// each beat.
pub fn audio_strobubble2(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();
        s.frame = s.frame.wrapping_add(1);

        // ── Init: pre-fill all 12 slots fully visible ──
        if !s.initialized {
            s.initialized = true;
            let chord_hue = s.chord_hue;
            let pattern = s.pattern;
            let pt = s.pattern_t;
            s.ensure_slots_filled(chord_hue, pattern, pt);
            for b in s.bubbles.iter_mut() {
                b.born_progress = 1.0;
            }
        }

        // ── Unpack audio ──
        let bass_f = ((a.bands[0] as u16 + a.bands[1] as u16) / 2) as f32 / 255.0;
        let mid_f = ((a.bands[3] as u16 + a.bands[4] as u16) / 2) as f32 / 255.0;
        let _high_f =
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
        let beat_pulse = ((1.0 - beat_phase) * std::f32::consts::TAU).cos() * 0.5 + 0.5;
        smooth(&mut s.beat_brightness, beat_pulse, 0.4, 0.15);

        // ── Onsets / strobe ──
        let kick_onset = a.kick > s.prev_kick.saturating_add(20);
        s.prev_kick = a.kick;
        if kick_onset {
            s.kick_t = 1.0;
            s.strobe_hue = (s.strobe_hue + 47.0).rem_euclid(360.0);
            s.explode_random_bubble();
        }
        let snare_onset = a.snare > s.prev_snare.saturating_add(28);
        s.prev_snare = a.snare;
        if snare_onset {
            s.kick_t = s.kick_t.max(0.9);
            s.strobe_hue = (s.strobe_hue + 47.0).rem_euclid(360.0);
        }
        s.kick_t *= 0.60;

        // ── Beat onset (BPM-derived) → step the geometry forward by π/2
        //    and ring a mild strobe backstop. pattern_t is the ONLY way the
        //    targets change, and it only changes here — so between beats
        //    targets are frozen and bubbles settle.
        let curr_phase = a.beat_phase;
        let beat_onset = (s.prev_beat_phase as i16) > 180 && (curr_phase as i16) < 60;
        s.prev_beat_phase = curr_phase;
        if beat_onset {
            s.beat_count += 1;
            s.kick_t = s.kick_t.max(0.55);
            // Step size is per-pattern: π/2 for CIRCLE (full revolution per
            // bar), π for SWING (back-forth-back-forth across 4 beats).
            s.pattern_t += pattern_step_size(s.pattern);
            // Flip the colour state — bubbles will lerp toward the other colour.
            s.color_state = !s.color_state;
        }

        // Advance color_t toward its target each frame (0.0 if state false,
        // 1.0 if state true). Lerp rate ~0.18 → reaches target in ~10 frames
        // (≈ one beat at 120 BPM), giving a visible transition between beats.
        let color_target = if s.color_state { 1.0 } else { 0.0 };
        s.color_t += (color_target - s.color_t) * 0.18;

        // ── Pitch variation tracker (still updated, now drives the
        //    between-beats colour/radius wobble rather than motion speed) ──
        if a.note_midi > 0 && s.prev_note_midi > 0 && a.note_midi != s.prev_note_midi {
            let diff = (a.note_midi as i16 - s.prev_note_midi as i16).unsigned_abs() as f32;
            s.pitch_var = (s.pitch_var + diff * 0.06).min(1.0);
        }
        if a.note_midi > 0 {
            s.prev_note_midi = a.note_midi;
        }
        s.pitch_var *= 0.982;

        // ── Pattern cycling (every BEATS_PER_BAR = 4 beats = 1 bar). On
        //    switch we reset pattern_t to 0 so the new pattern starts at
        //    its initial geometric pose.
        s.pattern_watchdog += 1;
        let want_switch = (beat_onset && s.beat_count >= BEATS_PER_BAR)
            || s.pattern_watchdog >= PATTERN_WATCHDOG_FRAMES;
        if want_switch {
            s.pattern = (s.pattern + 1) % NUM_PATTERNS;
            s.pattern_t = 0.0;
            s.pattern_watchdog = 0;
            s.beat_count = 0;
        }

        // ── Chord tracking ──
        if a.chord_root < 12 {
            s.chord_hue_target = (NOTE_HUES[a.chord_root as usize] + s.palette_offset) % 360.0;
        }
        s.chord_hue = lerp_hue(s.chord_hue, s.chord_hue_target, 0.08);

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

        // ── Refill empty slots (after explosions) ──
        let chord_hue = s.chord_hue;
        let pattern = s.pattern;
        let pt = s.pattern_t;
        s.ensure_slots_filled(chord_hue, pattern, pt);

        // (Dance pairing is intentionally OFF in V2 — bubbles must "stay
        // still in between" beats; constant orbital motion would break that.)

        // ── Lerp bubbles toward their (now-frozen-between-beats) target.
        //    A high factor lets them reach the target within ~5 frames after
        //    a beat hits, then they sit still until the next beat.
        let lerp_factor: f32 = 0.40;
        for b in s.bubbles.iter_mut() {
            if let BubbleState::Floating = b.state {
                let (tx, ty) = pattern_target(pattern, b.slot, pt);
                b.x += (tx - b.x) * lerp_factor;
                b.y += (ty - b.y) * lerp_factor;
                if b.born_progress < 1.0 {
                    b.born_progress = (b.born_progress + 0.06).min(1.0);
                }
            }
            if let BubbleState::Exploding(p) = b.state {
                b.state = BubbleState::Exploding(p + 0.12);
            }
        }

        // ── Cull only finished explosions; floating bubbles live as long as
        //    their slot exists (i.e. always).
        s.bubbles.retain(|b| match b.state {
            BubbleState::Exploding(p) => p < 1.0,
            BubbleState::Floating => true,
        });

        // ══════════════════════════════════════════════
        // RENDER
        // ══════════════════════════════════════════════
        strip.set_all([0, 0, 0, 0]);

        // Between-beats "alive" oscillation: radius breathes with bass/beat
        // and colour lerps smoothly between each bubble's two states.
        let global_pulse = 0.80 + s.beat_brightness * 0.15 + s.bass_smooth * 0.05;
        let radius_pulse = (1.0 + s.bass_smooth * 0.18 + s.beat_brightness * 0.08).min(1.30);
        let color_t = s.color_t;

        for b in s.bubbles.iter() {
            match b.state {
                BubbleState::Floating => {
                    let effective_r = b.radius * b.born_progress * radius_pulse;
                    if effective_r < 0.15 {
                        continue;
                    }
                    // Two-state colour: A = bubble's base hue, B = its
                    // complement (+180°). color_t smoothly crosses 0→1→0→…
                    // on every beat, lerping between A and B as it goes.
                    let color_a = b.hue;
                    let color_b = (b.hue + 180.0).rem_euclid(360.0);
                    let hue = lerp_hue(color_a, color_b, color_t);
                    draw_circle(strip, b.x, b.y, effective_r, hue, b.sat, global_pulse);
                }
                BubbleState::Exploding(p) => {
                    let pc = p.clamp(0.0, 1.0);
                    let ring_r = b.radius * (1.0 + pc * 2.0);
                    let brightness = (1.0 - pc).powi(2);
                    draw_ring(strip, b.x, b.y, ring_r, 1.1, b.hue, b.sat, brightness);
                }
            }
        }

        // ── Strobe overlay (coloured, rotates on kick/snare) ──
        if s.kick_t > 0.15 {
            let flash_b = s.kick_t;
            let strobe = hsv_to_rgb(s.strobe_hue, 0.92, 1.0);
            let sr = strobe[0] as f32;
            let sg = strobe[1] as f32;
            let sb = strobe[2] as f32;
            let leds = strip.controller_leds();
            for i in 0..NUM_LEDS {
                let e = leds[i];
                let r = (e[0] as f32 * (1.0 - flash_b) + sr * flash_b) as u8;
                let g = (e[1] as f32 * (1.0 - flash_b) + sg * flash_b) as u8;
                let bl = (e[2] as f32 * (1.0 - flash_b) + sb * flash_b) as u8;
                leds[i] = [r, g, bl, 0];
            }
        }
    });
}

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
    let core_r = radius * 0.7;
    for row in y_min..=y_max {
        for col in x_min..=x_max {
            let dx = col as f32 - cx;
            let dy = row as f32 - cy;
            let r = (dx * dx + dy * dy).sqrt();
            if r > radius {
                continue;
            }
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

fn draw_ring(
    strip: &mut LedStrip,
    cx: f32,
    cy: f32,
    ring_r: f32,
    thickness: f32,
    hue: f32,
    sat: f32,
    brightness: f32,
) {
    if brightness < 0.03 {
        return;
    }
    let bounds = ring_r + thickness * 0.6;
    let x_min = ((cx - bounds).floor() as i32).max(0);
    let x_max = ((cx + bounds).ceil() as i32).min(COLS as i32 - 1);
    let y_min = ((cy - bounds).floor() as i32).max(0);
    let y_max = ((cy + bounds).ceil() as i32).min(ROWS as i32 - 1);
    let half_t = thickness * 0.5;
    for row in y_min..=y_max {
        for col in x_min..=x_max {
            let dx = col as f32 - cx;
            let dy = row as f32 - cy;
            let r = (dx * dx + dy * dy).sqrt();
            let dist_from_ring = (r - ring_r).abs();
            if dist_from_ring > half_t {
                continue;
            }
            let fade = 1.0 - dist_from_ring / half_t;
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
