use crate::hardware::led::{hsv_to_rgb, LedStrip};
use std::cell::RefCell;

const COLS: usize = 60;
const ROWS: usize = 6;

const CHASE_PHASE: [usize; 6] = [0, 37, 17, 53, 8, 44];
const PULSE_PHASE: [f32; 6] = [0.0, 1.05, 2.09, 3.14, 4.19, 5.24];
const MAX_SPARKS: usize = 20;
const MAX_RIPPLES: usize = 4;

// Valid block counts (all divide 60 evenly)
const VALID_COUNTS: [usize; 6] = [1, 2, 3, 4, 5, 6];

// ---- Spectral fingerprint ----
// We compute a volume-independent spectral shape by normalizing the 8
// bands into a distribution (each band / total). From that we derive:
//
//   centroid  — weighted average band index (0=sub-bass, 7=treble).
//               Low when bass-heavy, high when bright/vocal.
//   spread    — how distributed vs concentrated the energy is.
//               Low for solo instruments, high for full mix.
//   bass_ratio — proportion of energy in bands 0-1.
//
// These are smoothed slowly to represent the current musical "section".
// When the fast (instantaneous) values diverge from the slow (section)
// values beyond a threshold, a section change is detected — this
// naturally fires on verse→chorus, instrument entries, solos, etc.
//
// The layout (block count + permutation) is derived deterministically
// from the spectral shape, so the same frequency content always produces
// the same visual layout.

const SECTION_CHANGE_THRESHOLD: f32 = 0.35;
const SECTION_COOLDOWN_FRAMES: usize = 45; // ~750ms at 60fps

struct Spark {
    col_frac: f32,
    row: usize,
    life: f32,
    max_life: f32,
    hue: f32,
}

struct Ripple {
    age: f32,
    hue: f32,
}

struct HybridState {
    // Spectral tracking
    slow_centroid: f32,
    slow_spread: f32,
    slow_bass_ratio: f32,
    section_cooldown: usize,
    // Beat detection for ripples
    prev_bass: u8,
    beat_cooldown: usize,
    // Current layout (derived from spectral fingerprint)
    active_count: usize,
    perm: [usize; 6],
    // Particles
    sparks: Vec<Spark>,
    ripples: Vec<Ripple>,
    energy_smooth: f32,
    seed: u32,
}

impl HybridState {
    fn rand(&mut self) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed
    }
}

thread_local! {
    static STATE: RefCell<HybridState> = RefCell::new(HybridState {
        slow_centroid: 3.5,
        slow_spread: 0.5,
        slow_bass_ratio: 0.3,
        section_cooldown: 0,
        prev_bass: 0,
        beat_cooldown: 0,
        active_count: 6,
        perm: [0, 1, 2, 3, 4, 5],
        sparks: Vec::new(),
        ripples: Vec::new(),
        energy_smooth: 0.0,
        seed: 7919,
    });
}

/// Compute volume-independent spectral features from the 8 bands.
fn spectral_features(bands: &[u8; 8]) -> (f32, f32, f32, f32) {
    let total: f32 = bands.iter().map(|&b| b as f32).sum::<f32>().max(1.0);

    // Normalized distribution
    let norm: [f32; 8] = std::array::from_fn(|i| bands[i] as f32 / total);

    // Centroid: weighted average band index (0–7)
    let centroid: f32 = norm.iter().enumerate()
        .map(|(i, &n)| i as f32 * n)
        .sum();

    // Spread: standard deviation around centroid
    let variance: f32 = norm.iter().enumerate()
        .map(|(i, &n)| {
            let d = i as f32 - centroid;
            d * d * n
        })
        .sum();
    let spread = variance.sqrt();

    // Bass ratio: energy in bands 0-1 relative to total
    let bass_ratio = norm[0] + norm[1];

    (centroid, spread, bass_ratio, total)
}

/// Deterministically map spectral shape to a block count (1–6).
/// Bass-heavy + concentrated → fewer, larger blocks.
/// Bright + spread → more, smaller blocks.
fn count_from_spectrum(centroid: f32, spread: f32, bass_ratio: f32) -> usize {
    // Combine features into a single 0–1 "complexity" score
    // High centroid (bright) → more blocks
    // High spread (full mix) → more blocks
    // High bass ratio (bass-heavy) → fewer blocks
    let complexity = (centroid / 7.0) * 0.4
        + (spread / 3.0).min(1.0) * 0.3
        + (1.0 - bass_ratio) * 0.3;

    // Map 0–1 to 1–6
    let idx = (complexity * 5.99).clamp(0.0, 5.99) as usize;
    VALID_COUNTS[idx]
}

/// Deterministically map spectral shape to a block-type permutation.
/// Uses a hash of the quantized spectral features so the same musical
/// content always produces the same arrangement.
fn perm_from_spectrum(centroid: f32, spread: f32, bass_ratio: f32) -> [usize; 6] {
    // Quantize features to reduce jitter
    let c = (centroid * 4.0) as u32;
    let s = (spread * 4.0) as u32;
    let b = (bass_ratio * 4.0) as u32;
    // Simple deterministic hash
    let hash = c.wrapping_mul(7)
        ^ s.wrapping_mul(13)
        ^ b.wrapping_mul(19);

    // Fisher-Yates shuffle seeded by hash
    let mut perm = [0usize, 1, 2, 3, 4, 5];
    let mut h = hash;
    for i in (1..6).rev() {
        h = h.wrapping_mul(1664525).wrapping_add(1013904223); // LCG
        let j = (h as usize) % (i + 1);
        perm.swap(i, j);
    }
    perm
}

/// Hybrid — variable blocks with spectral-fingerprint-driven layout.
/// Same frequency content → same layout. Transitions fire on musical
/// section changes (spectral shape shift), not random beats.
pub fn audio_hybrid(strip: &mut LedStrip, frame: usize, bands: &[u8; 8]) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();

        let bass_f = ((bands[0] as u16 + bands[1] as u16) / 2) as f32 / 255.0;
        let treble_f = ((bands[6] as u16 + bands[7] as u16) / 2) as f32 / 255.0;
        let mid_f = ((bands[3] as u16 + bands[4] as u16) / 2) as f32 / 255.0;
        let high_f = ((bands[5] as u16 + bands[6] as u16 + bands[7] as u16) / 3) as f32 / 255.0;
        let bass_u = ((bands[0] as u16 + bands[1] as u16) / 2) as usize;

        // ---- Spectral fingerprint ----
        let (centroid, spread, bass_ratio, total) = spectral_features(bands);

        // Update slow-tracking (represents current "section" character)
        const SLOW_RATE: f32 = 0.005; // very slow adaptation
        s.slow_centroid += (centroid - s.slow_centroid) * SLOW_RATE;
        s.slow_spread += (spread - s.slow_spread) * SLOW_RATE;
        s.slow_bass_ratio += (bass_ratio - s.slow_bass_ratio) * SLOW_RATE;

        s.section_cooldown = s.section_cooldown.saturating_sub(1);

        // Detect section change: how far is the current spectrum from
        // the slow-tracked "section" character?
        let centroid_diff = ((centroid - s.slow_centroid) / 7.0).abs();
        let spread_diff = ((spread - s.slow_spread) / 3.0).abs();
        let bass_diff = (bass_ratio - s.slow_bass_ratio).abs();
        let divergence = centroid_diff + spread_diff + bass_diff;

        let section_changed = s.section_cooldown == 0
            && divergence > SECTION_CHANGE_THRESHOLD
            && total > 50.0; // ignore near-silence

        if section_changed {
            // Snap slow trackers to current values (new section baseline)
            s.slow_centroid = centroid;
            s.slow_spread = spread;
            s.slow_bass_ratio = bass_ratio;
            s.section_cooldown = SECTION_COOLDOWN_FRAMES;

            // Derive new layout deterministically from spectral shape
            s.active_count = count_from_spectrum(centroid, spread, bass_ratio);
            s.perm = perm_from_spectrum(centroid, spread, bass_ratio);

            // Spawn a ripple on section change
            if s.ripples.len() < MAX_RIPPLES {
                // Hue from centroid — deterministic
                let hue = (centroid / 7.0 * 360.0) % 360.0;
                s.ripples.push(Ripple { age: 0.0, hue });
            }
        }

        // ---- Beat detection for ripples ----
        let bass_raw = ((bands[0] as u16 + bands[1] as u16) / 2) as u8;
        let bass_delta = bass_raw.saturating_sub(s.prev_bass);
        s.prev_bass = bass_raw;
        s.beat_cooldown = s.beat_cooldown.saturating_sub(1);

        if bass_delta > 15 && s.beat_cooldown == 0 && s.ripples.len() < MAX_RIPPLES {
            s.beat_cooldown = 6; // ~100ms cooldown
            // Hue derived from current spectral centroid — deterministic
            let hue = (centroid / 7.0 * 360.0 + spread * 40.0) % 360.0;
            s.ripples.push(Ripple { age: 0.0, hue });
        }

        let active_count = s.active_count;
        let block_width = COLS / active_count;
        let block_leds = block_width * ROWS;
        let perm = s.perm;

        // Sparkle: spawn based on high freq
        let spawn_count = (high_f * 3.0) as usize;
        for _ in 0..spawn_count {
            if s.sparks.len() >= MAX_SPARKS { break; }
            let col_frac = (s.rand() % 1000) as f32 / 1000.0;
            let row = (s.rand() as usize) % ROWS;
            let max_life = 8.0 + (s.rand() % 20) as f32;
            let hue = (mid_f * 360.0 + (s.rand() % 60) as f32 - 30.0).rem_euclid(360.0);
            s.sparks.push(Spark { col_frac, row, life: 0.0, max_life, hue });
        }

        // Energy bar smoothing
        if mid_f > s.energy_smooth {
            s.energy_smooth += (mid_f - s.energy_smooth) * 0.35;
        } else {
            s.energy_smooth += (mid_f - s.energy_smooth) * 0.1;
        }

        // Dominant band for chase color
        let mut max_band = 0usize;
        let mut max_val = 0u8;
        for (i, &v) in bands.iter().enumerate() {
            if v > max_val { max_val = v; max_band = i; }
        }
        let chase_hue = (max_band as f32) * 360.0 / 8.0;
        let chase_color = hsv_to_rgb(chase_hue, 1.0, 1.0);
        let chase_speed = 1 + bass_u / 32;

        strip.set_all([0, 0, 0, 0]);

        for slot in 0..active_count {
            let col_start = slot * block_width;
            let block_type = perm[slot % 6] % 6;

            match block_type {
                0 => {
                    // ---- PULSE ----
                    let phase = PULSE_PHASE[slot % 6];
                    let pulse_mod = (frame as f32 * 0.08 + phase).sin() * 0.5 + 0.5;
                    let brightness = bass_f * (0.4 + pulse_mod * 0.6);
                    let hue = (treble_f * 360.0 + slot as f32 * 30.0) % 360.0;
                    let color = hsv_to_rgb(hue, 1.0, brightness);
                    fill_block(strip, col_start, block_width, color);
                }
                1 => {
                    // ---- HORIZONTAL CHASE ----
                    render_chase(strip, frame, col_start, block_width, block_leds,
                                 slot, chase_speed, &chase_color, false);
                }
                2 => {
                    // ---- SPARKLE ----
                    let bg = hsv_to_rgb(bass_f * 30.0, 0.6, bass_f * 0.12);
                    fill_block(strip, col_start, block_width, bg);
                    for spark in s.sparks.iter() {
                        let spark_col = (spark.col_frac * block_width as f32) as usize;
                        let spark_col = spark_col.min(block_width - 1);
                        let t = spark.life / spark.max_life;
                        let brightness = if t < 0.15 {
                            t / 0.15
                        } else {
                            (1.0 - (t - 0.15) / 0.85).max(0.0)
                        };
                        let brightness = brightness * brightness;
                        let color = hsv_to_rgb(spark.hue, 0.7, brightness);
                        let led = spark.row * COLS + col_start + spark_col;
                        strip.set(led, color);
                    }
                }
                3 => {
                    // ---- VERTICAL CHASE ----
                    render_chase(strip, frame, col_start, block_width, block_leds,
                                 slot, chase_speed, &chase_color, true);
                }
                4 => {
                    // ---- ENERGY BAR ----
                    let energy = s.energy_smooth;
                    let half = block_width / 2;
                    let bar_half = (energy * half as f32) as usize;
                    let hue = 150.0 + mid_f * 60.0;

                    for row in 0..ROWS {
                        for col_off in 0..block_width {
                            let led = row * COLS + col_start + col_off;
                            let dist = if col_off >= half {
                                col_off - half
                            } else {
                                half - 1 - col_off
                            };
                            if dist < bar_half {
                                let t = dist as f32 / bar_half.max(1) as f32;
                                let val = 1.0 - t * 0.4;
                                strip.set(led, hsv_to_rgb(hue, 0.85, val));
                            } else if dist < bar_half + 2 {
                                let glow = 0.1 * (1.0 - (dist - bar_half) as f32 / 2.0);
                                strip.set(led, hsv_to_rgb(hue, 0.5, glow));
                            } else {
                                strip.set(led, [0, 0, 0, 0]);
                            }
                        }
                    }
                }
                5 => {
                    // ---- RIPPLE ----
                    let cx = block_width as f32 / 2.0;
                    let cy = ROWS as f32 / 2.0;
                    let max_radius = (block_width as f32 / 2.0) + 2.0;

                    for ripple in s.ripples.iter() {
                        let radius = ripple.age * 0.8;
                        let fade = (1.0 - ripple.age / 50.0).max(0.0);
                        let fade = fade * fade;
                        let ring_w = 1.5 + ripple.age * 0.15;
                        let color = hsv_to_rgb(ripple.hue, 0.9, 1.0);

                        for row in 0..ROWS {
                            for col_off in 0..block_width {
                                let dx = col_off as f32 - cx;
                                let dy = row as f32 - cy;
                                let dist = (dx * dx + dy * dy).sqrt();
                                let ring_dist = (dist - radius).abs();
                                if ring_dist < ring_w && dist < max_radius {
                                    let b = (1.0 - ring_dist / ring_w) * fade;
                                    let led = row * COLS + col_start + col_off;
                                    strip.set(led, [
                                        (color[0] as f32 * b) as u8,
                                        (color[1] as f32 * b) as u8,
                                        (color[2] as f32 * b) as u8,
                                        0,
                                    ]);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Age / cull particles
        s.sparks.iter_mut().for_each(|sp| sp.life += 1.0);
        s.sparks.retain(|sp| sp.life < sp.max_life);
        s.ripples.iter_mut().for_each(|r| r.age += 1.0);
        s.ripples.retain(|r| r.age < 50.0);
    });
}

fn fill_block(strip: &mut LedStrip, col_start: usize, width: usize, color: [u8; 4]) {
    for row in 0..ROWS {
        for col in col_start..(col_start + width) {
            strip.set(row * COLS + col, color);
        }
    }
}

fn render_chase(
    strip: &mut LedStrip,
    frame: usize,
    col_start: usize,
    block_width: usize,
    block_leds: usize,
    slot: usize,
    chase_speed: usize,
    chase_color: &[u8; 4],
    vertical: bool,
) {
    let phase = CHASE_PHASE[slot % 6];
    let pos = ((frame + phase) * chase_speed) % block_leds;
    let tail_len = (block_leds / 5).max(6);

    for row in 0..ROWS {
        for col_off in 0..block_width {
            let local_idx = if vertical {
                col_off * ROWS + row
            } else {
                row * block_width + col_off
            };
            let led_idx = row * COLS + col_start + col_off;

            let dist = ((local_idx as isize) - (pos as isize)).unsigned_abs() % block_leds;
            let dist = dist.min(block_leds - dist);

            if dist < tail_len {
                let fade = 1.0 - (dist as f32 / tail_len as f32);
                strip.set(led_idx, [
                    (chase_color[0] as f32 * fade) as u8,
                    (chase_color[1] as f32 * fade) as u8,
                    (chase_color[2] as f32 * fade) as u8,
                    0,
                ]);
            }
        }
    }
}
