//! "Listen" features built on the hardware mic:
//!
//! - `ClapDetector`: spots a double clap in the raw PCM stream so the lights
//!   can be toggled by hand-clapping.
//! - Audio tap: re-publishes the raw capture stream (S16_LE, 44.1 kHz, mono)
//!   on a localhost TCP port, so other programs on the Pi (the voice-command
//!   service in `ears/`) can listen without fighting over the ALSA device.
//! - `ListenConfig`: persisted settings (clap on/off, sensitivity, and the mic
//!   to open automatically at boot).

use std::fs;
use std::io::{ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::thread;

const LISTEN_FILE: &str = "listen.json";
pub const TAP_ADDR: &str = "127.0.0.1:8081";
const SENSITIVITY_DEFAULT: u8 = 5;

// ── Config ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ListenConfig {
    /// Double clap toggles the lights.
    pub clap: bool,
    /// 1 (only sharp, loud claps) … 10 (reacts to softer claps).
    pub sensitivity: u8,
    /// Hardware audio device to open at startup ("" = none). Set whenever a
    /// client starts hardware audio, cleared when a client stops it.
    pub mic: String,
}

impl Default for ListenConfig {
    fn default() -> Self {
        Self { clap: false, sensitivity: SENSITIVITY_DEFAULT, mic: String::new() }
    }
}

impl ListenConfig {
    pub fn load() -> Self {
        let path = Path::new(LISTEN_FILE);
        if !path.exists() {
            return Self::default();
        }
        match fs::read_to_string(path) {
            Ok(contents) => Self::parse_json(&contents),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        fs::write(LISTEN_FILE, self.to_json()).map_err(|e| e.to_string())
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"clap\":{},\"sensitivity\":{},\"mic\":\"{}\"}}",
            self.clap,
            self.sensitivity,
            self.mic.replace('\\', "\\\\").replace('"', "\\\"")
        )
    }

    fn parse_json(s: &str) -> Self {
        Self {
            clap: extract_bool(s, "clap").unwrap_or(false),
            sensitivity: extract_num(s, "sensitivity")
                .map(|v| (v as u8).clamp(1, 10))
                .unwrap_or(SENSITIVITY_DEFAULT),
            mic: extract_string(s, "mic").unwrap_or_default(),
        }
    }

    /// WebSocket text command params: `clap,sensitivity`, e.g. `1,5`.
    /// Leaves `mic` untouched (the caller merges it in).
    pub fn apply_command(&mut self, params: &str) -> bool {
        let p: Vec<&str> = params.split(',').collect();
        if p.len() != 2 {
            return false;
        }
        let sensitivity = match p[1].trim().parse::<u8>() {
            Ok(v) => v.clamp(1, 10),
            Err(_) => return false,
        };
        self.clap = matches!(p[0].trim(), "1" | "true");
        self.sensitivity = sensitivity;
        set_sensitivity(sensitivity);
        true
    }
}

// Shared with the capture thread so slider changes apply without restarting it.
static SENSITIVITY: AtomicU8 = AtomicU8::new(SENSITIVITY_DEFAULT);

pub fn set_sensitivity(s: u8) {
    SENSITIVITY.store(s.clamp(1, 10), Ordering::Relaxed);
}

// ── Double-clap detector ─────────────────────────────────────────────

const BLOCK: usize = 441; // 10 ms at 44.1 kHz
const MAX_CLAP_BLOCKS: u32 = 12; // a clap is over within ~120 ms
const MIN_GAP_BLOCKS: u64 = 15; // 150 ms between the two claps…
const MAX_GAP_BLOCKS: u64 = 70; // …and at most 700 ms
const COOLDOWN_BLOCKS: u64 = 100; // 1 s after a toggle

/// Finds two short, sharp sounds a fraction of a second apart, followed by
/// quiet. A steady beat (three or more evenly spaced hits) is rejected, so
/// music with a kick drum doesn't toggle the lights.
pub struct ClapDetector {
    buf: Vec<f32>,
    block: u64,
    background: f32,
    in_transient: bool,
    transient_start: u64,
    transient_blocks: u32,
    last_clap: Option<u64>,
    /// (block of second clap, gap between the claps) awaiting confirmation.
    pending: Option<(u64, u64)>,
    quiet_until: u64,
}

impl ClapDetector {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(BLOCK),
            block: 0,
            background: 0.01,
            in_transient: false,
            transient_start: 0,
            transient_blocks: 0,
            last_clap: None,
            pending: None,
            quiet_until: 0,
        }
    }

    /// Feed samples (−1.0…1.0). Returns true once per detected double clap.
    pub fn push(&mut self, samples: &[f32]) -> bool {
        let mut fired = false;
        for &s in samples {
            self.buf.push(s);
            if self.buf.len() == BLOCK {
                fired |= self.process_block();
                self.buf.clear();
            }
        }
        fired
    }

    fn process_block(&mut self) -> bool {
        self.block += 1;
        let now = self.block;
        let mut sum = 0.0f32;
        let mut peak = 0.0f32;
        for &s in &self.buf {
            sum += s * s;
            peak = peak.max(s.abs());
        }
        let rms = (sum / BLOCK as f32).sqrt();

        // Sensitivity 1…10 → loudness ratio over background 11…2 and a
        // minimum peak 0.275…0.05.
        let sens = SENSITIVITY.load(Ordering::Relaxed) as f32;
        let ratio = 12.0 - sens;
        let min_peak = 0.30 - 0.025 * sens;
        let loud = rms > self.background * ratio && peak > min_peak;

        let mut clap_at = None;
        if self.in_transient {
            if loud {
                self.transient_blocks += 1;
            } else {
                self.in_transient = false;
                if self.transient_blocks <= MAX_CLAP_BLOCKS {
                    clap_at = Some(self.transient_start);
                } else {
                    // A long sound (talking, music swell): forget partial claps.
                    self.last_clap = None;
                    self.pending = None;
                }
            }
        } else if loud {
            self.in_transient = true;
            self.transient_start = now;
            self.transient_blocks = 1;
        } else {
            // Track the room's background level while nothing stands out.
            self.background = (self.background * 0.98 + rms * 0.02).max(0.002);
        }

        if let Some(t) = clap_at {
            if now < self.quiet_until {
                return false;
            }
            if let Some((second, gap)) = self.pending {
                // Another hit right after a pair at a similar spacing: a beat.
                if t - second <= gap + 15 {
                    self.pending = None;
                    self.last_clap = None;
                    self.quiet_until = now + 200;
                    return false;
                }
            }
            match self.last_clap {
                Some(first) if (MIN_GAP_BLOCKS..=MAX_GAP_BLOCKS).contains(&(t - first)) => {
                    self.pending = Some((t, t - first));
                    self.last_clap = None;
                }
                _ => self.last_clap = Some(t),
            }
        }

        // Confirm a pair once the same spacing (plus margin) passed in quiet.
        if let Some((second, gap)) = self.pending {
            if !self.in_transient && now > second + gap + 15 {
                self.pending = None;
                self.quiet_until = now + COOLDOWN_BLOCKS;
                return true;
            }
        }
        false
    }
}

// ── Audio tap ────────────────────────────────────────────────────────

static TAP_CLIENTS: Mutex<Vec<TcpStream>> = Mutex::new(Vec::new());

/// Accept local listeners for the raw capture stream (background thread).
pub fn start_tap(addr: &str) {
    let listener = match TcpListener::bind(addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Audio tap: cannot listen on {}: {}", addr, e);
            return;
        }
    };
    println!("Audio tap listening on {}", addr);
    let addr = addr.to_string();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if stream.set_nonblocking(true).is_ok() {
                println!("Audio tap: client connected on {}", addr);
                TAP_CLIENTS.lock().unwrap().push(stream);
            }
        }
    });
}

/// Send raw PCM bytes to every tap client. A client that can't keep up
/// (or a partial write, which would misalign samples) is disconnected.
pub fn tap_write(bytes: &[u8]) {
    let mut clients = TAP_CLIENTS.lock().unwrap();
    if clients.is_empty() {
        return;
    }
    clients.retain_mut(|c| match c.write(bytes) {
        Ok(n) if n == bytes.len() => true,
        Ok(_) => false,
        Err(e) if e.kind() == ErrorKind::Interrupted => true,
        Err(_) => {
            println!("Audio tap: client disconnected");
            false
        }
    });
}

// ── JSON parsing helpers (module-local, like the other config files) ─

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

    fn silence(ms: usize) -> Vec<f32> {
        // Faint noise so the background estimate stays realistic.
        (0..ms * 44).map(|i| if i % 2 == 0 { 0.003 } else { -0.003 }).collect()
    }

    fn clap() -> Vec<f32> {
        // 30 ms burst that decays fast.
        (0..30 * 44)
            .map(|i| {
                let env = 0.8 * (-(i as f32) / 300.0).exp();
                if i % 2 == 0 { env } else { -env }
            })
            .collect()
    }

    fn run(parts: &[Vec<f32>]) -> usize {
        let mut d = ClapDetector::new();
        parts.iter().map(|p| d.push(p) as usize).sum()
    }

    #[test]
    fn double_clap_fires_once() {
        let n = run(&[silence(1000), clap(), silence(300), clap(), silence(1500)]);
        assert_eq!(n, 1);
    }

    #[test]
    fn single_clap_does_not_fire() {
        assert_eq!(run(&[silence(1000), clap(), silence(2000)]), 0);
    }

    #[test]
    fn steady_beat_does_not_fire() {
        let mut parts = vec![silence(1000)];
        for _ in 0..12 {
            parts.push(clap());
            parts.push(silence(470));
        }
        assert_eq!(run(&parts), 0);
    }

    #[test]
    fn long_noise_does_not_fire() {
        let loud: Vec<f32> = (0..500 * 44).map(|i| if i % 2 == 0 { 0.5 } else { -0.5 }).collect();
        assert_eq!(run(&[silence(1000), loud.clone(), silence(300), loud, silence(1500)]), 0);
    }

    #[test]
    fn config_roundtrip() {
        let c = ListenConfig { clap: true, sensitivity: 7, mic: "hw:1,0".into() };
        let back = ListenConfig::parse_json(&c.to_json());
        assert!(back.clap);
        assert_eq!(back.sensitivity, 7);
        assert_eq!(back.mic, "hw:1,0");
    }
}
