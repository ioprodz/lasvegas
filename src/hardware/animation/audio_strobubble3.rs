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

// 4 pairs of bubbles → 8 slots evenly spaced across the strip.
// Pair midpoints at columns 7.5, 22.5, 37.5, 52.5 (15-col spacing).
// HALF_DIST = COLS / NUM_SLOTS / 2 = 60/8/2 = 3.75 → all 8 bubble centres
// sit 7.5 cols apart (3.75, 11.25, 18.75, 26.25, …), so spacing reads as
// uniform — no visible 2-by-2 grouping.
const NUM_PAIRS: usize = 4;
const NUM_SLOTS: usize = NUM_PAIRS * 2;
const BUBBLE_RADIUS: f32 = 3.0;
const HALF_DIST: f32 = 3.75; // half the uniform bubble spacing
const CY: f32 = ROWS as f32 * 0.5 - 0.5; // 2.5

// Rhythm per bar: MOVE, MOVE, MOVE, FLIP.
//   • MOVE (beats 1–3, first 3/4 of bar) — vertical bounce. Three half-
//          cycles of sin-oscillation: bubble passes through rest at every
//          beat boundary, peaking at each half-beat. Adjacent bubbles (by
//          horizontal order) move in OPPOSITE directions: one rises while
//          its neighbour falls.
//   • FLIP (beat 4, last 1/4 of bar) — pair members orbit their shared
//          midpoint, one full revolution in a single beat. Quick spin
//          accent that punctuates the bar. Members return to their start
//          positions, so the next bar's MOVE is continuous.
//
// MOVE ends at sin(3π) = 0, putting bubbles back at the centre row; FLIP
// starts and ends at (cos=1/-1, sin=0), the same rest position — so the
// whole bar is geometrically continuous, no seams.
const BEATS_PER_BAR: usize = 4;
const MOVE_END: f32 = 0.75; // 3 of 4 beats
// MOVE vertical amplitude equals the FLIP orbit radius (HALF_DIST). This is
// what makes MOVE→FLIP and FLIP→MOVE velocity-continuous: at the seam, both
// motions have d/dt scaled by HALF_DIST so the sin- and cos-derivative
// magnitudes match. (Bubbles dip ~1.25 cols off the strip at MOVE peaks —
// same as during FLIP's vertical excursions.)
const BOUNCE_AMPLITUDE: f32 = HALF_DIST;
const MOVE_HALF_CYCLES: f32 = 3.0; // one half-cycle per MOVE beat

// Per-frame advance of bar_t. Targets ~120 BPM (15 frames per beat at
// 30 fps). bar_t is fraction of bar (0..1, wraps every bar). Audio energy
// modulates the rate slightly so it breathes with the music.
const FRAMES_PER_BEAT_NOMINAL: f32 = 15.0;
const BAR_T_PER_FRAME: f32 = 1.0 / (FRAMES_PER_BEAT_NOMINAL * BEATS_PER_BAR as f32);

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
    hue: f32,
    state: BubbleState,
    born_progress: f32,
}

struct State {
    initialized: bool,
    frame: usize,
    seed: u32,

    bubbles: Vec<Bubble>,

    // Bar phase (0..1, wraps every bar)
    bar_t: f32,
    // Flipped each bar: tracks which "side" each slot currently occupies.
    // false = original (side 0 right, side 1 left); true = swapped.
    swapped: bool,

    // Audio
    bass_smooth: f32,
    mid_smooth: f32,
    beat_brightness: f32,

    // Onsets
    prev_kick: u8,
    prev_snare: u8,
    prev_hihat: u8,
    kick_t: f32,
    hihat_t: f32,

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

    fn slot_hue(slot: u32, chord_hue: f32) -> f32 {
        // Headliner SWAP palette: each pair gets a base hue spaced around
        // the wheel; the two members of a pair are complementary (+180°).
        let pair = slot / 2;
        let side = slot % 2;
        let base = (chord_hue + pair as f32 * (360.0 / NUM_PAIRS as f32)).rem_euclid(360.0);
        if side == 0 { base } else { (base + 180.0).rem_euclid(360.0) }
    }

    fn spawn_at_slot(&mut self, slot: u32, x: f32, y: f32, chord_hue: f32) {
        self.bubbles.push(Bubble {
            slot,
            x,
            y,
            hue: Self::slot_hue(slot, chord_hue),
            state: BubbleState::Floating,
            born_progress: 0.0,
        });
    }

    fn ensure_slots_filled(&mut self, chord_hue: f32, bar_t: f32, swapped: bool) {
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
            let (sx, sy) = inherit.unwrap_or_else(|| pattern_target(slot, bar_t, swapped));
            self.spawn_at_slot(slot, sx, sy, chord_hue);
        }
    }

    fn refresh_hues(&mut self, chord_hue: f32) {
        for b in self.bubbles.iter_mut() {
            if matches!(b.state, BubbleState::Floating) {
                b.hue = Self::slot_hue(b.slot, chord_hue);
            }
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
        bar_t: 0.0,
        swapped: false,
        bass_smooth: 0.0,
        mid_smooth: 0.0,
        beat_brightness: 0.0,
        prev_kick: 0,
        prev_snare: 0,
        prev_hihat: 0,
        kick_t: 0.0,
        hihat_t: 0.0,
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

/// Geometric target position for a given slot at bar position `bar_t` ∈ [0,1).
/// `swapped` flips every bar — it tracks the accumulated half-revolutions
/// from past FLIPs, so MOVE always starts the bubble at its current side
/// (no jump at bar boundaries).
///
/// Each bar:
///   • [0,        MOVE_END): MOVE — vertical sin-oscillation, 3 half-cycles
///                           across 3 beats. Adjacent bubbles move opposite.
///                           Passes through CY at each beat boundary.
///   • [MOVE_END,        1]: FLIP — pair members trade places via a half-
///                           revolution orbit (1 beat). One goes over the
///                           top, its partner under the bottom — a real
///                           swap. `swapped` toggles at bar wrap.
fn pattern_target(slot: u32, bar_t: f32, swapped: bool) -> (f32, f32) {
    let pair = (slot / 2) as f32;
    let side = (slot % 2) as f32; // 0 = first member, 1 = second
    let midpoint_x = (pair + 0.5) * (COLS as f32 / NUM_PAIRS as f32); // 7.5, 22.5, 37.5, 52.5
    // Effective side after the accumulated swap. eff_side tells you which
    // physical position the bubble currently occupies.
    let eff_side = if swapped { 1.0 - side } else { side };
    let inner = if eff_side == 0.0 { HALF_DIST } else { -HALF_DIST };
    // h_idx = 2*pair + (1 - side) → bubbles in left-to-right horizontal order 0..7.
    let h_idx = (2.0 * pair) + (1.0 - side);
    let h_sign = if (h_idx as i32) % 2 == 0 { 1.0 } else { -1.0 };

    if bar_t < MOVE_END {
        // MOVE: y_offset = amplitude · h_sign · sin(move_progress · 3π).
        // Three sin half-cycles → bubbles pass through rest (sin=0) at the
        // start/end of each beat, peaking/troughing mid-beat. Adjacent
        // bubbles have flipped sign so they pulse opposite each other.
        let move_progress = bar_t / MOVE_END;
        let phase = move_progress * MOVE_HALF_CYCLES * std::f32::consts::PI;
        let y_offset = BOUNCE_AMPLITUDE * h_sign * phase.sin();
        (midpoint_x + inner, CY + y_offset)
    } else {
        // FLIP: half-revolution orbit. Starts from the bubble's current
        // (eff_side) angle and rotates by ±π. The direction reverses with
        // `swapped` so that the orbit's initial vertical velocity matches
        // MOVE's exit velocity in BOTH states — no glitch at the seam.
        // (End position is the same either way; only the arc taken differs:
        // unswapped bars flip via the top, swapped bars via the bottom.)
        let flip_progress = (bar_t - MOVE_END) / (1.0 - MOVE_END); // 0..1
        let start_angle = if eff_side == 0.0 { 0.0 } else { std::f32::consts::PI };
        let direction = if swapped { -1.0 } else { 1.0 };
        let angle = start_angle + flip_progress * std::f32::consts::PI * direction;
        (
            midpoint_x + HALF_DIST * angle.cos(),
            CY + HALF_DIST * angle.sin(),
        )
    }
}

/// Strobubble V3 — 4 pairs of Headliner-style circles, rhythm-locked
/// move-move-move-flip motion. Each pair is a hue + its +180° complement.
///
/// Per bar (4 beats):  MOVE · MOVE · MOVE · FLIP
///   • MOVE (3 beats) — vertical sin-bounce. Adjacent bubbles (by
///                      horizontal order) move in OPPOSITE directions; one
///                      rises while its neighbour falls, three times.
///   • FLIP (1 beat)  — pair members spin once around their shared
///                      midpoint. Quick punctuation accent that returns
///                      them to start, so the next bar is continuous.
///
/// Kicks explode a random bubble (replaced from inherited position).
/// Headliner-style strobe overlays: white kick flash + hihat coloured
/// micro-strobe. Black canvas, no background wash.
pub fn audio_strobubble3(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();
        s.frame = s.frame.wrapping_add(1);

        // ── Init: pre-fill all slots fully visible ──
        if !s.initialized {
            s.initialized = true;
            let chord_hue = s.chord_hue;
            let bar_t = s.bar_t;
            let swapped = s.swapped;
            s.ensure_slots_filled(chord_hue, bar_t, swapped);
            for b in s.bubbles.iter_mut() {
                b.born_progress = 1.0;
            }
        }

        // ── Unpack audio ──
        let bass_f = ((a.bands[0] as u16 + a.bands[1] as u16) / 2) as f32 / 255.0;
        let mid_f = ((a.bands[3] as u16 + a.bands[4] as u16) / 2) as f32 / 255.0;
        let beat_phase = a.beat_phase as f32 / 255.0;

        smooth(&mut s.bass_smooth, bass_f, 0.4, 0.08);
        smooth(&mut s.mid_smooth, mid_f, 0.35, 0.08);
        let beat_pulse = ((1.0 - beat_phase) * std::f32::consts::TAU).cos() * 0.5 + 0.5;
        smooth(&mut s.beat_brightness, beat_pulse, 0.4, 0.15);

        // ── Onsets ──
        let kick_onset = a.kick > s.prev_kick.saturating_add(25);
        s.prev_kick = a.kick;
        if kick_onset {
            s.kick_t = 1.0;
            s.explode_random_bubble();
        }
        s.kick_t *= 0.60;

        let snare_onset = a.snare > s.prev_snare.saturating_add(30);
        s.prev_snare = a.snare;
        if snare_onset {
            s.kick_t = s.kick_t.max(0.55);
        }

        let hihat_onset = a.hihat > s.prev_hihat.saturating_add(35);
        s.prev_hihat = a.hihat;
        if hihat_onset {
            s.hihat_t = 1.0;
        }
        s.hihat_t *= 0.45;

        // ── Bar phase advance. bar_t wraps 0..1 every bar (4 beats at
        //    ~120 BPM nominal). Audio energy modulates the rate slightly.
        //    Toggle `swapped` exactly when bar_t wraps — accumulates the
        //    half-revolution from each FLIP so MOVE begins seamlessly.
        let speed_mult = 1.0 + s.mid_smooth * 0.40 + s.bass_smooth * 0.20;
        let new_bar_t = s.bar_t + BAR_T_PER_FRAME * speed_mult;
        if new_bar_t >= 1.0 {
            s.swapped = !s.swapped;
        }
        s.bar_t = new_bar_t.rem_euclid(1.0);

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

        // ── Refill empty slots and keep hues tracking the live chord ──
        let chord_hue = s.chord_hue;
        let bar_t = s.bar_t;
        let swapped = s.swapped;
        s.ensure_slots_filled(chord_hue, bar_t, swapped);
        s.refresh_hues(chord_hue);

        // ── Lerp bubbles toward their (continuous) targets. MOVE→FLIP and
        //    FLIP→MOVE seams are geometrically continuous, so this lerp is
        //    mostly for explosion-respawn handoffs.
        let lerp_factor: f32 = 0.45;
        for b in s.bubbles.iter_mut() {
            if let BubbleState::Floating = b.state {
                let (tx, ty) = pattern_target(b.slot, bar_t, swapped);
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
        s.bubbles.retain(|b| match b.state {
            BubbleState::Exploding(p) => p < 1.0,
            BubbleState::Floating => true,
        });

        // ══════════════════════════════════════════════
        // RENDER — black canvas (no background wash)
        // ══════════════════════════════════════════════
        strip.set_all([0, 0, 0, 0]);

        let brightness = 0.78 + s.beat_brightness * 0.15 + s.kick_t * 0.10;

        for b in s.bubbles.iter() {
            match b.state {
                BubbleState::Floating => {
                    let effective_r = BUBBLE_RADIUS * b.born_progress;
                    if effective_r < 0.15 {
                        continue;
                    }
                    draw_circle(strip, b.x, b.y, effective_r, b.hue, 0.88, brightness);
                }
                BubbleState::Exploding(p) => {
                    let pc = p.clamp(0.0, 1.0);
                    let ring_r = BUBBLE_RADIUS * (1.0 + pc * 2.0);
                    let ring_b = (1.0 - pc).powi(2);
                    draw_ring(strip, b.x, b.y, ring_r, 1.1, b.hue, 0.88, ring_b);
                }
            }
        }

        // ── Hihat coloured micro-strobe (Headliner-style additive accent) ──
        if s.hihat_t > 0.12 {
            let strobe_hue = (chord_hue + 60.0) % 360.0;
            let strobe_b = s.hihat_t * 0.45;
            let c = hsv_to_rgb(strobe_hue, 0.70, strobe_b);
            let leds = strip.controller_leds();
            for i in 0..NUM_LEDS {
                let e = leds[i];
                leds[i] = [e[0].max(c[0]), e[1].max(c[1]), e[2].max(c[2]), 0];
            }
        }

        // ── White kick flash (Headliner-style) — fires on every detected kick ──
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

/// Soft-edge disc — solid core out to 70% of the radius, then a linear taper
/// to the edge. Identical to Headliner's draw_circle: adjacent radius-3
/// circles spaced 6 cols apart touch crisply. cy/cx may sit outside the
/// strip; off-grid pixels are simply clipped.
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
                [base[0].max(c[0]), base[1].max(c[1]), base[2].max(c[2]), 0],
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
                [base[0].max(c[0]), base[1].max(c[1]), base[2].max(c[2]), 0],
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
