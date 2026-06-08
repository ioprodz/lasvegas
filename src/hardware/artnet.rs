//! Art-Net (DMX-over-UDP) output.
//!
//! Mirrors the LED engine's current frame onto a single DMX universe so external
//! lighting systems (first target: QLC+) can be driven by the existing
//! audio-reactive pipeline. Reuses the same config-persistence idiom as
//! `calibration.rs` / `network.rs` and is wired into the 60fps main loop.

use super::led::{hsv_to_rgb, LedStrip};
use crate::command::AudioAnalysis;
use std::fs;
use std::net::UdpSocket;
use std::path::Path;

const ARTNET_FILE: &str = "artnet.json";
const ARTNET_PORT_DEFAULT: u16 = 6454;

/// Max channels in one DMX universe.
pub const DMX_UNIVERSE_SIZE: usize = 512;

// Grid geometry (mirrors led.rs): 60 columns × 6 rows.
const COLS: usize = 60;
const ROWS: usize = 6;
/// DMX channels emitted: 60 columns × 3 (RGB) = 180.
pub const DMX_CHANNELS: usize = COLS * 3;

/// Persisted Art-Net output configuration.
#[derive(Debug, Clone)]
pub struct ArtNetConfig {
    pub enabled: bool,
    pub dest_ip: String, // e.g. "192.168.1.49"
    pub port: u16,       // default 6454
    pub universe: u16,   // 15-bit Art-Net Port-Address (0..=32767)
    pub demo: bool,      // demo/test pattern, independent of audio
    pub rig: bool,       // beat-synced moving-head rig (driven by audio analysis)
}

impl Default for ArtNetConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dest_ip: "192.168.1.49".to_string(),
            port: ARTNET_PORT_DEFAULT,
            universe: 0,
            demo: false,
            rig: false,
        }
    }
}

impl ArtNetConfig {
    pub fn load() -> Self {
        let path = Path::new(ARTNET_FILE);
        if !path.exists() {
            return Self::default();
        }
        match fs::read_to_string(path) {
            Ok(contents) => Self::parse_json(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        fs::write(ARTNET_FILE, self.to_json()).map_err(|e| e.to_string())
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"enabled\":{},\"dest_ip\":\"{}\",\"port\":{},\"universe\":{},\"demo\":{},\"rig\":{}}}",
            self.enabled,
            self.dest_ip.replace('\\', "\\\\").replace('"', "\\\""),
            self.port,
            self.universe,
            self.demo,
            self.rig
        )
    }

    fn parse_json(s: &str) -> Option<Self> {
        Some(Self {
            enabled: extract_bool(s, "enabled").unwrap_or(false),
            dest_ip: extract_string(s, "dest_ip").unwrap_or_else(|| "192.168.1.49".into()),
            port: extract_num(s, "port").unwrap_or(ARTNET_PORT_DEFAULT as f64) as u16,
            universe: extract_num(s, "universe").unwrap_or(0.0) as u16,
            demo: extract_bool(s, "demo").unwrap_or(false),
            rig: extract_bool(s, "rig").unwrap_or(false),
        })
    }

    /// WebSocket text command params: `enabled,dest_ip,port,universe,demo[,rig]`
    /// e.g. `1,192.168.1.49,6454,0,0,1`
    pub fn from_command(params: &str) -> Option<Self> {
        let p: Vec<&str> = params.split(',').collect();
        if p.len() != 5 && p.len() != 6 {
            return None;
        }
        Some(Self {
            enabled: matches!(p[0].trim(), "1" | "true"),
            dest_ip: p[1].trim().to_string(),
            port: p[2].trim().parse().ok()?,
            universe: p[3].trim().parse().ok()?,
            demo: matches!(p[4].trim(), "1" | "true"),
            rig: p.get(5).map(|x| matches!(x.trim(), "1" | "true")).unwrap_or(false),
        })
    }
}

/// Holds the UDP socket and rolling sequence counter for Art-Net transmission.
pub struct ArtNetSender {
    socket: UdpSocket,
    sequence: u8,
}

impl ArtNetSender {
    pub fn new() -> std::io::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        // Harmless; allows a broadcast destination if ever configured.
        let _ = socket.set_broadcast(true);
        Ok(Self { socket, sequence: 1 })
    }

    /// Build one ArtDMX packet for `data` (≤512 channels) and send to dest.
    pub fn send_dmx(
        &mut self,
        dest_ip: &str,
        port: u16,
        universe: u16,
        data: &[u8],
    ) -> std::io::Result<()> {
        let pkt = build_artdmx(self.sequence, universe, data);
        // Sequence wraps 1..=255; 0 means "disable sequencing" per spec.
        self.sequence = self.sequence.wrapping_add(1);
        if self.sequence == 0 {
            self.sequence = 1;
        }
        self.socket.send_to(&pkt, (dest_ip, port))?;
        Ok(())
    }
}

/// Build an ArtDMX (OpDmx) packet. 18-byte header + even-length DMX payload.
/// Pure function — no `rs_ws281x` dependency, so it is unit-testable on any host.
pub fn build_artdmx(sequence: u8, universe: u16, data: &[u8]) -> Vec<u8> {
    let mut len = data.len().min(DMX_UNIVERSE_SIZE);
    if len < 2 {
        len = 2; // spec minimum
    }
    if len % 2 == 1 {
        len += 1; // length must be even
    }

    let mut pkt = Vec::with_capacity(18 + len);

    // 0..8 : ID "Art-Net" + null terminator
    pkt.extend_from_slice(b"Art-Net\0");
    // 8..10 : OpCode 0x5000 (OpDmx), little-endian
    pkt.push(0x00);
    pkt.push(0x50);
    // 10..12 : Protocol version 14, big-endian (Hi, Lo)
    pkt.push(0x00);
    pkt.push(14);
    // 12 : Sequence (0 = disabled; we use 1..=255)
    pkt.push(sequence);
    // 13 : Physical (informational only)
    pkt.push(0x00);
    // 14..16 : 15-bit Port-Address / "Universe", little-endian (Lo, Hi)
    pkt.push((universe & 0xFF) as u8);
    pkt.push(((universe >> 8) & 0x7F) as u8);
    // 16..18 : Length of DMX data, big-endian (Hi, Lo)
    pkt.push(((len >> 8) & 0xFF) as u8);
    pkt.push((len & 0xFF) as u8);
    // 18.. : DMX data, zero-padded to `len`
    let copy = data.len().min(len);
    pkt.extend_from_slice(&data[..copy]);
    pkt.resize(18 + len, 0);

    pkt
}

/// Downsample a flat `[r,g,b]*360` frame (60 cols × 6 rows, row-major, as
/// returned by `LedStrip::read_state`) to 180 DMX channels: per column, average
/// the 6 rows into one RGB triple.
///
/// Output channel layout: ch0=col0.R, ch1=col0.G, ch2=col0.B, ch3=col1.R, ...
pub fn frame_to_dmx(state: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; DMX_CHANNELS];
    if state.len() < COLS * ROWS * 3 {
        return out; // defensive
    }
    for col in 0..COLS {
        let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
        for row in 0..ROWS {
            let base = (row * COLS + col) * 3; // matches led.rs indexing
            r += state[base] as u32;
            g += state[base + 1] as u32;
            b += state[base + 2] as u32;
        }
        out[col * 3] = (r / ROWS as u32) as u8;
        out[col * 3 + 1] = (g / ROWS as u32) as u8;
        out[col * 3 + 2] = (b / ROWS as u32) as u8;
    }
    out
}

/// Audio-independent demo/test pattern: a moving hue sweep across the 60
/// columns (all 6 rows of a column share the column's hue). Visibly changes
/// over time (~full sweep every ~3s @60fps) so QLC+ response is obvious.
pub fn demo_pattern(strip: &mut LedStrip, frame: usize) {
    for col in 0..COLS {
        let hue = ((col as f32 * 6.0) + (frame as f32 * 2.0)) % 360.0;
        let rgb = hsv_to_rgb(hue, 1.0, 1.0);
        for row in 0..ROWS {
            strip.set(row * COLS + col, rgb);
        }
    }
}

// ── All-moving-head rig ────────────────────────────────────────────────
// Every fixture is a moving head (no washes/strobes/hazer): colour, dimming
// and strobe are all done on the movers' own channels.
// Motion: each head carries a phase offset (spread across the rig) so the heads
// point in MANY directions at once — a travelling spread, not a uniform fan —
// with near-full tilt so beams reach horizontal (forward/back), not just up.
// Every few bars ONE group at a time swings to face the audience (FACE_PAN/TILT)
// while the rest keep sweeping, handing off group to group. Most of 540°/180°.
//
// CRITICAL — real movers have a finite slew rate (~180-360°/s pan, ~120-180°/s
// tilt). Commanding faster makes the head lag and hover near the average (the
// "varies in the middle" symptom). So pan/tilt are SLOW free-running sweeps
// (one pan cycle per PAN_SWEEP_BARS bars → ~210°/s) that actually reach the
// rails, and a per-frame slew cap (MAX_PAN_STEP/MAX_TILT_STEP) guarantees the
// output is always physically reachable. The rhythmic punch is carried by the
// DIMMER and STROBE instead, which are instantaneous (no motor latency):
// dimmer has a low floor and snaps to full on energy+kick; the strobe fires a
// short decaying burst on big kicks and as a build at the end of each 4-bar
// phrase. Colour follows the chord (mirror twins share a hue → symmetric fan).
//
// DMX map (must match the QLC+ patch in ../dmx-stage/stage01.qxw):
//   movers : RIG_MOVERS x 9ch  (Betopper LM108: Pan,Tilt,Dim,R,G,B,White,Strobe,P/T-speed)
pub const RIG_MOVERS: usize = 24;
const RIG_MOVER_CH: usize = 9;
const MOVERS_PER_ROW: usize = 8; // RIG_MOVERS / MOVERS_PER_ROW = 3 trusses
pub const RIG_MOVERS_ADDR: usize = 0;

// The main loop sends Art-Net at ~63 Hz (measured on the wire). One "bar" at
// TARGET_BPM is the musical time base; sweeps are expressed in bars so they
// stay tempo-relative.
const SEND_HZ: f32 = 63.0;
const TARGET_BPM: f32 = 100.0;
const BAR_T_PER_FRAME: f32 = TARGET_BPM / (240.0 * SEND_HZ); // bars advanced per frame
const PAN_AMP: f32 = 0.45; // ±0.45 → ~485° of the 540° pan range
const TILT_AMP: f32 = 0.45; // ±0.45 → near-full 180° tilt (beams reach horizontal)
// Slow, slew-safe sweep periods (bars per full cycle). Peak angular velocity of
// an A·sin(2πt/T) sweep is A·2π/T: pan ≈ 243°·2π/(3·2.4s) ≈ 210°/s, tilt ≈
// 81°·2π/(2·2.4s) ≈ 106°/s — both within a real fixture's reach.
const PAN_SWEEP_BARS: f32 = 3.0;
const TILT_SWEEP_BARS: f32 = 2.0;
// Per-head phase offset spreads the rig across the sweep so the heads point in
// many directions at once (a travelling wave) instead of moving in lockstep.
const SPREAD_TURNS: f32 = 1.25;
// Periodic unison "face the audience" moment: every FACE_EVERY_BARS the whole
// rig swings to a single pose AIMED AT THE CROWD and holds it steady. Pan-center
// (0.5) faces the BACK of stage, so the front azimuth is ~180° away ≈ 0.83 (or
// 0.17 — same direction on a 540° head). FACE_TILT brings the beam down/forward.
// CALIBRATE both by eye from a screenshot taken DURING the hold.
// (Frequent + long for calibration; relax FACE_EVERY_BARS once it's dialed in.)
const FACE_EVERY_BARS: f32 = 4.0;
const FACE_WINDOW_BARS: f32 = 2.0;
const FACE_GROUPS: usize = 2; // only one group faces the crowd at a time (rotates)
const FACE_PAN: f32 = 0.663; // azimuth toward the audience/front (CALIBRATE; 0.997 = other way)
const FACE_TILT: f32 = 0.95; // elevation down/forward at the audience (CALIBRATE)
const PHASE_WRAP: f32 = 24.0; // common multiple of all periods → seamless loop
const STROBE_PHRASE_BARS: f32 = 1.0; // strobe build fires every N bars, alternating groups
const STROBE_GROUPS: usize = 2; // heads strobe in interleaved groups, not all at once
// Per-frame slew cap (DMX units/frame). At ~62 Hz: 2/frame ≈ 265°/s pan,
// 3/frame ≈ 132°/s tilt — the output can never demand more than the motor can do.
const MAX_PAN_STEP: f32 = 2.0;
const MAX_TILT_STEP: f32 = 3.0;
const NOTE_HUES: [f32; 12] = [
    0.0, 30.0, 55.0, 80.0, 120.0, 160.0, 195.0, 220.0, 260.0, 285.0, 320.0, 345.0,
];

struct RigState {
    phase: f32,    // free-running musical time, in bars (wraps at PHASE_WRAP)
    bass: f32,
    mid: f32,
    strobe: Vec<f32>,  // per-group strobe burst envelope (0..1), decays each frame
    kick_group: usize, // which group the next kick flashes
    prev_kick: bool,   // rising-edge detection for the kick
    pan: Vec<f32>,  // last commanded pan per mover (0..1), for slew limiting
    tilt: Vec<f32>, // last commanded tilt per mover (0..1)
}

thread_local! {
    static RIG: std::cell::RefCell<RigState> = std::cell::RefCell::new(RigState {
        phase: 0.0,
        bass: 0.0,
        mid: 0.0,
        strobe: vec![0.0; STROBE_GROUPS],
        kick_group: STROBE_GROUPS - 1, // so the first kick lands on group 0
        prev_kick: false,
        pan: vec![0.5; RIG_MOVERS],
        tilt: vec![0.5; RIG_MOVERS],
    });
}

/// Slow pan/tilt TARGET for mover `i` at musical time `phase` (bars), normalized
/// to [0, 1]. Each head carries a phase offset (SPREAD_TURNS across the rig) so
/// the heads point in many directions at once rather than in lockstep, with
/// near-full tilt so beams reach horizontal. Every FACE_EVERY_BARS the whole rig
/// converges to a unison low/forward pose and rakes across the audience. Sweeps
/// are slow enough to stay within slew; the caller also slew-limits the output.
fn mover_target(i: usize, phase: f32) -> (f32, f32) {
    let two_pi = std::f32::consts::TAU;
    let off = (i as f32 / RIG_MOVERS as f32) * SPREAD_TURNS * two_pi; // per-head spread
    let mut pan = 0.5 + PAN_AMP * (two_pi * phase / PAN_SWEEP_BARS + off).sin();
    let mut tilt = 0.5 + TILT_AMP * (two_pi * phase / TILT_SWEEP_BARS + off * 0.6).sin();

    // "Face the audience" moment near the end of every FACE_EVERY_BARS — but only
    // ONE group at a time: the facing group swings to (FACE_PAN, FACE_TILT) and
    // holds it while the rest keep sweeping. Each moment hands off to the next group.
    let face_group = (phase / FACE_EVERY_BARS) as usize % FACE_GROUPS;
    let cyc = phase.rem_euclid(FACE_EVERY_BARS);
    if cyc > FACE_EVERY_BARS - FACE_WINDOW_BARS && i % FACE_GROUPS == face_group {
        let x = (cyc - (FACE_EVERY_BARS - FACE_WINDOW_BARS)) / FACE_WINDOW_BARS; // 0..1
        // ramp in, hold steady in the middle, ramp out (clear to observe)
        let blend = if x < 0.25 {
            x / 0.25
        } else if x > 0.75 {
            (1.0 - x) / 0.25
        } else {
            1.0
        };
        pan = pan * (1.0 - blend) + FACE_PAN * blend;
        tilt = tilt * (1.0 - blend) + FACE_TILT * blend;
    }
    (pan.clamp(0.0, 1.0), tilt.clamp(0.0, 1.0))
}

/// Symmetric chord-shifted palette: mirror-twin columns share a hue, so the
/// rig reads as a colour fan that re-tints with the detected chord root.
fn mover_hue(i: usize, chord_root: u8) -> f32 {
    let base = if chord_root < 12 {
        NOTE_HUES[chord_root as usize]
    } else {
        200.0
    };
    let col = i % MOVERS_PER_ROW;
    let pair = col.min(MOVERS_PER_ROW - 1 - col) as f32; // 0..(rows/2) symmetric
    let pairs = (MOVERS_PER_ROW as f32 / 2.0).max(1.0);
    (base + pair * (360.0 / pairs)).rem_euclid(360.0)
}

/// Build the moving-head rig universe. Pan/tilt are slow, slew-limited sweeps
/// (two opposed groups) that reach the rails; the beat is carried by the dimmer
/// (low floor, snaps to full on energy+kick) and the strobe (flash on kick);
/// colour follows the chord. Stateful (per-thread): advances musical phase and
/// remembers each head's last pan/tilt so the output can be slew-capped.
pub fn rig_to_dmx(a: &AudioAnalysis, _frame: usize) -> Vec<u8> {
    RIG.with(|st| {
        let s = &mut *st.borrow_mut();
        let mut dmx = vec![0u8; DMX_UNIVERSE_SIZE];

        // Smooth bass/mid and advance musical phase. Keep the tempo modulation
        // gentle so the sweeps stay slow enough for the motors.
        let bass = (a.bands[0] as f32 + a.bands[1] as f32) / 2.0 / 255.0;
        let mid = (a.bands[3] as f32 + a.bands[4] as f32) / 2.0 / 255.0;
        smooth(&mut s.bass, bass, 0.4, 0.08);
        smooth(&mut s.mid, mid, 0.35, 0.08);
        let speed = 1.0 + s.mid * 0.15 + s.bass * 0.08; // gentle tempo breathing
        s.phase = (s.phase + BAR_T_PER_FRAME * speed).rem_euclid(PHASE_WRAP);

        let energy = a.bands.iter().map(|&b| b as f32).sum::<f32>() / (8.0 * 255.0);
        let kick = a.kick as f32 / 255.0;
        // Punchy dimmer: low ambient floor, energy + kick add toward full so the
        // rhythm reads on intensity (instant) rather than on the slow motors.
        let dim = (0.30 + 0.50 * energy + 0.40 * kick).min(1.0);
        // Grouped strobe (preset ShutterStrobeSlowFast: 0=open, higher=faster
        // strobe). The heads split into STROBE_GROUPS interleaved groups that fire
        // at DIFFERENT times instead of all together: the phrase build chases from
        // group to group, and each kick advances to the next group. Each group
        // holds a decaying burst envelope (a 1-frame pulse wouldn't show).
        let kick_hi = kick > 0.55;
        if kick_hi && !s.prev_kick {
            s.kick_group = (s.kick_group + 1) % STROBE_GROUPS; // rising edge → next group
        }
        s.prev_kick = kick_hi;
        let phrase_accent = s.phase.rem_euclid(STROBE_PHRASE_BARS) > STROBE_PHRASE_BARS - 0.25;
        let accent_group = (s.phase / STROBE_PHRASE_BARS) as usize % STROBE_GROUPS;
        for g in 0..STROBE_GROUPS {
            let fire = (kick_hi && g == s.kick_group) || (phrase_accent && g == accent_group);
            if fire {
                s.strobe[g] = 1.0;
            } else {
                s.strobe[g] *= 0.88;
            }
        }
        let pan_step = MAX_PAN_STEP / 255.0;
        let tilt_step = MAX_TILT_STEP / 255.0;

        for i in 0..RIG_MOVERS {
            let base = RIG_MOVERS_ADDR + i * RIG_MOVER_CH;
            let (tp, tt) = mover_target(i, s.phase);
            // Slew-limit toward the target so the command is always reachable.
            s.pan[i] += (tp - s.pan[i]).clamp(-pan_step, pan_step);
            s.tilt[i] += (tt - s.tilt[i]).clamp(-tilt_step, tilt_step);
            let (r, g, b) = hsv(mover_hue(i, a.chord_root), 0.9, 1.0);
            // Per-group strobe: this head flashes only when its group is firing.
            let sg = s.strobe[i % STROBE_GROUPS];
            let strobe = if sg > 0.15 {
                (150.0 + 80.0 * sg).min(255.0) as u8 // fast-strobe range
            } else {
                0 // open / steady
            };
            dmx[base] = (s.pan[i] * 255.0) as u8;
            dmx[base + 1] = (s.tilt[i] * 255.0) as u8;
            dmx[base + 2] = (dim * 255.0) as u8;
            dmx[base + 3] = r;
            dmx[base + 4] = g;
            dmx[base + 5] = b;
            dmx[base + 6] = 0; // White
            dmx[base + 7] = strobe; // grouped strobe (by interleaved group)
            // P/T speed preset is SpeedPanTiltSlowFast (0=slowest, 255=fastest).
            // Drive it FASTEST so the head follows our DMX directly — our own
            // per-frame slew cap is what shapes the smooth motion. (0 here pinned
            // the motor to its slowest, making it lag toward the average.)
            dmx[base + 8] = 255;
        }

        dmx
    })
}

fn smooth(current: &mut f32, target: f32, attack: f32, decay: f32) {
    if target > *current {
        *current += (target - *current) * attack;
    } else {
        *current += (target - *current) * decay;
    }
}

/// Local HSV→RGB (0..360, 0..1, 0..1). Kept dependency-free so `rig_to_dmx`
/// stays unit-testable without the LED hardware backend.
fn hsv(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = ((h % 360.0) + 360.0) % 360.0;
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as u16 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

// ── JSON parsing helpers (kept module-local, matching the per-config
//    self-contained convention used elsewhere in src/hardware) ──────────

fn extract_string(s: &str, key: &str) -> Option<String> {
    let pat = format!("\"{}\"", key);
    let idx = s.find(&pat)?;
    let rest = &s[idx + pat.len()..];
    let colon = rest.find(':')?;
    let after = rest[colon + 1..].trim_start();
    let after = after.strip_prefix('"')?;
    let end = after.find('"')?;
    Some(after[..end].to_string())
}

fn extract_num(s: &str, key: &str) -> Option<f64> {
    let pat = format!("\"{}\"", key);
    let idx = s.find(&pat)?;
    let rest = &s[idx + pat.len()..];
    let colon = rest.find(':')?;
    let after = rest[colon + 1..].trim_start();
    let end = after
        .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
        .unwrap_or(after.len());
    after[..end].parse().ok()
}

fn extract_bool(s: &str, key: &str) -> Option<bool> {
    let pat = format!("\"{}\"", key);
    let idx = s.find(&pat)?;
    let rest = &s[idx + pat.len()..];
    let colon = rest.find(':')?;
    let after = rest[colon + 1..].trim_start();
    if after.starts_with("true") {
        Some(true)
    } else if after.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artdmx_header_layout() {
        let data = [1u8, 2, 3, 4];
        let pkt = build_artdmx(7, 0, &data);
        // ID
        assert_eq!(&pkt[0..8], b"Art-Net\0");
        // OpCode 0x5000 little-endian
        assert_eq!(&pkt[8..10], &[0x00, 0x50]);
        // ProtVer 14 big-endian
        assert_eq!(&pkt[10..12], &[0x00, 0x0E]);
        // sequence / physical
        assert_eq!(pkt[12], 7);
        assert_eq!(pkt[13], 0);
        // length big-endian == 4
        assert_eq!(&pkt[16..18], &[0x00, 0x04]);
        assert_eq!(pkt.len(), 18 + 4);
        assert_eq!(&pkt[18..22], &data);
    }

    #[test]
    fn artdmx_universe_little_endian_15bit() {
        // universe 0x1234 -> lo=0x34, hi=0x12 (masked to 7 bits)
        let pkt = build_artdmx(1, 0x1234, &[0u8; 2]);
        assert_eq!(pkt[14], 0x34);
        assert_eq!(pkt[15], 0x12);
    }

    #[test]
    fn artdmx_length_even_and_clamped() {
        // odd length rounds up to even
        let pkt = build_artdmx(1, 0, &[9u8; 3]);
        assert_eq!(&pkt[16..18], &[0x00, 0x04]);
        assert_eq!(pkt.len(), 18 + 4);
        assert_eq!(pkt[21], 0); // padded byte

        // over-512 clamps to 512
        let big = vec![1u8; 600];
        let pkt = build_artdmx(1, 0, &big);
        assert_eq!(&pkt[16..18], &[0x02, 0x00]); // 512
        assert_eq!(pkt.len(), 18 + 512);
    }

    #[test]
    fn frame_to_dmx_averages_columns() {
        // Build a frame where column 0 has rows valued 0,10,20,30,40,50 in R.
        let mut state = vec![0u8; COLS * ROWS * 3];
        for row in 0..ROWS {
            let base = (row * COLS + 0) * 3;
            state[base] = (row * 10) as u8;
        }
        let dmx = frame_to_dmx(&state);
        assert_eq!(dmx.len(), DMX_CHANNELS);
        // average of 0,10,20,30,40,50 = 25
        assert_eq!(dmx[0], 25);
        assert_eq!(dmx[1], 0);
        assert_eq!(dmx[2], 0);
    }

    #[test]
    fn rig_outputs_512_and_moves() {
        let mut a = AudioAnalysis::default();
        a.bands = [200; 8];
        a.bpm = 128;
        a.beat_phase = 64;
        a.kick = 200;
        a.chord_root = 5;
        let d = rig_to_dmx(&a, 10);
        assert_eq!(d.len(), DMX_UNIVERSE_SIZE);
        assert!(d[2] > 200, "dimmer should track energy/kick");
        assert!(d[7] > 150, "mover 0's strobe group should flash on the kick");
        assert_eq!(d[9 + 7], 0, "the other strobe group should NOT flash on the same kick");
        assert!(RIG_MOVERS * RIG_MOVER_CH <= DMX_UNIVERSE_SIZE, "rig fits one universe");
    }

    #[test]
    fn rig_moves_each_frame() {
        // The bar advances per call, so the heads' tilt (Strobubble vertical
        // bounce) changes between successive frames even with silent input.
        let a = AudioAnalysis::default();
        let d0 = rig_to_dmx(&a, 0);
        let d1 = rig_to_dmx(&a, 1);
        assert_eq!(d0.len(), DMX_UNIVERSE_SIZE);
        assert_ne!(d0[1], d1[1], "mover tilt should advance each frame");
    }

    #[test]
    fn frame_to_dmx_short_input_is_safe() {
        let dmx = frame_to_dmx(&[1, 2, 3]);
        assert_eq!(dmx.len(), DMX_CHANNELS);
        assert!(dmx.iter().all(|&b| b == 0));
    }
}
