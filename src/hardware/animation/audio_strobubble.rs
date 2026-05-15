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

const MAX_BUBBLES: usize = 30;
const TARGET_BUBBLES: usize = 20;
const SPAWN_INTERVAL: usize = 8;

// Dance pairing — bubbles transiently lock into an orbit around their midpoint.
// MIN_DIST is the centre-to-centre distance at pair formation; the rotation
// preserves length, so this also becomes the orbit diameter. Set it large
// enough that even fat bubbles (radius ~3.5) keep clear of each other.
const DANCE_SPAWN_INTERVAL: usize = 22;
const DANCE_MIN_DIST: f32 = 7.0;
const DANCE_MAX_DIST: f32 = 13.0;
const DANCE_MIN_FRAMES: u32 = 50;
const DANCE_MAX_EXTRA_FRAMES: u32 = 65; // total range 50..115 frames

// Movement patterns, cycled every 4 beats (or every 8s as watchdog fallback).
const NUM_PATTERNS: usize = 5;
const PATTERN_DRIFT: usize = 0;
const PATTERN_STREAM: usize = 1;
const PATTERN_WAVE: usize = 2;
const PATTERN_ORBIT: usize = 3;
const PATTERN_CONVERGE: usize = 4;
const PATTERN_WATCHDOG_FRAMES: usize = 240;

#[derive(Clone, Copy)]
enum BubbleState {
    Floating,
    Exploding(f32), // progress 0..1 (ring expanding + fading)
}

#[derive(Clone, Copy)]
struct Bubble {
    id: u32, // stable identity (Vec indices shift when bubbles are removed)
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    radius: f32,
    hue: f32,
    sat: f32,
    state: BubbleState,
    born_progress: f32, // 0..1; 1.0 = fully grown
    dancing: bool,      // set each frame when this bubble's position came from a dance pair
}

#[derive(Clone, Copy)]
struct DancePair {
    a_id: u32,
    b_id: u32,
    remaining: u32,
}

struct State {
    initialized: bool,
    frame: usize,
    seed: u32,

    bubbles: Vec<Bubble>,
    spawn_timer: usize,
    next_bubble_id: u32,

    // Dance pairs (bubbles orbiting each other transiently)
    dance_pairs: Vec<DancePair>,
    dance_spawn_timer: usize,

    // Pattern cycling
    pattern: usize,
    pattern_watchdog: usize,
    prev_beat_phase: u8,
    beat_count: usize,
    stream_vx: f32,
    stream_vy: f32,

    // Pitch variation tracker (drives bubble speed)
    prev_note_midi: u8,
    pitch_var: f32,

    // Audio smoothing
    energy: f32,
    bass_smooth: f32,
    mid_smooth: f32,
    beat_brightness: f32,

    // Onset (main beat strobe)
    prev_kick: u8,
    prev_snare: u8,
    kick_t: f32,
    strobe_hue: f32, // rotates on every kick / snare so each flash lands on a different colour

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
    }

    /// Spawn a bubble that either drifts in from an edge (already at full
    /// radius) or is "born" on-screen (radius grows from 0). Either way,
    /// no in-view pop-in glitch.
    fn spawn_bubble(&mut self, chord_hue: f32) {
        if self.bubbles.len() >= MAX_BUBBLES {
            return;
        }
        let radius = 2.0 + self.randf() * 1.5; // 2.0 – 3.5
        let speed = 0.18 + self.randf() * 0.30; // 0.18 – 0.48 px/frame
        let hue_jitter = (self.randf() - 0.5) * 100.0;
        let hue = (chord_hue + hue_jitter).rem_euclid(360.0);
        let sat = 0.80 + self.randf() * 0.18;

        let edge_spawn = self.randf() < 0.65;
        let (x, y, vx, vy, born) = if edge_spawn {
            let r_edge = self.randf();
            let r_perp = self.randf();
            // Spawn slightly outside the visible area, with velocity pointing in.
            if r_edge < 0.40 {
                // Left → right
                let angle = (self.randf() - 0.5) * 0.6;
                (
                    -radius - 0.5,
                    r_perp * 9.0 - 1.5,
                    speed * angle.cos(),
                    speed * angle.sin(),
                    1.0,
                )
            } else if r_edge < 0.80 {
                // Right → left
                let angle = std::f32::consts::PI + (self.randf() - 0.5) * 0.6;
                (
                    COLS as f32 + radius + 0.5,
                    r_perp * 9.0 - 1.5,
                    speed * angle.cos(),
                    speed * angle.sin(),
                    1.0,
                )
            } else if r_edge < 0.90 {
                // Top → bottom (virtual)
                let angle = std::f32::consts::FRAC_PI_2 + (self.randf() - 0.5) * 0.6;
                (
                    r_perp * COLS as f32,
                    -radius - 0.5,
                    speed * angle.cos(),
                    speed * angle.sin(),
                    1.0,
                )
            } else {
                // Bottom → top (virtual)
                let angle = -std::f32::consts::FRAC_PI_2 + (self.randf() - 0.5) * 0.6;
                (
                    r_perp * COLS as f32,
                    ROWS as f32 + radius + 0.5,
                    speed * angle.cos(),
                    speed * angle.sin(),
                    1.0,
                )
            }
        } else {
            // Born on-screen: starts at radius 0, grows in over ~17 frames.
            let angle = self.randf() * std::f32::consts::TAU;
            (
                self.randf() * COLS as f32,
                self.randf() * ROWS as f32,
                speed * 0.6 * angle.cos(),
                speed * 0.6 * angle.sin(),
                0.0,
            )
        };

        let id = self.next_bubble_id;
        self.next_bubble_id = self.next_bubble_id.wrapping_add(1);
        self.bubbles.push(Bubble {
            id,
            x,
            y,
            vx,
            vy,
            radius,
            hue,
            sat,
            state: BubbleState::Floating,
            born_progress: born,
            dancing: false,
        });
    }

    /// Look for two fully-grown floating bubbles close to each other but
    /// not already in another pair, and lock them into an orbit around
    /// their midpoint for a short while. Picks the closest viable pair.
    fn try_create_dance_pair(&mut self) {
        let mut best: Option<(usize, usize, f32)> = None;
        for i in 0..self.bubbles.len() {
            let bi = &self.bubbles[i];
            if !matches!(bi.state, BubbleState::Floating) || bi.born_progress < 0.6 {
                continue;
            }
            let i_id = bi.id;
            if self
                .dance_pairs
                .iter()
                .any(|p| p.a_id == i_id || p.b_id == i_id)
            {
                continue;
            }
            for j in (i + 1)..self.bubbles.len() {
                let bj = &self.bubbles[j];
                if !matches!(bj.state, BubbleState::Floating) || bj.born_progress < 0.6 {
                    continue;
                }
                let j_id = bj.id;
                if self
                    .dance_pairs
                    .iter()
                    .any(|p| p.a_id == j_id || p.b_id == j_id)
                {
                    continue;
                }
                let dx = bj.x - bi.x;
                let dy = bj.y - bi.y;
                let r = (dx * dx + dy * dy).sqrt();
                if r >= DANCE_MIN_DIST && r <= DANCE_MAX_DIST {
                    if best.map(|(_, _, br)| r < br).unwrap_or(true) {
                        best = Some((i, j, r));
                    }
                }
            }
        }
        if let Some((i, j, _)) = best {
            let a_id = self.bubbles[i].id;
            let b_id = self.bubbles[j].id;
            let r = self.randf();
            let remaining = DANCE_MIN_FRAMES + (r * DANCE_MAX_EXTRA_FRAMES as f32) as u32;
            self.dance_pairs.push(DancePair {
                a_id,
                b_id,
                remaining,
            });
        }
    }

    /// Pick a fully-grown floating bubble at random and pop it.
    fn explode_random_bubble(&mut self) {
        if self.bubbles.is_empty() {
            return;
        }
        // Up to 8 attempts to find a viable target — cheap and avoids alloc.
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
        spawn_timer: 0,
        next_bubble_id: 1,
        dance_pairs: Vec::new(),
        dance_spawn_timer: 0,
        pattern: PATTERN_DRIFT,
        pattern_watchdog: 0,
        prev_beat_phase: 0,
        beat_count: 0,
        stream_vx: 0.30,
        stream_vy: 0.0,
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

/// Strobubble — drifting coloured bubbles on black, with a stage strobe
/// hammering overhead. Bubbles never pop in/out mid-view: they drift in
/// from edges (or are born on-screen growing from radius 0), float across,
/// and either drift out again or visibly explode (ring expanding outward
/// + fading) — typically triggered by kicks.
pub fn audio_strobubble(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();
        s.frame = s.frame.wrapping_add(1);
        let frame = s.frame;

        // ── First-frame init: pre-spawn a few bubbles so the strip is alive
        //    immediately rather than empty until the first spawn timer fires.
        if !s.initialized {
            s.initialized = true;
            let chord_hue = s.chord_hue;
            for _ in 0..5 {
                let rx = s.randf();
                let ry = s.randf();
                s.spawn_bubble(chord_hue);
                // Force them to be "already in flight" rather than off-edge
                if let Some(last) = s.bubbles.last_mut() {
                    last.x = 5.0 + rx * (COLS as f32 - 10.0);
                    last.y = 0.5 + ry * (ROWS as f32 - 1.0);
                    last.born_progress = 1.0;
                }
            }
        }

        // ── Unpack audio ──
        let _high_f =
            ((a.bands[5] as u16 + a.bands[6] as u16 + a.bands[7] as u16) / 3) as f32 / 255.0;
        let bass_f = ((a.bands[0] as u16 + a.bands[1] as u16) / 2) as f32 / 255.0;
        let mid_f = ((a.bands[3] as u16 + a.bands[4] as u16) / 2) as f32 / 255.0;
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

        // ── Onsets ──
        // Strobe fires on kicks (full intensity, pops a bubble), snares
        // (full intensity, no bubble change) and every BPM-detected beat
        // (milder pulse). Together these cover beats 1–4 in typical 4/4.
        let kick_onset = a.kick > s.prev_kick.saturating_add(20);
        s.prev_kick = a.kick;
        if kick_onset {
            s.kick_t = 1.0;
            s.strobe_hue = (s.strobe_hue + 47.0).rem_euclid(360.0);
            // Pop a bubble + spawn 1–3 fresh ones (so the strip stays dense)
            s.explode_random_bubble();
            let chord_hue = s.chord_hue;
            s.spawn_bubble(chord_hue);
            if s.randf() < 0.55 {
                s.spawn_bubble(chord_hue);
            }
            if s.randf() < 0.25 {
                s.spawn_bubble(chord_hue);
            }
        }
        let snare_onset = a.snare > s.prev_snare.saturating_add(28);
        s.prev_snare = a.snare;
        if snare_onset {
            // Strobe but don't disturb the bubbles.
            s.kick_t = s.kick_t.max(0.9);
            s.strobe_hue = (s.strobe_hue + 47.0).rem_euclid(360.0);
        }
        s.kick_t *= 0.60;

        // ── Beat onset (BPM-derived) → drives pattern cycling + mild strobe ──
        let curr_phase = a.beat_phase;
        let beat_onset = (s.prev_beat_phase as i16) > 180 && (curr_phase as i16) < 60;
        s.prev_beat_phase = curr_phase;
        if beat_onset {
            s.beat_count += 1;
            // Mild strobe on every beat as a backstop when drum detection is sparse.
            s.kick_t = s.kick_t.max(0.55);
        }

        // ── Pitch variation tracker — drives bubble speed ──
        if a.note_midi > 0 && s.prev_note_midi > 0 && a.note_midi != s.prev_note_midi {
            let diff = (a.note_midi as i16 - s.prev_note_midi as i16).unsigned_abs() as f32;
            s.pitch_var = (s.pitch_var + diff * 0.06).min(1.0);
        }
        if a.note_midi > 0 {
            s.prev_note_midi = a.note_midi;
        }
        s.pitch_var *= 0.982;

        // ── Pattern cycling: every 4 beats, or after watchdog timeout ──
        s.pattern_watchdog += 1;
        let beat_advance = beat_onset && s.beat_count % 4 == 0;
        if beat_advance || s.pattern_watchdog >= PATTERN_WATCHDOG_FRAMES {
            s.pattern = (s.pattern + 1) % NUM_PATTERNS;
            s.pattern_watchdog = 0;
            s.beat_count = 0;
            // Pattern-entry init
            if s.pattern == PATTERN_STREAM {
                let angle = s.randf() * std::f32::consts::TAU;
                let speed = 0.38;
                s.stream_vx = speed * angle.cos();
                s.stream_vy = speed * angle.sin() * 0.35; // soften vertical
            }
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

        // ── Periodic spawn (keeps population near TARGET_BUBBLES) ──
        s.spawn_timer += 1;
        if s.spawn_timer >= SPAWN_INTERVAL && s.bubbles.len() < TARGET_BUBBLES {
            let chord_hue = s.chord_hue;
            s.spawn_bubble(chord_hue);
            s.spawn_timer = 0;
        }

        // ── Try to form a new dance pair periodically + with extra chance on kicks ──
        s.dance_spawn_timer += 1;
        let timer_fired = s.dance_spawn_timer >= DANCE_SPAWN_INTERVAL;
        let kick_pair_chance = if kick_onset { s.randf() < 0.55 } else { false };
        if timer_fired || kick_pair_chance {
            s.dance_spawn_timer = 0;
            s.try_create_dance_pair();
        }

        // ── Advance bubbles ──
        // Speed is driven primarily by pitch variation; a small floor so
        // bubbles never feel frozen during long held notes.
        let speed_boost = 0.55 + s.pitch_var * 1.45 + s.mid_smooth * 0.15;
        let pattern = s.pattern;
        let stream_vx = s.stream_vx;
        let stream_vy = s.stream_vy;
        let cx_strip = COLS as f32 / 2.0;
        let cy_strip = ROWS as f32 / 2.0 - 0.5;
        let frame_f = frame as f32;

        // ── Resolve dance pairs first: rotate each pair around its midpoint,
        // mark its members as `dancing` so the pattern step skips them. ──
        for b in s.bubbles.iter_mut() {
            b.dancing = false;
        }
        let rot_step = (0.10 + s.pitch_var * 0.06) * speed_boost;
        let cos_step = rot_step.cos();
        let sin_step = rot_step.sin();
        let mut pi = 0;
        while pi < s.dance_pairs.len() {
            let (a_id, b_id, decayed) = {
                let pair = &mut s.dance_pairs[pi];
                pair.remaining = pair.remaining.saturating_sub(1);
                (pair.a_id, pair.b_id, pair.remaining == 0)
            };
            if decayed {
                s.dance_pairs.swap_remove(pi);
                continue;
            }
            let idx_a = s
                .bubbles
                .iter()
                .position(|b| b.id == a_id && matches!(b.state, BubbleState::Floating));
            let idx_b = s
                .bubbles
                .iter()
                .position(|b| b.id == b_id && matches!(b.state, BubbleState::Floating));
            if let (Some(ia), Some(ib)) = (idx_a, idx_b) {
                let ax = s.bubbles[ia].x;
                let ay = s.bubbles[ia].y;
                let bx = s.bubbles[ib].x;
                let by = s.bubbles[ib].y;
                let mx = (ax + bx) * 0.5;
                let my = (ay + by) * 0.5;
                let hdx = (ax - bx) * 0.5;
                let hdy = (ay - by) * 0.5;
                let new_hdx = hdx * cos_step - hdy * sin_step;
                let new_hdy = hdx * sin_step + hdy * cos_step;
                s.bubbles[ia].x = mx + new_hdx;
                s.bubbles[ia].y = my + new_hdy;
                s.bubbles[ia].dancing = true;
                s.bubbles[ib].x = mx - new_hdx;
                s.bubbles[ib].y = my - new_hdy;
                s.bubbles[ib].dancing = true;
                pi += 1;
            } else {
                // Partner missing (exploded or culled) — drop the pair.
                s.dance_pairs.swap_remove(pi);
            }
        }

        for b in s.bubbles.iter_mut() {
            match b.state {
                BubbleState::Floating => {
                    if b.dancing {
                        // Position already set by the dance-pair loop; just
                        // still advance born_progress so newly-paired bubbles
                        // can finish growing in.
                        if b.born_progress < 1.0 {
                            b.born_progress = (b.born_progress + 0.06).min(1.0);
                        }
                        continue;
                    }
                    let (dx, dy) = match pattern {
                        PATTERN_DRIFT => (b.vx, b.vy),
                        PATTERN_STREAM => (stream_vx, stream_vy),
                        PATTERN_WAVE => {
                            let hdir = if b.vx >= 0.0 { 1.0 } else { -1.0 };
                            let hspeed = 0.32 * hdir;
                            let wave_y = (frame_f * 0.11 + b.x * 0.20).sin() * 0.22;
                            (hspeed, wave_y)
                        }
                        PATTERN_ORBIT => {
                            let ox = b.x - cx_strip;
                            let oy = b.y - cy_strip;
                            let r = (ox * ox + oy * oy).sqrt();
                            if r < 1.0 {
                                // Push outward when too close to centre — avoids stalls.
                                let kick_angle =
                                    (frame_f * 0.03 + b.x * 0.21).rem_euclid(std::f32::consts::TAU);
                                (kick_angle.cos() * 0.22, kick_angle.sin() * 0.22)
                            } else {
                                let orbit_speed = 0.30;
                                (-oy / r * orbit_speed, ox / r * orbit_speed)
                            }
                        }
                        PATTERN_CONVERGE => {
                            let to_cx = cx_strip - b.x;
                            let to_cy = cy_strip - b.y;
                            let r = (to_cx * to_cx + to_cy * to_cy).sqrt();
                            if r < 1.5 {
                                // Reached the centre — pop visibly.
                                b.state = BubbleState::Exploding(0.0);
                                (0.0, 0.0)
                            } else {
                                let conv = 0.28;
                                (to_cx / r * conv, to_cy / r * conv)
                            }
                        }
                        _ => (0.0, 0.0),
                    };
                    b.x += dx * speed_boost;
                    b.y += dy * speed_boost;
                    if b.born_progress < 1.0 {
                        b.born_progress = (b.born_progress + 0.06).min(1.0);
                    }
                }
                BubbleState::Exploding(p) => {
                    b.state = BubbleState::Exploding(p + 0.12);
                }
            }
        }

        // ── Cull only when fully off-screen or explosion complete ──
        s.bubbles.retain(|b| match b.state {
            BubbleState::Exploding(p) => p < 1.0,
            BubbleState::Floating => {
                let margin = b.radius + 0.5;
                b.x > -margin
                    && b.x < COLS as f32 + margin
                    && b.y > -margin
                    && b.y < ROWS as f32 + margin
            }
        });

        // ══════════════════════════════════════════════
        // RENDER
        // ══════════════════════════════════════════════
        strip.set_all([0, 0, 0, 0]);

        let global_pulse = 0.78 + s.beat_brightness * 0.15 + s.bass_smooth * 0.07;

        for b in s.bubbles.iter() {
            match b.state {
                BubbleState::Floating => {
                    // Born bubbles ease in via born_progress (radius grows 0→full)
                    let effective_r = b.radius * b.born_progress;
                    if effective_r < 0.2 {
                        continue;
                    }
                    draw_circle(strip, b.x, b.y, effective_r, b.hue, b.sat, global_pulse);
                }
                BubbleState::Exploding(p) => {
                    let pc = p.clamp(0.0, 1.0);
                    // Ring grows from base radius to ~3× base, brightness fades
                    let ring_r = b.radius * (1.0 + pc * 2.0);
                    let brightness = (1.0 - pc).powi(2);
                    draw_ring(strip, b.x, b.y, ring_r, 1.1, b.hue, b.sat, brightness);
                }
            }
        }

        // ── Strobe (kick / snare / beat — fires on the main beat). ──
        // Lerps every pixel toward the current strobe colour (rotates 47°
        // on every kick or snare onset, so each flash is a different hue).
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

/// Soft-edge filled disc — solid core out to 70% radius then quick taper.
/// Clips cleanly when centre is off-grid (so bubbles can drift in/out).
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

/// Thin annulus — for the explosion ring.
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
