use crate::command::AudioAnalysis;
use crate::hardware::led::{hsv_to_rgb, LedStrip};
use std::cell::RefCell;

const COLS: usize = 60;
const ROWS: usize = 6;

// Classic Game of Life shapes for spawning
const GLIDER: [(i32, i32); 5] = [(0, 0), (1, 0), (2, 0), (2, -1), (1, -2)];
const BLINKER: [(i32, i32); 3] = [(0, 0), (1, 0), (2, 0)];
const BLOCK: [(i32, i32); 4] = [(0, 0), (1, 0), (0, 1), (1, 1)];
const LWSS: [(i32, i32); 9] = [
    (0, 0),
    (1, 0),
    (2, 0),
    (3, 0),
    (4, -1),
    (4, -3),
    (0, -3),
    (0, -1),
    (1, -3),
];
const R_PENTOMINO: [(i32, i32); 5] = [(1, 0), (2, 0), (0, -1), (1, -1), (1, -2)];

struct LifeState {
    grid: [[bool; COLS]; ROWS],
    next: [[bool; COLS]; ROWS],
    age: [[u16; COLS]; ROWS], // how many generations a cell has been alive
    hue_map: [[f32; COLS]; ROWS], // hue when cell was born
    frame: usize,
    gen_timer: f32,   // accumulates toward next generation step
    gen_count: usize, // total generations
    population: usize,
    dead_frames: usize, // frames with very low population
    energy: f32,
    bass_smooth: f32,
    mid_smooth: f32,
    chord_hue: f32,
    chord_hue_target: f32,
    prev_kick: u8,
    kick_flash: f32,
    bpm_smooth: f32,
    seed: u32,
}

impl LifeState {
    fn rand(&mut self) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed
    }
    fn rand_range(&mut self, max: usize) -> usize {
        self.rand() as usize % max
    }
}

thread_local! {
    static STATE: RefCell<LifeState> = RefCell::new(LifeState {
        grid: [[false; COLS]; ROWS],
        next: [[false; COLS]; ROWS],
        age: [[0; COLS]; ROWS],
        hue_map: [[0.0; COLS]; ROWS],
        frame: 0,
        gen_timer: 0.0,
        gen_count: 0,
        population: 0,
        dead_frames: 0,
        energy: 0.3,
        bass_smooth: 0.0,
        mid_smooth: 0.0,
        chord_hue: 0.0,
        chord_hue_target: 0.0,
        prev_kick: 0,
        kick_flash: 0.0,
        bpm_smooth: 120.0,
        seed: 54321,
    });
}

/// Conway's Game of Life — music drives simulation speed, spawns, and colors.
pub fn game_of_life(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();

        // Seed initial population
        if s.frame == 0 {
            spawn_random(s, 80);
        }
        s.frame = s.frame.wrapping_add(1);

        // ── Audio ──
        let bass_f = ((a.bands[0] as u16 + a.bands[1] as u16) / 2) as f32 / 255.0;
        let mid_f = ((a.bands[3] as u16 + a.bands[4] as u16) / 2) as f32 / 255.0;
        smooth(
            &mut s.energy,
            a.bands.iter().map(|&b| b as f32).sum::<f32>() / (255.0 * 8.0),
            0.3,
            0.05,
        );
        smooth(&mut s.bass_smooth, bass_f, 0.4, 0.08);
        smooth(&mut s.mid_smooth, mid_f, 0.3, 0.06);
        if a.bpm > 0 {
            smooth(&mut s.bpm_smooth, a.bpm as f32, 0.05, 0.05);
        }
        let energy = s.energy.max(0.15);

        // Chord hue
        if a.chord_root < 12 {
            let note_hues = [
                0.0, 30.0, 55.0, 80.0, 120.0, 160.0, 195.0, 220.0, 260.0, 285.0, 320.0, 345.0,
            ];
            s.chord_hue_target = note_hues[a.chord_root as usize];
        }
        let hue_diff = ((s.chord_hue_target - s.chord_hue + 540.0) % 360.0) - 180.0;
        s.chord_hue += hue_diff * 0.08;
        s.chord_hue = ((s.chord_hue % 360.0) + 360.0) % 360.0;

        // Beat detection
        let kick_onset = a.kick > 150 && s.prev_kick < 100;
        s.prev_kick = a.kick;
        if kick_onset {
            s.kick_flash = 1.0;
        }
        s.kick_flash *= 0.85;

        // ── Simulation speed: tied to BPM and energy ──
        // Higher energy = faster generations
        let beat_boost = if s.kick_flash > 0.5 { 3.0 } else { 1.0 };
        let gen_speed = (s.bpm_smooth / 120.0) * (1.0 + energy * 2.0) * beat_boost;
        s.gen_timer += gen_speed * 0.25;

        // Step one or more generations
        while s.gen_timer >= 1.0 {
            s.gen_timer -= 1.0;
            step_generation(s);
        }

        // ── Spawning logic ──
        // Count population
        s.population = 0;
        for row in 0..ROWS {
            for col in 0..COLS {
                if s.grid[row][col] {
                    s.population += 1;
                }
            }
        }

        // If population is very low, increase dead_frames counter
        if s.population < 10 {
            s.dead_frames += 1;
        } else {
            s.dead_frames = 0;
        }

        // Spawn new life when things die off
        if s.dead_frames > 30 {
            // Big respawn: multiple shapes
            let num_shapes = 3 + s.rand_range(4);
            for _ in 0..num_shapes {
                spawn_shape(s);
            }
            spawn_random(s, 20);
            s.dead_frames = 0;
        } else if s.population < 20 && s.dead_frames > 10 {
            // Small injection
            spawn_shape(s);
            spawn_random(s, 8);
            s.dead_frames = 0;
        }

        // Kick spawns a shape
        if kick_onset {
            spawn_shape(s);
        }

        // Snare/hihat can spawn small random cells
        if a.snare > 150 {
            let n = 2 + s.rand_range(4);
            spawn_random(s, n);
        }

        // ── Render ──
        for row in 0..ROWS {
            for col in 0..COLS {
                if s.grid[row][col] {
                    let age = s.age[row][col].min(200) as f32;
                    let birth_hue = s.hue_map[row][col];

                    // Hue shifts slightly with age
                    let hue = (birth_hue + age * 0.5) % 360.0;

                    // Saturation: young cells are vivid, old cells fade slightly
                    let sat = (0.9 - age * 0.002).max(0.4);

                    // Brightness: pulses with energy, kick flash boosts
                    let base_bright = 0.5 + energy * 0.3;
                    let kick_boost = if s.kick_flash > 0.3 {
                        s.kick_flash * 0.3
                    } else {
                        0.0
                    };
                    let bright = (base_bright + kick_boost).min(1.0);

                    // Newborn cells flash brighter
                    let newborn_flash = if s.age[row][col] < 3 { 0.2 } else { 0.0 };

                    let [r, g, b, _] = hsv_to_rgb(hue, sat, (bright + newborn_flash).min(1.0));
                    strip.set(row * COLS + col, [r, g, b, 0]);
                } else {
                    // Dead cells: very faint glow if they recently died (ghost effect)
                    let age = s.age[row][col];
                    if age > 0 && age < 8 {
                        let ghost = (8 - age) as f32 / 8.0;
                        let dim = (ghost * 0.1 * (0.5 + energy)) as f32;
                        let [r, g, b, _] = hsv_to_rgb(s.hue_map[row][col], 0.5, dim);
                        strip.set(row * COLS + col, [r, g, b, 0]);
                    } else {
                        strip.set(row * COLS + col, [0, 0, 0, 0]);
                    }
                }
            }
        }
    });
}

fn step_generation(s: &mut LifeState) {
    // Standard Conway rules with wrapping edges
    for row in 0..ROWS {
        for col in 0..COLS {
            let mut neighbors = 0u8;
            for dr in [-1i32, 0, 1] {
                for dc in [-1i32, 0, 1] {
                    if dr == 0 && dc == 0 {
                        continue;
                    }
                    let nr = ((row as i32 + dr) % ROWS as i32 + ROWS as i32) as usize % ROWS;
                    let nc = ((col as i32 + dc) % COLS as i32 + COLS as i32) as usize % COLS;
                    if s.grid[nr][nc] {
                        neighbors += 1;
                    }
                }
            }

            s.next[row][col] = if s.grid[row][col] {
                // Alive: survive with 2 or 3 neighbors
                neighbors == 2 || neighbors == 3
            } else {
                // Dead: birth with exactly 3 neighbors
                neighbors == 3
            };
        }
    }

    // Update grid and ages
    for row in 0..ROWS {
        for col in 0..COLS {
            if s.next[row][col] {
                if s.grid[row][col] {
                    // Still alive: increment age
                    s.age[row][col] = s.age[row][col].saturating_add(1);
                } else {
                    // Newborn: set age to 1, assign current chord hue
                    s.age[row][col] = 1;
                    s.hue_map[row][col] = s.chord_hue;
                }
            } else {
                if s.grid[row][col] {
                    // Just died: reset age to small value for ghost trail
                    s.age[row][col] = 1;
                } else {
                    // Still dead: decay ghost
                    s.age[row][col] = s.age[row][col].saturating_add(1);
                }
            }
            s.grid[row][col] = s.next[row][col];
        }
    }
    s.gen_count += 1;
}

fn spawn_shape(s: &mut LifeState) {
    let shape_type = s.rand_range(5);
    let ox = s.rand_range(COLS) as i32;
    let oy = s.rand_range(ROWS) as i32;
    let hue = s.chord_hue;

    let shape: &[(i32, i32)] = match shape_type {
        0 => &GLIDER,
        1 => &BLINKER,
        2 => &BLOCK,
        3 => &LWSS,
        _ => &R_PENTOMINO,
    };

    for &(dx, dy) in shape {
        let col = ((ox + dx) % COLS as i32 + COLS as i32) as usize % COLS;
        let row = ((oy + dy) % ROWS as i32 + ROWS as i32) as usize % ROWS;
        s.grid[row][col] = true;
        s.age[row][col] = 1;
        s.hue_map[row][col] = hue;
    }
}

fn spawn_random(s: &mut LifeState, count: usize) {
    let hue = s.chord_hue;
    for _ in 0..count {
        let col = s.rand_range(COLS);
        let row = s.rand_range(ROWS);
        s.grid[row][col] = true;
        s.age[row][col] = 1;
        s.hue_map[row][col] = hue;
    }
}

fn smooth(current: &mut f32, target: f32, attack: f32, decay: f32) {
    if target > *current {
        *current += (target - *current) * attack;
    } else {
        *current += (target - *current) * decay;
    }
}
