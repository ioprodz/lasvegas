use crate::command::AudioAnalysis;
use crate::hardware::led::{hsv_to_rgb, LedStrip};
use std::cell::RefCell;

const COLS: usize = 60;
const ROWS: usize = 6;
const PROGRESSION_LEN: usize = 4;

// ── Per-block game entities ──

#[derive(Copy, Clone)]
struct Invader {
    x: f32,
    dir: f32,
    frame_phase: f32, // for leg animation
}

#[derive(Copy, Clone)]
struct PongBall {
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
}

#[derive(Copy, Clone)]
struct SnakeSegment {
    x: f32,
    y: f32,
}

#[derive(Copy, Clone)]
struct Bird {
    y: f32,
    vy: f32,
}

#[derive(Copy, Clone)]
struct NyanTrail {
    offset: f32,
}

#[derive(Copy, Clone)]
struct MarioRunner {
    y: f32, // jump height
    vy: f32,
    on_ground: bool,
}

// ── Main state ──

struct ArcadeState {
    frame: usize,
    // Chord progression fingerprint (same as harmonic series)
    chord_history: [u8; PROGRESSION_LEN],
    chord_count: usize,
    prev_chord_root: u8,
    chord_stable_frames: usize,
    fingerprint: u64,
    active_count: usize,
    block_types: [usize; 6],
    // Audio
    chord_hue: f32,
    chord_hue_target: f32,
    energy: f32,
    bass_smooth: f32,
    mid_smooth: f32,
    treble_smooth: f32,
    beat_brightness: f32,
    prev_kick: u8,
    kick_flash: f32,
    bpm_smooth: f32,
    // Per-block game state
    invaders: [[Invader; 4]; 6], // up to 4 invaders per block
    pong_balls: [PongBall; 6],
    pong_paddles: [[f32; 2]; 6], // left, right paddle y
    snake_segments: [[SnakeSegment; 8]; 6],
    snake_dir: [f32; 6],
    snake_timer: [f32; 6],
    birds: [Bird; 6],
    bird_pipe_x: [f32; 6],
    bird_pipe_gap: [f32; 6],
    nyan: [NyanTrail; 6],
    mario: [MarioRunner; 6],
    mario_coin_x: [f32; 6],
    // RNG
    seed: u32,
}

impl ArcadeState {
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
        let raw = (fp & 0xFF) as usize % 4;
        self.active_count = [2, 3, 4, 5][raw];
        for i in 0..6 {
            let shift = 8 + i * 4;
            self.block_types[i] = ((fp >> shift) & 0x0F) as usize % 6;
        }
        // Mirror: right blocks match left
        let n = self.active_count;
        for i in 0..n / 2 {
            self.block_types[n - 1 - i] = self.block_types[i];
        }
    }
}

const INIT_INVADER: Invader = Invader {
    x: 0.0,
    dir: 1.0,
    frame_phase: 0.0,
};
const INIT_BALL: PongBall = PongBall {
    x: 0.5,
    y: 2.5,
    dx: 0.15,
    dy: 0.1,
};
const INIT_SEG: SnakeSegment = SnakeSegment { x: 0.0, y: 3.0 };
const INIT_BIRD: Bird = Bird { y: 3.0, vy: 0.0 };
const INIT_NYAN: NyanTrail = NyanTrail { offset: 0.0 };
const INIT_MARIO: MarioRunner = MarioRunner {
    y: 0.0,
    vy: 0.0,
    on_ground: true,
};

thread_local! {
    static STATE: RefCell<ArcadeState> = RefCell::new(ArcadeState {
        frame: 0,
        chord_history: [255; PROGRESSION_LEN],
        chord_count: 0,
        prev_chord_root: 255,
        chord_stable_frames: 0,
        fingerprint: 0,
        active_count: 3,
        block_types: [0, 1, 2, 3, 4, 5],
        chord_hue: 0.0,
        chord_hue_target: 0.0,
        energy: 0.0,
        bass_smooth: 0.0,
        mid_smooth: 0.0,
        treble_smooth: 0.0,
        beat_brightness: 0.0,
        prev_kick: 0,
        kick_flash: 0.0,
        bpm_smooth: 120.0,
        invaders: [[INIT_INVADER; 4]; 6],
        pong_balls: [INIT_BALL; 6],
        pong_paddles: [[2.0, 3.0]; 6],
        snake_segments: [[INIT_SEG; 8]; 6],
        snake_dir: [1.0; 6],
        snake_timer: [0.0; 6],
        birds: [INIT_BIRD; 6],
        bird_pipe_x: [15.0; 6],
        bird_pipe_gap: [2.0; 6],
        nyan: [INIT_NYAN; 6],
        mario: [INIT_MARIO; 6],
        mario_coin_x: [10.0; 6],
        seed: 77777,
    });
}

/// Retro Arcade — 6 classic game visualizations in chord-driven blocks.
pub fn retro_arcade(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();
        s.frame = s.frame.wrapping_add(1);

        // ── Audio smoothing ──
        let bass_f = ((a.bands[0] as u16 + a.bands[1] as u16) / 2) as f32 / 255.0;
        let mid_f = ((a.bands[3] as u16 + a.bands[4] as u16) / 2) as f32 / 255.0;
        let treble_f = ((a.bands[6] as u16 + a.bands[7] as u16) / 2) as f32 / 255.0;
        smooth(
            &mut s.energy,
            a.bands.iter().map(|&b| b as f32).sum::<f32>() / (255.0 * 8.0),
            0.3,
            0.05,
        );
        smooth(&mut s.bass_smooth, bass_f, 0.4, 0.08);
        smooth(&mut s.mid_smooth, mid_f, 0.3, 0.06);
        smooth(&mut s.treble_smooth, treble_f, 0.3, 0.06);
        if a.bpm > 0 {
            smooth(&mut s.bpm_smooth, a.bpm as f32, 0.05, 0.05);
        }
        let energy = s.energy.max(0.15);
        let speed_mult = s.bpm_smooth / 120.0;

        // Beat detection
        let kick_onset = a.kick > 150 && s.prev_kick < 100;
        s.prev_kick = a.kick;
        if kick_onset {
            s.kick_flash = 1.0;
        }
        s.kick_flash *= 0.85;
        s.beat_brightness *= 0.9;
        if kick_onset {
            s.beat_brightness = 1.0;
        }

        // ── Chord hue ──
        if a.chord_root < 12 {
            let note_hues = [
                0.0, 30.0, 55.0, 80.0, 120.0, 160.0, 195.0, 220.0, 260.0, 285.0, 320.0, 345.0,
            ];
            s.chord_hue_target = note_hues[a.chord_root as usize];
        }
        let hue_diff = ((s.chord_hue_target - s.chord_hue + 540.0) % 360.0) - 180.0;
        s.chord_hue += hue_diff * 0.08;
        s.chord_hue = ((s.chord_hue % 360.0) + 360.0) % 360.0;

        // ── Chord progression → fingerprint ──
        if a.chord_root < 12 {
            if a.chord_root == s.prev_chord_root {
                s.chord_stable_frames += 1;
            } else {
                s.chord_stable_frames = 0;
            }
            s.prev_chord_root = a.chord_root;

            if s.chord_stable_frames == 15 {
                let idx = s.chord_count % PROGRESSION_LEN;
                if s.chord_history[idx] != a.chord_root {
                    s.chord_history[idx] = a.chord_root;
                    s.chord_count += 1;
                    let new_fp = s.compute_fingerprint();
                    if new_fp != s.fingerprint {
                        s.fingerprint = new_fp;
                        s.apply_fingerprint();
                    }
                }
            }
        }

        // ── Initialize blocks on first meaningful frame ──
        if s.frame == 1 {
            for slot in 0..6 {
                // Invaders: spread across block
                for j in 0..4 {
                    s.invaders[slot][j].x = j as f32 * 0.25 + 0.1;
                    let r = s.randf();
                    s.invaders[slot][j].dir = if r > 0.5 { 1.0 } else { -1.0 };
                    s.invaders[slot][j].frame_phase = s.randf() * 6.28;
                }
                // Snake: horizontal line
                for j in 0..8 {
                    s.snake_segments[slot][j].x = 0.5 - j as f32 * 0.06;
                    s.snake_segments[slot][j].y = 3.0;
                }
                s.bird_pipe_x[slot] = 0.8 + s.randf() * 0.3;
                s.bird_pipe_gap[slot] = 1.5 + s.randf() * 1.5;
                s.mario_coin_x[slot] = 0.6 + s.randf() * 0.3;
            }
        }

        let active_count = s.active_count;
        let block_width = COLS / active_count;

        // ── Clear ──
        for i in 0..ROWS * COLS {
            strip.set(i, [0, 0, 0, 0]);
        }

        // ── Render each block ──
        for slot in 0..active_count {
            let col_start = slot * block_width;
            let block_type = s.block_types[slot % 6];
            let slot_hue = (s.chord_hue + slot as f32 * 40.0) % 360.0;

            match block_type {
                0 => render_invaders(
                    strip,
                    s,
                    slot,
                    col_start,
                    block_width,
                    slot_hue,
                    energy,
                    speed_mult,
                    kick_onset,
                ),
                1 => render_pong(
                    strip,
                    s,
                    slot,
                    col_start,
                    block_width,
                    slot_hue,
                    energy,
                    speed_mult,
                ),
                2 => render_snake(
                    strip,
                    s,
                    slot,
                    col_start,
                    block_width,
                    slot_hue,
                    energy,
                    speed_mult,
                    kick_onset,
                ),
                3 => render_flappy(
                    strip,
                    s,
                    slot,
                    col_start,
                    block_width,
                    slot_hue,
                    energy,
                    speed_mult,
                    kick_onset,
                ),
                4 => render_nyan(
                    strip,
                    s,
                    slot,
                    col_start,
                    block_width,
                    slot_hue,
                    energy,
                    speed_mult,
                ),
                _ => render_mario(
                    strip,
                    s,
                    slot,
                    col_start,
                    block_width,
                    slot_hue,
                    energy,
                    speed_mult,
                    kick_onset,
                ),
            }

            // Block separator: thin dark line
            if slot < active_count - 1 {
                let sep_col = col_start + block_width - 1;
                for row in 0..ROWS {
                    strip.set(row * COLS + sep_col, [0, 0, 0, 0]);
                }
            }
        }
    });
}

// ── 0: Space Invaders ──
fn render_invaders(
    strip: &mut LedStrip,
    s: &mut ArcadeState,
    slot: usize,
    col_start: usize,
    bw: usize,
    hue: f32,
    energy: f32,
    speed: f32,
    kick: bool,
) {
    // Invaders march side to side, step down on kick
    for j in 0..4 {
        let inv = &mut s.invaders[slot][j];
        inv.x += inv.dir * 0.008 * speed * (1.0 + energy);
        inv.frame_phase += 0.1 * speed;
        if inv.x > 0.85 || inv.x < 0.05 {
            inv.dir = -inv.dir;
        }
    }

    // Draw invaders (each is ~3 cols wide, on rows 0-2)
    for j in 0..4 {
        let inv = &s.invaders[slot][j];
        let cx = col_start as f32 + inv.x * bw as f32;
        let row_base = if kick { 1 } else { 0 }; // step down on kick
        let legs_up = inv.frame_phase.sin() > 0.0;

        // Body (3 pixels on row_base)
        for dc in -1i32..=1 {
            let col =
                (cx as i32 + dc).clamp(col_start as i32, (col_start + bw - 1) as i32) as usize;
            let bright = 0.7 + energy * 0.3;
            let [r, g, b, _] = hsv_to_rgb(hue + j as f32 * 20.0, 0.8, bright);
            strip.set(row_base * COLS + col, [r, g, b, 0]);
        }
        // Eyes (row_base + 1)
        if row_base + 1 < ROWS {
            let eye_l =
                (cx as i32 - 1).clamp(col_start as i32, (col_start + bw - 1) as i32) as usize;
            let eye_r =
                (cx as i32 + 1).clamp(col_start as i32, (col_start + bw - 1) as i32) as usize;
            strip.set((row_base + 1) * COLS + eye_l, [180, 180, 220, 0]);
            strip.set((row_base + 1) * COLS + eye_r, [180, 180, 220, 0]);
        }
        // Legs (row_base + 2)
        if row_base + 2 < ROWS {
            let leg_off = if legs_up { 0 } else { 1 };
            let leg_col = (cx as i32 - 1 + leg_off)
                .clamp(col_start as i32, (col_start + bw - 1) as i32)
                as usize;
            let leg_col2 = (cx as i32 + 1 - leg_off)
                .clamp(col_start as i32, (col_start + bw - 1) as i32)
                as usize;
            let [r, g, b, _] = hsv_to_rgb(hue + j as f32 * 20.0, 0.6, 0.5);
            strip.set((row_base + 2) * COLS + leg_col, [r, g, b, 0]);
            strip.set((row_base + 2) * COLS + leg_col2, [r, g, b, 0]);
        }
    }

    // Player ship at bottom
    let ship_x = col_start + bw / 2;
    let ship_sway = (s.frame as f32 * 0.03 * speed).sin() * (bw as f32 * 0.3);
    let sx = (ship_x as f32 + ship_sway).clamp(col_start as f32 + 1.0, (col_start + bw - 2) as f32)
        as usize;
    strip.set((ROWS - 1) * COLS + sx, [0, 255, 0, 0]);
    if sx > col_start {
        strip.set((ROWS - 1) * COLS + sx - 1, [0, 150, 0, 0]);
    }
    if sx < col_start + bw - 1 {
        strip.set((ROWS - 1) * COLS + sx + 1, [0, 150, 0, 0]);
    }

    // Bullet on kick
    if kick || s.kick_flash > 0.5 {
        let bullet_row = (ROWS as f32 - 2.0 - s.kick_flash * 3.0).max(0.0) as usize;
        if sx >= col_start && sx < col_start + bw {
            strip.set(bullet_row * COLS + sx, [255, 255, 100, 0]);
        }
    }
}

// ── 1: Pong ──
fn render_pong(
    strip: &mut LedStrip,
    s: &mut ArcadeState,
    slot: usize,
    col_start: usize,
    bw: usize,
    hue: f32,
    energy: f32,
    speed: f32,
) {
    let ball = &mut s.pong_balls[slot];
    let paddles = &mut s.pong_paddles[slot];

    // Move ball
    ball.x += ball.dx * speed * (0.8 + energy * 0.5);
    ball.y += ball.dy * speed * (0.8 + energy * 0.5);

    // Bounce off top/bottom
    if ball.y <= 0.5 || ball.y >= ROWS as f32 - 0.5 {
        ball.dy = -ball.dy;
        ball.y = ball.y.clamp(0.5, ROWS as f32 - 0.5);
    }

    // Bounce off paddles (left/right edges of block)
    if ball.x <= 1.0 {
        ball.dx = ball.dx.abs();
        ball.dy += (ball.y - paddles[0]) * 0.1;
    }
    if ball.x >= bw as f32 - 1.0 {
        ball.dx = -ball.dx.abs();
        ball.dy += (ball.y - paddles[1]) * 0.1;
    }
    ball.x = ball.x.clamp(0.5, bw as f32 - 0.5);
    ball.dy = ball.dy.clamp(-0.3, 0.3);

    // AI paddles track ball
    smooth(&mut paddles[0], ball.y, 0.08, 0.08);
    smooth(&mut paddles[1], ball.y, 0.06, 0.06);

    // Draw paddles (2 pixels tall)
    for i in 0..2 {
        for p in 0..2 {
            let py = (paddles[p] as i32 - 1 + i).clamp(0, ROWS as i32 - 1) as usize;
            let px = if p == 0 {
                col_start
            } else {
                col_start + bw - 1
            };
            let [r, g, b, _] = hsv_to_rgb(hue + p as f32 * 180.0, 0.7, 0.8);
            strip.set(py * COLS + px, [r, g, b, 0]);
        }
    }

    // Draw ball
    let bx = col_start + ball.x.clamp(0.0, (bw - 1) as f32) as usize;
    let by = ball.y.clamp(0.0, (ROWS - 1) as f32) as usize;
    strip.set(by * COLS + bx, [255, 255, 255, 0]);
    // Trail
    let trail_x = (bx as f32 - ball.dx.signum() * 1.5) as i32;
    if trail_x >= col_start as i32 && trail_x < (col_start + bw) as i32 {
        strip.set(by * COLS + trail_x as usize, [80, 80, 80, 0]);
    }

    // Center line (dashed)
    let mid_col = col_start + bw / 2;
    for row in 0..ROWS {
        if row % 2 == 0 {
            strip.set(row * COLS + mid_col, [30, 30, 30, 0]);
        }
    }
}

// ── 2: Snake ──
fn render_snake(
    strip: &mut LedStrip,
    s: &mut ArcadeState,
    slot: usize,
    col_start: usize,
    bw: usize,
    hue: f32,
    energy: f32,
    speed: f32,
    kick: bool,
) {
    // Turn on kick
    if kick {
        let cur = s.snake_dir[slot];
        // Random turn: choose a perpendicular or continue
        let r = s.randf();
        s.snake_dir[slot] = if r < 0.3 {
            // Turn "up" in normalized space
            if cur.abs() > 0.5 {
                0.0
            } else {
                1.0
            }
        } else if r < 0.6 {
            if cur.abs() > 0.5 {
                0.0
            } else {
                -1.0
            }
        } else {
            cur
        };
    }

    s.snake_timer[slot] += 0.04 * speed * (1.0 + energy);
    if s.snake_timer[slot] >= 1.0 {
        s.snake_timer[slot] = 0.0;

        // Move: shift segments back
        for j in (1..8).rev() {
            s.snake_segments[slot][j].x = s.snake_segments[slot][j - 1].x;
            s.snake_segments[slot][j].y = s.snake_segments[slot][j - 1].y;
        }

        let dir = s.snake_dir[slot];
        let head = &mut s.snake_segments[slot][0];
        if dir.abs() < 0.1 {
            // Moving horizontally
            head.x += 0.08;
        } else {
            head.y += dir * 0.5;
        }
        // Wrap
        if head.x > 1.0 {
            head.x -= 1.0;
        }
        if head.x < 0.0 {
            head.x += 1.0;
        }
        head.y = head.y.clamp(0.0, (ROWS - 1) as f32);
    }

    // Draw snake
    for j in 0..8 {
        let seg = &s.snake_segments[slot][j];
        let col = col_start + (seg.x * (bw - 1) as f32) as usize;
        let row = seg.y.clamp(0.0, (ROWS - 1) as f32) as usize;
        let bright = 1.0 - j as f32 * 0.1;
        let [r, g, b, _] = hsv_to_rgb(hue + j as f32 * 8.0, 0.8, bright * (0.6 + energy * 0.4));
        if col >= col_start && col < col_start + bw {
            strip.set(row * COLS + col, [r, g, b, 0]);
        }
    }

    // Food: pulsing dot driven by treble
    let food_x = col_start + ((s.frame as f32 * 0.002).sin() * 0.3 + 0.6) as usize * bw.max(1) / 1;
    let food_col = col_start + (bw * 3 / 4).min(bw - 1);
    let food_row = ((s.frame as f32 * 0.01).sin() * 1.5 + 3.0) as usize % ROWS;
    let food_pulse = 0.5 + 0.5 * (s.frame as f32 * 0.15).sin();
    let fb = (200.0 * food_pulse) as u8;
    let _ = food_x; // use computed food_col instead
    strip.set(food_row * COLS + food_col, [fb, 20, 20, 0]);
}

// ── 3: Flappy Bird ──
fn render_flappy(
    strip: &mut LedStrip,
    s: &mut ArcadeState,
    slot: usize,
    col_start: usize,
    bw: usize,
    _hue: f32,
    energy: f32,
    speed: f32,
    kick: bool,
) {
    // Flap on kick
    if kick {
        s.birds[slot].vy = -0.6;
    }
    s.birds[slot].vy += 0.04 * speed; // gravity
    s.birds[slot].y += s.birds[slot].vy;
    s.birds[slot].y = s.birds[slot].y.clamp(0.0, (ROWS - 1) as f32);

    // Move pipe
    s.bird_pipe_x[slot] -= 0.01 * speed * (1.0 + energy * 0.5);
    if s.bird_pipe_x[slot] < 0.0 {
        s.bird_pipe_x[slot] = 1.0;
        s.bird_pipe_gap[slot] = 1.0 + s.randf() * 2.0;
    }

    // Draw pipe
    let pipe_col = col_start + (s.bird_pipe_x[slot] * (bw - 1) as f32) as usize;
    let gap_center = s.bird_pipe_gap[slot] + 1.0;
    let gap_half = 1.2;
    if pipe_col >= col_start && pipe_col < col_start + bw {
        for row in 0..ROWS {
            let ry = row as f32;
            if (ry - gap_center).abs() > gap_half {
                let [r, g, b, _] = hsv_to_rgb(120.0, 0.8, 0.5 + energy * 0.3);
                strip.set(row * COLS + pipe_col, [r, g, b, 0]);
            }
        }
        // Pipe caps
        let cap_top = (gap_center - gap_half).max(0.0) as usize;
        let cap_bot = (gap_center + gap_half).min((ROWS - 1) as f32) as usize;
        if cap_top < ROWS {
            let [r, g, b, _] = hsv_to_rgb(120.0, 0.6, 0.7);
            strip.set(cap_top * COLS + pipe_col, [r, g, b, 0]);
        }
        if cap_bot < ROWS {
            let [r, g, b, _] = hsv_to_rgb(120.0, 0.6, 0.7);
            strip.set(cap_bot * COLS + pipe_col, [r, g, b, 0]);
        }
    }

    // Draw bird (2 pixels: body + wing)
    let bird_col = col_start + bw / 4;
    let bird_row = s.birds[slot].y.clamp(0.0, (ROWS - 1) as f32) as usize;
    strip.set(bird_row * COLS + bird_col, [255, 220, 0, 0]); // yellow body
                                                             // Wing flap
    let wing_up = s.birds[slot].vy < 0.0;
    let wing_row = if wing_up {
        bird_row.saturating_sub(1)
    } else {
        (bird_row + 1).min(ROWS - 1)
    };
    strip.set(wing_row * COLS + bird_col, [255, 180, 0, 0]);
    // Eye
    if bird_col + 1 < col_start + bw {
        strip.set(bird_row * COLS + bird_col + 1, [255, 255, 255, 0]);
    }
}

// ── 4: Nyan Cat ──
fn render_nyan(
    strip: &mut LedStrip,
    s: &mut ArcadeState,
    slot: usize,
    col_start: usize,
    bw: usize,
    hue: f32,
    energy: f32,
    speed: f32,
) {
    s.nyan[slot].offset += 0.08 * speed * (0.8 + energy * 0.5);
    let off = s.nyan[slot].offset;

    // Rainbow trail: fills most of the block
    let cat_col = col_start + bw * 3 / 4;
    let rainbow_hues = [0.0, 30.0, 60.0, 120.0, 240.0, 280.0]; // ROYGBV for 6 rows

    for row in 0..ROWS {
        let row_hue = rainbow_hues[row];
        for col_off in 0..(bw * 3 / 4) {
            let col = col_start + col_off;
            // Wavy offset per row
            let wave = (col as f32 * 0.3 + off + row as f32 * 0.5).sin() * 0.3;
            let bright = (0.5 + wave + energy * 0.3).clamp(0.2, 0.9);
            let [r, g, b, _] = hsv_to_rgb(row_hue + hue * 0.1, 0.85, bright);
            strip.set(row * COLS + col, [r, g, b, 0]);
        }
    }

    // Cat body (poptart: pink square, rows 1-4)
    let bounce = (off * 2.0).sin().abs() * 0.8;
    let cat_row_off = bounce as i32;
    for row in 1..5 {
        let r = (row as i32 + cat_row_off).clamp(0, ROWS as i32 - 1) as usize;
        if cat_col < col_start + bw {
            strip.set(r * COLS + cat_col, [255, 150, 180, 0]); // pink poptart
        }
        if cat_col + 1 < col_start + bw {
            strip.set(r * COLS + cat_col + 1, [200, 120, 150, 0]);
        }
    }
    // Cat face
    let face_col = (cat_col + 2).min(col_start + bw - 1);
    let face_r = (2i32 + cat_row_off).clamp(0, ROWS as i32 - 1) as usize;
    strip.set(face_r * COLS + face_col, [140, 140, 140, 0]); // gray face
                                                             // Eyes
    let eye_r = (1i32 + cat_row_off).clamp(0, ROWS as i32 - 1) as usize;
    if face_col < col_start + bw {
        strip.set(eye_r * COLS + face_col, [40, 40, 40, 0]);
    }
}

// ── 5: Mario ──
fn render_mario(
    strip: &mut LedStrip,
    s: &mut ArcadeState,
    slot: usize,
    col_start: usize,
    bw: usize,
    _hue: f32,
    energy: f32,
    speed: f32,
    kick: bool,
) {
    let mario = &mut s.mario[slot];

    // Jump on kick
    if kick && mario.on_ground {
        mario.vy = -0.8;
        mario.on_ground = false;
    }
    mario.vy += 0.06 * speed; // gravity
    mario.y += mario.vy;
    if mario.y >= 0.0 {
        mario.y = 0.0;
        mario.vy = 0.0;
        mario.on_ground = true;
    }

    // Scroll coins
    s.mario_coin_x[slot] -= 0.008 * speed * (1.0 + energy * 0.5);
    if s.mario_coin_x[slot] < 0.0 {
        s.mario_coin_x[slot] = 1.0;
    }

    // Ground: brown blocks at bottom row
    for col_off in 0..bw {
        let col = col_start + col_off;
        let brick = if (col_off + s.frame / 8) % 3 == 0 {
            0.4
        } else {
            0.55
        };
        let [r, g, b, _] = hsv_to_rgb(25.0, 0.7, brick);
        strip.set((ROWS - 1) * COLS + col, [r, g, b, 0]);
    }

    // Question block
    let q_col = col_start + (s.mario_coin_x[slot] * (bw - 1) as f32) as usize;
    if q_col >= col_start && q_col < col_start + bw {
        let pulse = 0.6 + 0.4 * (s.frame as f32 * 0.1).sin();
        let [r, g, b, _] = hsv_to_rgb(45.0, 0.9, pulse);
        strip.set(1 * COLS + q_col, [r, g, b, 0]);
        // ? mark
        strip.set(2 * COLS + q_col, [200, 180, 50, 0]);
    }

    // Pipe
    let pipe_col = col_start + bw * 2 / 3;
    for row in 3..ROWS - 1 {
        let [r, g, b, _] = hsv_to_rgb(120.0, 0.8, 0.5);
        if pipe_col < col_start + bw {
            strip.set(row * COLS + pipe_col, [r, g, b, 0]);
        }
        if pipe_col + 1 < col_start + bw {
            strip.set(row * COLS + pipe_col + 1, [r, g, b, 0]);
        }
    }

    // Mario (2 rows tall above ground)
    let mario_col = col_start + bw / 4;
    let ground_row = ROWS - 2;
    let jump_offset = (-mario.y * 2.0) as i32;
    let body_row = (ground_row as i32 - jump_offset).clamp(0, ROWS as i32 - 1) as usize;
    let head_row = body_row.saturating_sub(1);

    // Hat (red)
    strip.set(head_row * COLS + mario_col, [220, 30, 0, 0]);
    // Face
    if mario_col > col_start {
        strip.set(head_row * COLS + mario_col - 1, [240, 190, 130, 0]);
    }
    // Body (blue overalls + red shirt)
    strip.set(body_row * COLS + mario_col, [0, 60, 220, 0]);
    // Running legs: alternate on beat
    let leg_frame = (s.frame / 4) % 2 == 0;
    if body_row + 1 < ROWS - 1 {
        let leg_col = if leg_frame {
            mario_col.saturating_sub(1).max(col_start)
        } else {
            (mario_col + 1).min(col_start + bw - 1)
        };
        strip.set((body_row + 1) * COLS + leg_col, [0, 40, 180, 0]);
    }

    // Coin sparkle
    if s.kick_flash > 0.3 {
        let sparkle_row = 0;
        let sparkle_col = (q_col + 1).min(col_start + bw - 1);
        let b = (255.0 * s.kick_flash) as u8;
        strip.set(sparkle_row * COLS + sparkle_col, [b, b, 0, 0]);
    }
}

fn smooth(current: &mut f32, target: f32, attack: f32, decay: f32) {
    if target > *current {
        *current += (target - *current) * attack;
    } else {
        *current += (target - *current) * decay;
    }
}
