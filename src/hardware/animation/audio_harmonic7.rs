use crate::command::AudioAnalysis;
use crate::hardware::led::{hsv_to_rgb, LedStrip};
use std::cell::RefCell;

const COLS: usize = 60;
const ROWS: usize = 6;
const NOTE_HUES: [f32; 12] = [
    0.0, 30.0, 55.0, 80.0, 120.0, 160.0, 195.0, 220.0, 260.0, 285.0, 320.0, 345.0,
];

// ── State ────────────────────────────────────────────────────

struct TidalState {
    frame: usize,
    // Audio smoothing
    chord_hue: f32,
    chord_hue_target: f32,
    energy: f32,
    bass_smooth: f32,
    mid_smooth: f32,
    treble_smooth: f32,
    beat_brightness: f32,
    // Per-row band energies (6 rows = 6 frequency ranges)
    row_energy: [f32; 6],
    // Flow offset: accumulates over time, speed driven by audio
    flow_x: f32,
    flow_y: f32,
    // Melody tracking: smoothed pitch position (0..1 across strip)
    melody_pos: f32,
    melody_target: f32,
    melody_glow: f32,
    prev_note_midi: u8,
    // Beat ripple
    beat_ripple_x: f32,
    beat_ripple_age: f32,
    prev_kick: u8,
    // BPM
    bpm_smooth: f32,
}

thread_local! {
    static STATE: RefCell<TidalState> = RefCell::new(TidalState {
        frame: 0,
        chord_hue: 180.0,
        chord_hue_target: 180.0,
        energy: 0.3,
        bass_smooth: 0.0,
        mid_smooth: 0.0,
        treble_smooth: 0.0,
        beat_brightness: 0.0,
        row_energy: [0.0; 6],
        flow_x: 0.0,
        flow_y: 0.0,
        melody_pos: 0.5,
        melody_target: 0.5,
        melody_glow: 0.0,
        prev_note_midi: 0,
        beat_ripple_x: 30.0,
        beat_ripple_age: 99.0,
        prev_kick: 0,
        bpm_smooth: 120.0,
    });
}

/// Harmonic Memory V7 — "Tidal"
/// No blocks, no particles. The entire strip is one unified flowing surface.
/// Six rows map to six frequency ranges, creating horizontal strata.
/// Color washes sweep across driven by chord and tempo. Melody creates
/// a traveling glow. Kicks send ripples outward. Everything is smooth.
pub fn audio_harmonic7(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();
        s.frame = s.frame.wrapping_add(1);
        let t = s.frame as f32;

        // ── Unpack audio ──
        let beat_phase = a.beat_phase as f32 / 255.0;
        let bass_f = ((a.bands[0] as u16 + a.bands[1] as u16) / 2) as f32 / 255.0;
        let mid_f = ((a.bands[3] as u16 + a.bands[4] as u16) / 2) as f32 / 255.0;
        let high_f =
            ((a.bands[5] as u16 + a.bands[6] as u16 + a.bands[7] as u16) / 3) as f32 / 255.0;
        let bpm = a.bpm as f32;

        // ── Smooth audio ──
        smooth(
            &mut s.energy,
            a.bands.iter().map(|&b| b as f32).sum::<f32>() / (255.0 * 8.0),
            0.25,
            0.04,
        );
        smooth(&mut s.bass_smooth, bass_f, 0.35, 0.06);
        smooth(&mut s.mid_smooth, mid_f, 0.3, 0.06);
        smooth(&mut s.treble_smooth, high_f, 0.25, 0.08);
        let energy = s.energy.max(0.25);

        // Per-row energy: each row tracks a frequency range
        // Row 0 (top) = treble, Row 5 (bottom) = sub-bass
        let band_targets: [f32; 6] = [
            high_f,
            ((a.bands[5] as u16 + a.bands[6] as u16) / 2) as f32 / 255.0,
            mid_f,
            ((a.bands[2] as u16 + a.bands[3] as u16) / 2) as f32 / 255.0,
            bass_f,
            a.bands[0] as f32 / 255.0,
        ];
        for i in 0..6 {
            smooth(&mut s.row_energy[i], band_targets[i], 0.3, 0.06);
        }

        // ── BPM ──
        if bpm > 30.0 && bpm < 250.0 {
            smooth(&mut s.bpm_smooth, bpm, 0.1, 0.02);
        }
        let tempo_speed = s.bpm_smooth / 120.0;

        // ── Beat ──
        let beat_pulse = ((1.0 - beat_phase) * std::f32::consts::TAU).cos() * 0.5 + 0.5;
        smooth(&mut s.beat_brightness, beat_pulse, 0.35, 0.12);

        // ── Chord hue ──
        if a.chord_root < 12 {
            s.chord_hue_target = NOTE_HUES[a.chord_root as usize];
        }
        s.chord_hue = lerp_hue(s.chord_hue, s.chord_hue_target, 0.05);

        // ── Flow: accumulate position offset driven by tempo + energy ──
        let flow_speed = 0.3 + tempo_speed * 0.5 + energy * 0.3;
        s.flow_x += flow_speed * 0.04;
        s.flow_y += flow_speed * 0.015;

        // ── Melody tracking ──
        if a.note_midi > 0 && a.note_midi != s.prev_note_midi {
            s.melody_target = ((a.note_midi as f32 - 36.0) / 48.0).clamp(0.0, 1.0);
            s.melody_glow = 1.0;
        }
        if a.note_midi > 0 {
            s.prev_note_midi = a.note_midi;
        }
        smooth(&mut s.melody_pos, s.melody_target, 0.08, 0.08);
        s.melody_glow *= 0.97;

        // ── Kick ripple ──
        let kick_onset = a.kick > s.prev_kick.saturating_add(25);
        s.prev_kick = a.kick;
        if kick_onset {
            s.beat_ripple_x = s.melody_pos * COLS as f32;
            s.beat_ripple_age = 0.0;
        }
        s.beat_ripple_age += 0.8 * tempo_speed;

        // ══════════════════════════════════════════════
        // RENDER
        // ══════════════════════════════════════════════
        let melody_col = s.melody_pos * (COLS - 1) as f32;

        for row in 0..ROWS {
            let row_f = row as f32;
            let row_norm = row_f / (ROWS - 1) as f32; // 0=top, 1=bottom
            let row_e = s.row_energy[row].max(0.15);

            // Row-specific flow: lower rows flow slower (heavier), upper faster
            let row_flow = s.flow_x * (1.3 - row_norm * 0.6);
            // Vertical breathing: rows expand/contract with bass
            let row_shift = (s.flow_y + row_f * 0.4).sin() * s.bass_smooth * 0.3;

            for col in 0..COLS {
                let col_f = col as f32;
                let col_norm = col_f / (COLS - 1) as f32; // 0..1

                // ── Base color: flowing gradient ──
                // Horizontal hue gradient shifts with flow
                let hue_base = s.chord_hue
                    + col_f * 1.2                         // spread across strip
                    + row_f * 15.0                        // slight vertical shift
                    + (col_f * 0.08 + row_flow).sin() * 20.0; // undulating offset

                // ── Brightness: layered sine waves ──
                // Wide primary wave
                let wave1 = (col_f / 12.0 + row_flow * 0.15 + row_shift).sin() * 0.5 + 0.5;
                // Narrower secondary wave (counter-flow)
                let wave2 = (col_f / 7.0 - row_flow * 0.1 + row_f * 0.6).sin() * 0.5 + 0.5;
                // Very slow breathing across whole strip
                let wave3 =
                    (col_norm * std::f32::consts::PI + t * 0.008 * tempo_speed).sin() * 0.5 + 0.5;

                let wave_blend = wave1 * 0.45 + wave2 * 0.3 + wave3 * 0.25;

                // ── Row energy modulation ──
                // This row's frequency band controls its brightness range
                let row_brightness = 0.15 + row_e * 0.6 + wave_blend * (0.2 + energy * 0.5);

                // ── Beat breathing: gentle pulse ──
                let beat_mod = 1.0 + s.beat_brightness * 0.2;

                // ── Melody glow: soft gaussian around current note position ──
                let melody_dist = (col_f - melody_col).abs();
                let melody_width = 6.0 + energy * 4.0;
                let melody_brightness = s.melody_glow
                    * (-(melody_dist * melody_dist) / (melody_width * melody_width)).exp()
                    * 0.35;

                // ── Kick ripple: expanding ring from kick origin ──
                let ripple_brightness = if s.beat_ripple_age < 30.0 {
                    let ripple_dist = (col_f - s.beat_ripple_x).abs();
                    let ripple_radius = s.beat_ripple_age * 1.5;
                    let ring_dist = (ripple_dist - ripple_radius).abs();
                    let ring = (1.0 - ring_dist / 4.0).max(0.0);
                    let fade = (1.0 - s.beat_ripple_age / 30.0).max(0.0);
                    ring * fade * 0.25
                } else {
                    0.0
                };

                // ── Combine ──
                let val = (row_brightness * beat_mod + melody_brightness + ripple_brightness)
                    .clamp(0.05, 1.0);

                // Hue: base + slight shift from melody proximity
                let melody_hue_shift = melody_brightness * 40.0;
                let hue = (hue_base + melody_hue_shift).rem_euclid(360.0);

                // Saturation: lower near melody glow (whiter), higher elsewhere
                let sat = (0.55 - melody_brightness * 0.3).clamp(0.2, 0.6);

                let led = row * COLS + col;
                strip.set(led, hsv_to_rgb(hue, sat, val));
            }
        }
    });
}

// ── Helpers ──────────────────────────────────────────────────

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
