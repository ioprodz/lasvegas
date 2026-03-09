use crate::command::AudioAnalysis;
use crate::hardware::led::{hsv_to_rgb, LedStrip};
use std::cell::RefCell;

const COLS: usize = 60;
const ROWS: usize = 6;
const NOTE_HUES: [f32; 12] = [
    0.0,   // C  — red
    30.0,  // C# — orange
    55.0,  // D  — gold
    80.0,  // D# — yellow-green
    120.0, // E  — green
    160.0, // F  — teal
    195.0, // F# — cyan
    220.0, // G  — blue
    260.0, // G# — indigo
    285.0, // A  — violet
    320.0, // A# — magenta
    345.0, // B  — rose
];

struct MelodyTrail {
    col: f32,
    row: f32,
    hue: f32,
    life: f32,
}

struct KickWave {
    age: f32,
    intensity: f32,
}

struct SnareFlash {
    age: f32,
    col: usize,
}

struct SynState {
    // Background chord color
    chord_hue: f32,
    chord_hue_target: f32,
    chord_sat: f32,
    chord_sat_target: f32,
    // Melody
    melody_trails: Vec<MelodyTrail>,
    melody_col: f32,   // smoothed horizontal position from pitch
    melody_col_target: f32,
    // Kick waves (travel upward from bottom)
    kick_waves: Vec<KickWave>,
    prev_kick: u8,
    // Snare flashes
    snare_flashes: Vec<SnareFlash>,
    prev_snare: u8,
    // Hi-hat sparkles
    seed: u32,
    // Beat breathing
    beat_brightness: f32,
    // Vocal glow
    vocal_glow: f32,
    // Bass line wave phase
    bass_phase: f32,
}

impl SynState {
    fn rand(&mut self) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed
    }
}

thread_local! {
    static STATE: RefCell<SynState> = RefCell::new(SynState {
        chord_hue: 0.0,
        chord_hue_target: 0.0,
        chord_sat: 0.5,
        chord_sat_target: 0.5,
        melody_trails: Vec::new(),
        melody_col: 30.0,
        melody_col_target: 30.0,
        kick_waves: Vec::new(),
        prev_kick: 0,
        snare_flashes: Vec::new(),
        prev_snare: 0,
        seed: 31337,
        beat_brightness: 0.0,
        vocal_glow: 0.0,
        bass_phase: 0.0,
    });
}

/// Synesthesia — a layered music visualization where every musical element
/// has its own visual representation:
///
/// Layer 1 (background):  Chord color wash — hue from chord root, saturation
///                         from chord quality (major=warm, minor=cool)
/// Layer 2 (bottom rows):  Kick drum shockwaves — travel upward on hits
/// Layer 3 (bottom half):  Bass line — flowing sine wave, wavelength from pitch
/// Layer 4 (middle):       Melody tracker — bright spot that follows pitch
///                         horizontally, leaves fading trails
/// Layer 5 (mid rows):     Snare — bright horizontal flashes
/// Layer 6 (top rows):     Hi-hat — rapid sparkles, density from hihat energy
/// Layer 7 (all):          Beat breathing — subtle brightness pulse synced to BPM
/// Layer 8 (middle glow):  Vocals — warm wide glow when vocals detected
pub fn audio_synesthesia(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();

        let kick_f = a.kick as f32 / 255.0;
        let _snare_f = a.snare as f32 / 255.0;
        let hihat_f = a.hihat as f32 / 255.0;
        let vocals_f = a.vocals as f32 / 255.0;
        let bass_line_f = a.bass_line as f32 / 255.0;
        let beat_phase = a.beat_phase as f32 / 255.0;
        let bass_f = ((a.bands[0] as u16 + a.bands[1] as u16) / 2) as f32 / 255.0;

        // ============================================
        // LAYER 1: Chord color wash (background)
        // ============================================
        if a.chord_root < 12 {
            s.chord_hue_target = NOTE_HUES[a.chord_root as usize];
            // Major = saturated warm, minor = desaturated cool
            s.chord_sat_target = match a.chord_quality {
                0 => 0.7,        // major — rich
                1 => 0.4,        // minor — muted
                2 => 0.3,        // dim — dark
                3 => 0.8,        // aug — vivid
                4 | 5 | 6 => 0.6, // 7ths — moderate
                _ => 0.5,
            };
        }
        // Smooth chord color transitions
        s.chord_hue = lerp_hue(s.chord_hue, s.chord_hue_target, 0.08);
        s.chord_sat += (s.chord_sat_target - s.chord_sat) * 0.08;

        // ============================================
        // LAYER 7: Beat breathing (compute first, used by background)
        // ============================================
        // Sine pulse synced to beat phase: peaks at phase 0 (on the beat)
        let beat_pulse = ((1.0 - beat_phase) * std::f32::consts::TAU).cos() * 0.5 + 0.5;
        s.beat_brightness += (beat_pulse - s.beat_brightness) * 0.3;

        // Clear to black — layers paint only where active
        strip.set_all([0, 0, 0, 0]);

        // ============================================
        // LAYER 2: Kick drum shockwaves (bottom → up)
        // ============================================
        let kick_onset = a.kick > s.prev_kick.saturating_add(30);
        s.prev_kick = a.kick;
        if kick_onset {
            s.kick_waves.push(KickWave {
                age: 0.0,
                intensity: kick_f.max(0.6),
            });
        }

        for wave in s.kick_waves.iter() {
            let wave_row = wave.age * 0.8; // travels upward
            let fade = (1.0 - wave.age / 20.0).max(0.0) * wave.intensity;
            let hue = (s.chord_hue + 180.0) % 360.0; // complementary color

            for row in 0..ROWS {
                let dist = (row as f32 - (ROWS as f32 - 1.0 - wave_row)).abs();
                if dist < 0.8 {
                    let brightness = (1.0 - dist / 0.8) * fade;
                    if brightness > 0.05 {
                        for col in 0..COLS {
                            let led = row * COLS + col;
                            add_color(strip, led, hsv_to_rgb(hue, 0.9, brightness));
                        }
                    }
                }
            }
        }
        s.kick_waves.iter_mut().for_each(|w| w.age += 1.0);
        s.kick_waves.retain(|w| w.age < 20.0);

        // ============================================
        // LAYER 3: Bass line wave (bottom two rows)
        // ============================================
        if a.note_midi > 0 && bass_line_f > 0.1 {
            // Wavelength inversely proportional to MIDI note
            // Lower notes = wider waves, higher = tighter
            let wavelength = 120.0 / (a.note_midi as f32).max(30.0) * 30.0;
            s.bass_phase += 0.15 + bass_f * 0.1;

            let bass_hue = if a.chord_root < 12 {
                NOTE_HUES[a.chord_root as usize]
            } else {
                s.chord_hue
            };

            for col in 0..COLS {
                let wave = ((col as f32 / wavelength) * std::f32::consts::TAU + s.bass_phase)
                    .sin();
                // Only light LEDs on the positive half of the wave
                if wave > 0.0 {
                    let brightness = wave * wave * bass_line_f * 0.7;
                    for &row in &[4, 5] {
                        let led = row * COLS + col;
                        add_color(strip, led, hsv_to_rgb(bass_hue + 10.0, 0.8, brightness));
                    }
                }
            }
        }

        // ============================================
        // LAYER 4: Melody tracker (follows pitch)
        // ============================================
        if a.note_midi > 0 {
            // Map MIDI note to horizontal position: C2(36)→left, C6(84)→right
            let note_col = ((a.note_midi as f32 - 36.0) / 48.0 * COLS as f32)
                .clamp(0.0, (COLS - 1) as f32);
            s.melody_col_target = note_col;
        }
        s.melody_col += (s.melody_col_target - s.melody_col) * 0.2;

        // Drop a trail point
        if a.note_midi > 0 {
            let note_hue = NOTE_HUES[a.note_midi as usize % 12];
            // Vertical position from spectral centroid
            let total: f32 = a.bands.iter().map(|&b| b as f32).sum::<f32>().max(1.0);
            let centroid: f32 = a.bands.iter().enumerate()
                .map(|(i, &b)| i as f32 * b as f32 / total).sum();
            let melody_row = (centroid / 7.0 * (ROWS - 1) as f32).clamp(0.0, (ROWS - 1) as f32);

            if s.melody_trails.len() < 30 {
                s.melody_trails.push(MelodyTrail {
                    col: s.melody_col,
                    row: melody_row,
                    hue: note_hue,
                    life: 0.0,
                });
            }
        }

        // Render melody trails
        let melody_max_life = 25.0;
        for trail in s.melody_trails.iter() {
            let fade = (1.0 - trail.life / melody_max_life).max(0.0);
            let fade = fade * fade;
            let tc = trail.col as usize;
            let tr = trail.row as usize;

            // Glow radius: wider when vocals are present
            let glow_w: usize = if vocals_f > 0.3 { 4 } else { 2 };
            let glow_h: usize = 1;

            for dr in 0..=(glow_h * 2) {
                let row = (tr as isize + dr as isize - glow_h as isize) as usize;
                if row >= ROWS { continue; }
                for dc in 0..=(glow_w * 2) {
                    let col = (tc as isize + dc as isize - glow_w as isize) as usize;
                    if col >= COLS { continue; }
                    let dist_c = (dc as f32 - glow_w as f32).abs() / glow_w as f32;
                    let dist_r = (dr as f32 - glow_h as f32).abs() / (glow_h as f32).max(1.0);
                    let dist = (dist_c * dist_c + dist_r * dist_r).sqrt().min(1.0);
                    let b = (1.0 - dist) * fade;
                    if b > 0.02 {
                        add_color(strip, row * COLS + col, hsv_to_rgb(trail.hue, 0.6, b));
                    }
                }
            }
        }
        s.melody_trails.iter_mut().for_each(|t| t.life += 1.0);
        s.melody_trails.retain(|t| t.life < melody_max_life);

        // ============================================
        // LAYER 5: Snare flashes (middle rows)
        // ============================================
        let snare_onset = a.snare > s.prev_snare.saturating_add(40);
        s.prev_snare = a.snare;
        if snare_onset {
            // Flash across a random-ish span of middle columns
            let center = s.rand() as usize % COLS;
            s.snare_flashes.push(SnareFlash { age: 0.0, col: center });
        }

        for flash in s.snare_flashes.iter() {
            let fade = (1.0 - flash.age / 10.0).max(0.0);
            let width = 8 + (flash.age * 3.0) as usize; // expands as it fades
            let snare_hue = (s.chord_hue + 60.0) % 360.0; // offset from chord

            for &row in &[2, 3] {
                for col in 0..COLS {
                    let dist = ((col as isize) - (flash.col as isize)).unsigned_abs();
                    let dist = dist.min(COLS - dist); // wrap
                    if dist < width {
                        let b = (1.0 - dist as f32 / width as f32) * fade * 0.8;
                        add_color(strip, row * COLS + col, hsv_to_rgb(snare_hue, 0.4, b));
                    }
                }
            }
        }
        s.snare_flashes.iter_mut().for_each(|f| f.age += 1.0);
        s.snare_flashes.retain(|f| f.age < 10.0);

        // ============================================
        // LAYER 6: Hi-hat sparkles (top rows)
        // ============================================
        let spark_count = (hihat_f * 4.0) as usize;
        for _ in 0..spark_count {
            let col = (s.rand() as usize) % COLS;
            let row = (s.rand() as usize) % 2; // top two rows
            let brightness = 0.3 + (s.rand() % 70) as f32 / 100.0;
            let hue = (s.chord_hue + 90.0 + (s.rand() % 40) as f32) % 360.0;
            strip.set(row * COLS + col, hsv_to_rgb(hue, 0.3, brightness));
        }

        // ============================================
        // LAYER 8: Vocal glow (warm center glow)
        // ============================================
        s.vocal_glow += (vocals_f - s.vocal_glow) * 0.1;
        if s.vocal_glow > 0.05 {
            let vocal_hue = (s.chord_hue + 30.0) % 360.0; // warm offset
            let center_col = COLS / 2;
            let glow_radius = 12.0 + s.vocal_glow * 10.0;

            for row in 1..5 { // middle rows
                let row_factor = 1.0 - ((row as f32 - 2.5).abs() / 2.0).min(1.0);
                for col in 0..COLS {
                    let dist = (col as f32 - center_col as f32).abs();
                    if dist < glow_radius {
                        let b = (1.0 - dist / glow_radius) * s.vocal_glow * 0.3 * row_factor;
                        if b > 0.01 {
                            add_color(strip, row * COLS + col,
                                hsv_to_rgb(vocal_hue, 0.25, b));
                        }
                    }
                }
            }
        }
    });
}

/// Lerp between two hues on the shortest path around the color wheel.
fn lerp_hue(a: f32, b: f32, t: f32) -> f32 {
    let mut diff = b - a;
    if diff > 180.0 { diff -= 360.0; }
    else if diff < -180.0 { diff += 360.0; }
    (a + diff * t).rem_euclid(360.0)
}

/// Additive color blend onto an existing LED pixel.
fn add_color(strip: &mut LedStrip, idx: usize, color: [u8; 4]) {
    // Read-back isn't available so we use a simple max blend
    // which works well visually for layered light effects
    let leds = strip.controller_leds();
    let existing = leds[idx];
    strip.set(idx, [
        existing[0].max(color[0]),
        existing[1].max(color[1]),
        existing[2].max(color[2]),
        0,
    ]);
}
