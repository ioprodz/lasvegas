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
const MAX_BLOCKS: usize = 6;
const CY: f32 = ROWS as f32 * 0.5 - 0.5; // 2.5

// Rhythm scaffold (reused from Strobubble V3): MOVE-MOVE-MOVE-FLIP per bar.
// MOVE_END = 0.75 (3 of 4 beats). MOVE: 3 vertical sin half-cycles.
// FLIP: half-revolution orbit around pair midpoint. BOUNCE_AMPLITUDE = orbit
// radius (half_dist) so dy/dt magnitudes match at the seams (the A=R trick).
const BEATS_PER_BAR: usize = 4;
const MOVE_END: f32 = 0.75;
const MOVE_HALF_CYCLES: f32 = 3.0;
const FRAMES_PER_BEAT_NOMINAL: f32 = 15.0;
const BAR_T_PER_FRAME: f32 = 1.0 / (FRAMES_PER_BEAT_NOMINAL * BEATS_PER_BAR as f32);

#[derive(Clone, Copy)]
enum OrbState {
    Floating,
    Exploding(f32),
}

#[derive(Clone, Copy)]
struct Orb {
    block: u32, // 0..block_count
    side: u32,  // 0 or 1 (pair member)
    x: f32,
    y: f32,
    hue: f32,
    state: OrbState,
    born: f32,
}

struct State {
    initialized: bool,
    frame: usize,
    seed: u32,

    orbs: Vec<Orb>,
    bar_t: f32,
    swapped: bool,

    // Smoothed audio
    bass_smooth: f32,
    mid_smooth: f32,
    high_smooth: f32,
    energy: f32,
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

    // Layout (from chord fingerprint)
    block_count: usize,
    block_styles: [u8; MAX_BLOCKS],
    block_reversed: [bool; MAX_BLOCKS],

    // Plasma wash
    plasma_phase: f32,
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

        // Block count: 2 / 3 / 4
        let raw = (fp & 0xFF) as usize % 3;
        self.block_count = [2, 3, 4][raw];

        // Per-block style: 3 visual flavours
        for i in 0..MAX_BLOCKS {
            let shift = 8 + i * 4;
            self.block_styles[i] = (((fp >> shift) & 0x0F) % 3) as u8;
        }
        // Mirror styles across the centre
        let n = self.block_count;
        for i in 0..n / 2 {
            self.block_styles[n - 1 - i] = self.block_styles[i];
        }

        // Per-block reversal — flips the eff_swap, so mirrored blocks
        // counter-pulse against their partners.
        for i in 0..MAX_BLOCKS {
            self.block_reversed[i] = ((fp >> (48 + i)) & 1) == 1;
        }
        for i in 0..n / 2 {
            self.block_reversed[n - 1 - i] = !self.block_reversed[i];
        }

        self.palette_offset = ((fp >> 32) & 0xFF) as f32 / 255.0 * 330.0;
    }

    fn slot_hue(block: u32, side: u32, chord_hue: f32, total_blocks: usize) -> f32 {
        let base = (chord_hue + block as f32 * (360.0 / total_blocks.max(1) as f32))
            .rem_euclid(360.0);
        if side == 0 {
            base
        } else {
            (base + 180.0).rem_euclid(360.0)
        }
    }

    fn ensure_orbs_filled(&mut self, chord_hue: f32, bar_t: f32, swapped: bool) {
        let n = self.block_count;
        for block in 0..n as u32 {
            let block_swap = swapped ^ self.block_reversed[block as usize];
            for side in 0..2u32 {
                let has_floating = self.orbs.iter().any(|o| {
                    o.block == block && o.side == side && matches!(o.state, OrbState::Floating)
                });
                if has_floating {
                    continue;
                }
                let inherit = self
                    .orbs
                    .iter()
                    .find(|o| {
                        o.block == block
                            && o.side == side
                            && matches!(o.state, OrbState::Exploding(_))
                    })
                    .map(|o| (o.x, o.y));
                let (sx, sy) =
                    inherit.unwrap_or_else(|| pattern_target(block, side, n, bar_t, block_swap));
                self.orbs.push(Orb {
                    block,
                    side,
                    x: sx,
                    y: sy,
                    hue: Self::slot_hue(block, side, chord_hue, n),
                    state: OrbState::Floating,
                    born: 0.0,
                });
            }
        }
    }

    fn refresh_hues(&mut self, chord_hue: f32) {
        let n = self.block_count;
        for o in self.orbs.iter_mut() {
            if matches!(o.state, OrbState::Floating) {
                o.hue = Self::slot_hue(o.block, o.side, chord_hue, n);
            }
        }
    }

    fn explode_random_orb(&mut self) {
        if self.orbs.is_empty() {
            return;
        }
        for _ in 0..8 {
            let idx = (self.rand() as usize) % self.orbs.len();
            let o = self.orbs[idx];
            if matches!(o.state, OrbState::Floating) && o.born > 0.55 {
                self.orbs[idx].state = OrbState::Exploding(0.0);
                return;
            }
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State {
        initialized: false,
        frame: 0,
        seed: 0xC0DEC0FE_u32,
        orbs: Vec::new(),
        bar_t: 0.0,
        swapped: false,
        bass_smooth: 0.0,
        mid_smooth: 0.0,
        high_smooth: 0.0,
        energy: 0.0,
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
        block_count: 3,
        block_styles: [0, 1, 2, 0, 1, 2],
        block_reversed: [false, true, false, true, false, true],
        plasma_phase: 0.0,
    });
}

/// Geometric target for orb (block, side) at bar phase `bar_t`.
/// `swapped` here is the per-block effective swap (global ^ block_reversed),
/// so block-reversed blocks run a half-bar out of phase with their partners.
///
/// Per bar:
///   • [0, 0.75)  MOVE — vertical sin-bounce, 3 half-cycles. Adjacent orbs
///                       (by global horizontal index) bounce in opposite
///                       directions.
///   • [0.75, 1)  FLIP — pair members orbit their shared midpoint via a
///                       half-revolution. Orbit radius = bounce amplitude
///                       (A = R), so dy/dt magnitudes match at the seams.
///                       FLIP direction inverts with `swapped` to keep the
///                       sign right in both swap states.
fn pattern_target(
    block: u32,
    side: u32,
    total_blocks: usize,
    bar_t: f32,
    swapped: bool,
) -> (f32, f32) {
    let block_width = COLS as f32 / total_blocks.max(1) as f32;
    let midpoint_x = (block as f32 + 0.5) * block_width;
    let half_dist = block_width * 0.25;
    let amplitude = half_dist; // A = R for velocity continuity

    let eff_side = if swapped { 1.0 - side as f32 } else { side as f32 };
    let inner = if eff_side == 0.0 { half_dist } else { -half_dist };

    // Global horizontal index → adjacent orbs get opposite vertical signs.
    let h_idx = 2 * block + (1 - side);
    let h_sign: f32 = if h_idx % 2 == 0 { 1.0 } else { -1.0 };

    if bar_t < MOVE_END {
        let p = bar_t / MOVE_END;
        let phase = p * MOVE_HALF_CYCLES * std::f32::consts::PI;
        let y_off = amplitude * h_sign * phase.sin();
        (midpoint_x + inner, CY + y_off)
    } else {
        let p = (bar_t - MOVE_END) / (1.0 - MOVE_END);
        let start_angle = if eff_side == 0.0 {
            0.0
        } else {
            std::f32::consts::PI
        };
        let dir: f32 = if swapped { -1.0 } else { 1.0 };
        let angle = start_angle + p * std::f32::consts::PI * dir;
        (
            midpoint_x + half_dist * angle.cos(),
            CY + half_dist * angle.sin(),
        )
    }
}

/// Harmonic Rhythm — chord-fingerprint block-partitioned strip (à la
/// Harmonic V4/V5) where every block runs the same Strobubble V3 rhythm
/// scaffold. 2–4 blocks (mirrored), each holding one orb pair locked to
/// MOVE·MOVE·MOVE·FLIP. Mirrored blocks counter-pulse via `block_reversed`.
/// Per-block visual style (disc / ring / diamond) chosen by fingerprint.
/// Subtle plasma wash beneath the rhythm-locked orbs.
pub fn audio_harmonic_rhythm(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();
        s.frame = s.frame.wrapping_add(1);

        if !s.initialized {
            s.initialized = true;
            let chord_hue = s.chord_hue;
            let bar_t = s.bar_t;
            let swapped = s.swapped;
            s.ensure_orbs_filled(chord_hue, bar_t, swapped);
            for o in s.orbs.iter_mut() {
                o.born = 1.0;
            }
        }

        // ── Audio unpack ──
        let bass_f = ((a.bands[0] as u16 + a.bands[1] as u16) / 2) as f32 / 255.0;
        let mid_f = ((a.bands[3] as u16 + a.bands[4] as u16) / 2) as f32 / 255.0;
        let high_f =
            ((a.bands[5] as u16 + a.bands[6] as u16 + a.bands[7] as u16) / 3) as f32 / 255.0;
        let beat_phase = a.beat_phase as f32 / 255.0;

        smooth(&mut s.bass_smooth, bass_f, 0.4, 0.08);
        smooth(&mut s.mid_smooth, mid_f, 0.35, 0.08);
        smooth(&mut s.high_smooth, high_f, 0.3, 0.10);
        smooth(
            &mut s.energy,
            a.bands.iter().map(|&b| b as f32).sum::<f32>() / (255.0 * 8.0),
            0.3,
            0.05,
        );
        let beat_pulse = ((1.0 - beat_phase) * std::f32::consts::TAU).cos() * 0.5 + 0.5;
        smooth(&mut s.beat_brightness, beat_pulse, 0.4, 0.15);

        // ── Onsets ──
        let kick_onset = a.kick > s.prev_kick.saturating_add(25);
        s.prev_kick = a.kick;
        if kick_onset {
            s.kick_t = 1.0;
            s.explode_random_orb();
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

        // ── Bar phase advance ──
        let speed_mult = 1.0 + s.mid_smooth * 0.40 + s.bass_smooth * 0.20;
        let new_bar_t = s.bar_t + BAR_T_PER_FRAME * speed_mult;
        if new_bar_t >= 1.0 {
            s.swapped = !s.swapped;
        }
        s.bar_t = new_bar_t.rem_euclid(1.0);

        // ── Plasma phase ──
        s.plasma_phase += 0.020 + s.mid_smooth * 0.025;

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
                    // Layout changed — drop orbs from blocks no longer in use.
                    let n = s.block_count as u32;
                    s.orbs.retain(|o| o.block < n);
                }
            }
        }

        // ── Refill any missing orbs and refresh hues ──
        let chord_hue = s.chord_hue;
        let bar_t = s.bar_t;
        let swapped = s.swapped;
        s.ensure_orbs_filled(chord_hue, bar_t, swapped);
        s.refresh_hues(chord_hue);

        // ── Lerp orbs toward continuous targets ──
        let lerp_factor: f32 = 0.45;
        let n = s.block_count;
        for o in s.orbs.iter_mut() {
            if let OrbState::Floating = o.state {
                let block_swap = swapped ^ s.block_reversed[o.block as usize];
                let (tx, ty) = pattern_target(o.block, o.side, n, bar_t, block_swap);
                o.x += (tx - o.x) * lerp_factor;
                o.y += (ty - o.y) * lerp_factor;
                if o.born < 1.0 {
                    o.born = (o.born + 0.06).min(1.0);
                }
            }
            if let OrbState::Exploding(p) = o.state {
                o.state = OrbState::Exploding(p + 0.12);
            }
        }
        s.orbs.retain(|o| match o.state {
            OrbState::Exploding(p) => p < 1.0,
            OrbState::Floating => true,
        });

        // ══════════════════════════════════════════════
        // RENDER
        // ══════════════════════════════════════════════
        strip.set_all([0, 0, 0, 0]);

        // ── Background plasma wash (subtle, beneath the orbs) ──
        let plasma_phase = s.plasma_phase;
        let bg_energy = s.energy.max(0.25);
        for row in 0..ROWS {
            for col in 0..COLS {
                let nx = col as f32 / COLS as f32;
                let ny = row as f32 / ROWS as f32;
                let v1 = (nx * 6.0 + plasma_phase).sin();
                let v2 = (ny * 4.0 - plasma_phase * 0.7).sin();
                let v3 = ((nx + ny) * 5.0 + plasma_phase * 0.5).sin();
                let p = (v1 + v2 + v3) / 6.0 + 0.5; // 0..1
                let b = 0.05 + p * 0.07 * bg_energy;
                let h = (chord_hue + p * 50.0 + 200.0) % 360.0;
                let c = hsv_to_rgb(h, 0.55, b);
                let idx = row * COLS + col;
                strip.set(idx, c);
            }
        }

        // ── Per-block orb render (style-dependent) ──
        let block_brightness = 0.78 + s.beat_brightness * 0.15 + s.kick_t * 0.10;

        for o in s.orbs.iter() {
            let style = s.block_styles[o.block as usize];
            let block_width = COLS as f32 / s.block_count.max(1) as f32;
            let orb_radius = (block_width * 0.18).clamp(1.6, 3.0);

            match o.state {
                OrbState::Floating => {
                    let r = orb_radius * o.born;
                    if r < 0.15 {
                        continue;
                    }
                    match style {
                        0 => draw_circle(strip, o.x, o.y, r, o.hue, 0.88, block_brightness),
                        1 => draw_ring(strip, o.x, o.y, r, 1.0, o.hue, 0.88, block_brightness),
                        2 => draw_diamond(strip, o.x, o.y, r * 1.15, o.hue, 0.88, block_brightness),
                        _ => draw_circle(strip, o.x, o.y, r, o.hue, 0.88, block_brightness),
                    }
                }
                OrbState::Exploding(p) => {
                    let pc = p.clamp(0.0, 1.0);
                    let ring_r = orb_radius * (1.0 + pc * 2.0);
                    let ring_b = (1.0 - pc).powi(2);
                    draw_ring(strip, o.x, o.y, ring_r, 1.1, o.hue, 0.88, ring_b);
                }
            }
        }

        // ── Hihat coloured micro-strobe (max-blend) ──
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

        // ── White kick flash (lerp to white) ──
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

// ── Drawing helpers ──────────────────────────────────────────

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

/// 4-point soft diamond — Manhattan distance with row stretched to match
/// column aspect, solid core out to 60% then linear taper.
fn draw_diamond(
    strip: &mut LedStrip,
    cx: f32,
    cy: f32,
    radius: f32,
    hue: f32,
    sat: f32,
    brightness: f32,
) {
    if radius < 0.2 || brightness < 0.02 {
        return;
    }
    let x_min = ((cx - radius).floor() as i32).max(0);
    let x_max = ((cx + radius).ceil() as i32).min(COLS as i32 - 1);
    let y_min = ((cy - radius).floor() as i32).max(0);
    let y_max = ((cy + radius).ceil() as i32).min(ROWS as i32 - 1);
    let core_r = radius * 0.6;
    for row in y_min..=y_max {
        for col in x_min..=x_max {
            let dx = (col as f32 - cx).abs();
            let dy = (row as f32 - cy).abs() * 1.6; // stretch y for aspect
            let d = dx + dy;
            if d > radius {
                continue;
            }
            let fade = if d <= core_r {
                1.0
            } else {
                (1.0 - (d - core_r) / (radius - core_r)).max(0.0)
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
