//! Bluetooth device management via `bluetoothctl` subprocess.
//!
//! Provides scanning, pairing, connecting, disconnecting, and listing
//! paired Bluetooth devices — all by shelling out to `bluetoothctl`.

use std::collections::HashMap;
use std::process::Command as ProcessCommand;

#[derive(Debug, Clone)]
pub struct BtDevice {
    pub mac: String,
    pub name: String,
    pub paired: bool,
    pub connected: bool,
}

/// List Bluetooth devices.
///
/// Merges two sources:
/// - `bluetoothctl paired-devices` for persisted paired devices (survives reboot)
/// - `bluetoothctl devices` for recently scanned (unpaired) devices
///
/// Then queries `bluetoothctl info <MAC>` for each to get accurate
/// paired/connected/name status.
pub fn list_devices() -> Vec<BtDevice> {
    let mut seen = HashMap::<String, BtDevice>::new();

    // 1. Paired devices (persisted across reboots)
    if let Ok(output) = ProcessCommand::new("bluetoothctl")
        .args(["devices", "Paired"])
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            if let Some(dev) = parse_device_line(line) {
                seen.insert(dev.mac.clone(), dev);
            }
        }
    }

    // 2. All known/cached devices (includes recently scanned)
    if let Ok(output) = ProcessCommand::new("bluetoothctl")
        .args(["devices"])
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            if let Some(dev) = parse_device_line(line) {
                seen.entry(dev.mac.clone()).or_insert(dev);
            }
        }
    }

    // 3. Query each device for accurate status via `bluetoothctl info`
    let mut devices: Vec<BtDevice> = seen.into_values().collect();
    for dev in &mut devices {
        if let Some((paired, connected, name)) = query_device_info(&dev.mac) {
            dev.paired = paired;
            dev.connected = connected;
            if !name.is_empty() {
                dev.name = name;
            }
        }
    }

    // Sort: connected first, then paired, then new
    devices.sort_by(|a, b| {
        let rank = |d: &BtDevice| {
            if d.connected {
                0
            } else if d.paired {
                1
            } else {
                2
            }
        };
        rank(a).cmp(&rank(b)).then(a.name.cmp(&b.name))
    });

    devices
}

/// Start a Bluetooth scan for `duration_secs` seconds, then return discovered devices.
pub fn scan(duration_secs: u32) -> Vec<BtDevice> {
    let commands = format!(
        "power on\nagent NoInputNoOutput\ndefault-agent\nscan on\n"
    );

    // Run scan in a session — wait for the scan duration + 2s buffer
    let _ = run_bluetoothctl_session(&commands, (duration_secs + 2) as u64);

    // Return all discovered devices
    list_devices()
}

/// Pair with a device by MAC address. Returns Ok(()) or error message.
///
/// Runs agent setup, trust, and pair in a single `bluetoothctl` session
/// so the pairing agent is active when the device responds.
pub fn pair(mac: &str) -> Result<(), String> {
    let commands = format!(
        "power on\nagent NoInputNoOutput\ndefault-agent\ntrust {}\npair {}\n",
        mac, mac
    );

    let output = run_bluetoothctl_session(&commands, 10)?;

    if output.contains("Pairing successful") || output.contains("Already exists") {
        Ok(())
    } else if output.contains("AuthenticationFailed") || output.contains("auth failed") {
        Err("Authentication failed — make sure the device is in pairing mode".to_string())
    } else if output.contains("Failed to pair") {
        // Extract a clean error message
        let msg = output
            .lines()
            .find(|l| l.contains("Failed to pair"))
            .unwrap_or("Unknown error");
        Err(strip_ansi(msg).to_string())
    } else {
        // Might have succeeded — check info
        if let Some((paired, _, _)) = query_device_info(mac) {
            if paired {
                return Ok(());
            }
        }
        Err(format!("Pairing may have failed: {}", strip_ansi(&output)))
    }
}

/// Connect to a paired device.
pub fn connect(mac: &str) -> Result<(), String> {
    let commands = format!(
        "agent NoInputNoOutput\ndefault-agent\nconnect {}\n",
        mac
    );

    let output = run_bluetoothctl_session(&commands, 10)?;

    if output.contains("Connection successful") || output.contains("Already connected") {
        Ok(())
    } else if output.contains("Failed to connect") {
        let msg = output
            .lines()
            .find(|l| l.contains("Failed to connect"))
            .unwrap_or("Connection failed");
        Err(strip_ansi(msg).to_string())
    } else {
        // Check actual state
        if let Some((_, connected, _)) = query_device_info(mac) {
            if connected {
                return Ok(());
            }
        }
        Err(format!("Connect may have failed: {}", strip_ansi(&output)))
    }
}

/// Disconnect a device.
pub fn disconnect(mac: &str) -> Result<(), String> {
    let output = ProcessCommand::new("bluetoothctl")
        .args(["disconnect", mac])
        .output()
        .map_err(|e| format!("Failed to run bluetoothctl disconnect: {}", e))?;

    let text = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);

    if text.contains("Successful") || text.contains("not connected") {
        Ok(())
    } else {
        Err(format!("Disconnect failed: {}", text.trim()))
    }
}

/// Remove (unpair) a device.
pub fn remove(mac: &str) -> Result<(), String> {
    let output = ProcessCommand::new("bluetoothctl")
        .args(["remove", mac])
        .output()
        .map_err(|e| format!("Failed to run bluetoothctl remove: {}", e))?;

    let text = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);

    if text.contains("removed") || text.contains("not available") {
        Ok(())
    } else {
        Err(format!("Remove failed: {}", text.trim()))
    }
}

// ── Internal helpers ─────────────────────────────────────────────────

/// Run a sequence of commands in a single `bluetoothctl` session via stdin.
/// Sends commands, waits a bit for them to execute, then sends quit.
fn run_bluetoothctl_session(commands: &str, wait_secs: u64) -> Result<String, String> {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = ProcessCommand::new("bluetoothctl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start bluetoothctl: {}", e))?;

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(commands.as_bytes());
        // Drop stdin after a delay so bluetoothctl processes commands
        std::thread::sleep(std::time::Duration::from_secs(wait_secs));
        let _ = stdin.write_all(b"quit\n");
    }
    // stdin is dropped here, which closes it

    let output = child
        .wait_with_output()
        .map_err(|e| format!("Failed to read bluetoothctl output: {}", e))?;

    let text = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    Ok(text)
}

/// Strip ANSI escape codes from a string.
fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_escape = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if c == 'm' {
                in_escape = false;
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn parse_device_line(line: &str) -> Option<BtDevice> {
    // "Device AA:BB:CC:DD:EE:FF Some Device Name"
    let line = line.trim();
    let rest = line.strip_prefix("Device ")?;
    let mac = rest.get(..17)?.to_string();
    if mac.len() != 17 || mac.chars().filter(|&c| c == ':').count() != 5 {
        return None;
    }
    let name = rest.get(18..)?.trim().to_string();
    let name = if name.is_empty() {
        mac.clone()
    } else {
        name
    };

    Some(BtDevice {
        mac,
        name,
        paired: false,
        connected: false,
    })
}

/// Query `bluetoothctl info <MAC>` for paired/connected status and name.
///
/// Example output:
/// ```text
/// Device 08:C8:C2:B6:24:F8 (public)
///   Name: JBL TUNE520BT
///   Paired: yes
///   Trusted: yes
///   Connected: no
/// ```
fn query_device_info(mac: &str) -> Option<(bool, bool, String)> {
    let output = ProcessCommand::new("bluetoothctl")
        .args(["info", mac])
        .output()
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout);
    if text.contains("not available") {
        return None;
    }

    let mut paired = false;
    let mut connected = false;
    let mut name = String::new();

    for line in text.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("Paired: ") {
            paired = val.trim() == "yes";
        } else if let Some(val) = line.strip_prefix("Connected: ") {
            connected = val.trim() == "yes";
        } else if let Some(val) = line.strip_prefix("Name: ") {
            name = val.trim().to_string();
        }
    }

    Some((paired, connected, name))
}

/// Serialize a list of BtDevices to JSON.
pub fn devices_to_json(devices: &[BtDevice]) -> String {
    let entries: Vec<String> = devices
        .iter()
        .map(|d| {
            format!(
                r#"{{"mac":"{}","name":"{}","paired":{},"connected":{}}}"#,
                d.mac.replace('"', r#"\""#),
                d.name.replace('"', r#"\""#),
                d.paired,
                d.connected,
            )
        })
        .collect();
    format!("[{}]", entries.join(","))
}
