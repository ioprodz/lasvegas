//! Hardware audio capture via subprocess.
//!
//! Uses `pactl list sources short` to enumerate all audio inputs (ALSA,
//! Bluetooth, USB) and `parec` to stream raw PCM.  Falls back to
//! `arecord -l` / `arecord` if PulseAudio/PipeWire is unavailable.
//!
//! When running as root (via sudo), commands that need the user's
//! PipeWire session (`pactl`, `parec`) are executed via
//! `sudo -u <user> --preserve-env=XDG_RUNTIME_DIR`.

use crate::command::Command;
use crate::hardware::audio_analysis::AudioPipeline;
use std::io::Read;
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

/// Represents an available audio input device.
#[derive(Debug, Clone)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
}

// ── User-session helper ──────────────────────────────────────────────

/// Detect the non-root login user (the one with a PipeWire session).
/// Returns (username, uid) or None if already running as that user.
fn get_session_user() -> Option<(String, u32)> {
    // If we're not root, no wrapping needed
    if !nix_is_root() {
        return None;
    }

    // Check SUDO_USER first (set when running `sudo ./lasvegas`)
    if let Ok(user) = std::env::var("SUDO_USER") {
        if !user.is_empty() && user != "root" {
            if let Ok(uid) = std::env::var("SUDO_UID") {
                if let Ok(uid) = uid.parse::<u32>() {
                    return Some((user, uid));
                }
            }
        }
    }

    // Fallback: find the first user with a PipeWire socket in /run/user/
    if let Ok(entries) = std::fs::read_dir("/run/user") {
        for entry in entries.flatten() {
            let uid_str = entry.file_name().to_string_lossy().to_string();
            if let Ok(uid) = uid_str.parse::<u32>() {
                if uid >= 1000 {
                    let pw_sock = entry.path().join("pipewire-0");
                    if pw_sock.exists() {
                        // Resolve username from uid
                        let output = ProcessCommand::new("id")
                            .args(["-nu", &uid.to_string()])
                            .output();
                        if let Ok(o) = output {
                            let name = String::from_utf8_lossy(&o.stdout).trim().to_string();
                            if !name.is_empty() {
                                return Some((name, uid));
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

fn nix_is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

/// Build a `Command` that runs `program` as the session user if we're root.
/// Sets XDG_RUNTIME_DIR so PipeWire/PulseAudio tools find the user socket.
fn user_command(program: &str) -> ProcessCommand {
    if let Some((user, uid)) = get_session_user() {
        let mut cmd = ProcessCommand::new("sudo");
        cmd.args([
            "-u",
            &user,
            &format!("XDG_RUNTIME_DIR=/run/user/{}", uid),
            program,
        ]);
        cmd
    } else {
        ProcessCommand::new(program)
    }
}

// ── Public API ───────────────────────────────────────────────────────

/// Enumerate all capture (input) sources.
pub fn list_input_devices() -> Vec<AudioDevice> {
    let mut devices = list_pactl_sources();
    if !devices.is_empty() {
        return devices;
    }

    devices = list_arecord_devices();
    if !devices.is_empty() {
        return devices;
    }

    vec![AudioDevice {
        id: "default".to_string(),
        name: "Default".to_string(),
    }]
}

// ── pactl enumeration ────────────────────────────────────────────────

fn list_pactl_sources() -> Vec<AudioDevice> {
    let output = match user_command("pactl")
        .args(["list", "sources", "short"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let mut devices = Vec::new();

    for line in text.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 2 {
            continue;
        }
        let source_name = parts[1];

        // Skip monitor sources (they capture speaker output, not mic input)
        if source_name.contains(".monitor") {
            continue;
        }

        let friendly = make_friendly_name(source_name);
        devices.push(AudioDevice {
            id: source_name.to_string(),
            name: friendly,
        });
    }

    devices
}

fn make_friendly_name(source: &str) -> String {
    if source.starts_with("bluez_source.") || source.starts_with("bluez_input.") {
        let rest = source
            .strip_prefix("bluez_source.")
            .or_else(|| source.strip_prefix("bluez_input."))
            .unwrap_or(source);
        let mac_part: String = rest.chars().take(17).collect();
        let mac = mac_part.replace('_', ":");
        format!("Bluetooth {}", mac)
    } else if source.starts_with("alsa_input.") {
        let rest = source.strip_prefix("alsa_input.").unwrap_or(source);
        if let Some(usb_part) = rest.strip_prefix("usb-") {
            let name = usb_part
                .split('-')
                .take_while(|s| s.parse::<u32>().is_err())
                .collect::<Vec<&str>>()
                .join(" ")
                .replace('_', " ");
            if !name.is_empty() {
                return name;
            }
        }
        rest.replace('_', " ").replace('.', " ")
    } else {
        source.replace('_', " ").replace('.', " ")
    }
}

// ── arecord fallback ─────────────────────────────────────────────────

fn list_arecord_devices() -> Vec<AudioDevice> {
    let output = match ProcessCommand::new("arecord").arg("-l").output() {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let mut devices = Vec::new();

    for line in text.lines() {
        if line.starts_with("card ") {
            if let Some(dev) = parse_arecord_device_line(line) {
                devices.push(dev);
            }
        }
    }

    devices
}

fn parse_arecord_device_line(line: &str) -> Option<AudioDevice> {
    let card_num = line
        .strip_prefix("card ")?
        .split(':')
        .next()?
        .trim()
        .parse::<u32>()
        .ok()?;

    let device_part = line.find(", device ")?;
    let after_device = &line[device_part + ", device ".len()..];
    let device_num = after_device
        .split(':')
        .next()?
        .trim()
        .parse::<u32>()
        .ok()?;

    let name = if let Some(start) = line.find('[') {
        if let Some(end) = line[start..].find(']') {
            line[start + 1..start + end].to_string()
        } else {
            format!("Card {}", card_num)
        }
    } else {
        format!("Card {}", card_num)
    };

    Some(AudioDevice {
        id: format!("hw:{},{}", card_num, device_num),
        name,
    })
}

// ── Capture ──────────────────────────────────────────────────────────

/// Spawn a capture thread that streams audio from the given device.
/// Uses `parec` for PipeWire sources (run as session user), `arecord` for ALSA hw.
pub fn start_capture(
    device_id: &str,
    cmd_tx: Sender<Command>,
    stop: Arc<AtomicBool>,
) -> Result<JoinHandle<()>, String> {
    let device_id = device_id.to_string();
    let use_pw = !device_id.starts_with("hw:") && !device_id.starts_with("default");

    let handle = thread::spawn(move || {
        let sample_rate = 44100u32;

        let mut child = if use_pw {
            // PipeWire source — use pw-cat --record --raw for raw S16 PCM to stdout
            match user_command("pw-cat")
                .args([
                    "--record",
                    "--raw",
                    &format!("--target={}", device_id),
                    "--format=s16",
                    "--rate=44100",
                    "--channels=1",
                    "-",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Failed to start pw-cat: {}", e);
                    return;
                }
            }
        } else {
            // ALSA hw device — use arecord
            match ProcessCommand::new("arecord")
                .args([
                    "-D",
                    &device_id,
                    "-f",
                    "S16_LE",
                    "-r",
                    &sample_rate.to_string(),
                    "-c",
                    "1",
                    "-t",
                    "raw",
                    "-",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Failed to start arecord: {}", e);
                    return;
                }
            }
        };

        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                eprintln!("Failed to get capture stdout");
                return;
            }
        };

        println!(
            "Hardware audio capture started on '{}' @ {}Hz (via {})",
            device_id,
            sample_rate,
            if use_pw { "pw-cat" } else { "arecord" }
        );

        let mut pipeline = AudioPipeline::new(sample_rate as f32);
        let chunk_frames = 1024usize;
        let mut byte_buf = vec![0u8; chunk_frames * 2];
        let mut reader = std::io::BufReader::new(stdout);

        while !stop.load(Ordering::Relaxed) {
            match reader.read(&mut byte_buf) {
                Ok(0) => {
                    eprintln!("Audio capture stream ended");
                    break;
                }
                Ok(n) => {
                    let sample_count = n / 2;
                    let samples: Vec<f32> = (0..sample_count)
                        .map(|i| {
                            let lo = byte_buf[i * 2] as i16;
                            let hi = (byte_buf[i * 2 + 1] as i16) << 8;
                            (lo | hi) as f32 / 32768.0
                        })
                        .collect();

                    if let Some(analysis) = pipeline.push_samples(&samples) {
                        if cmd_tx.send(Command::ExtendedAudioData(analysis)).is_err() {
                            break;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Audio capture read error: {}", e);
                    break;
                }
            }
        }

        let _ = child.kill();
        let _ = child.wait();
        println!("Hardware audio capture stopped");
    });

    Ok(handle)
}
