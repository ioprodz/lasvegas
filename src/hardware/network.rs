//! Network management via NetworkManager (`nmcli`) subprocess.
//!
//! Manages three roles concurrently:
//! - `eth0` console (profile `lv-eth0`, DHCP or static)
//! - Onboard Wi-Fi client (profiles `lv-client-<ssid>`)
//! - TP-Link USB Wi-Fi AP on `wlan_ap` (profile `lv-ap`, NetworkManager
//!   `ipv4.method=shared` for DHCP + NAT).
//!
//! Apply-and-revert protects eth0 and Wi-Fi client changes: a snapshot of
//! the previous NM profile is restored if the UI doesn't confirm within
//! the timeout.

use std::fs;
use std::path::Path;
use std::process::Command as ProcessCommand;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const NETWORK_FILE: &str = "network.json";
const PROFILE_AP: &str = "lv-ap";
const PROFILE_ETH: &str = "lv-eth0";
const PROFILE_CLIENT_PREFIX: &str = "lv-client-";
const IFACE_AP: &str = "wlan_ap";
const IFACE_ETH: &str = "eth0";
const IFACE_WIFI: &str = "wlan0";
const STAGE_TIMEOUT_SECS: u32 = 30;

// ── Data types ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct InterfaceState {
    pub name: String,
    pub kind: String,        // "ethernet" | "wifi" | "loopback" | other
    pub state: String,       // "connected" | "disconnected" | "unavailable" | ...
    pub ssid: String,        // empty unless wifi-client
    pub ip4: String,
    pub signal_pct: u8,
    pub role: String,        // "eth-console" | "wifi-client" | "ap" | "other"
}

#[derive(Debug, Clone)]
pub struct ApClient {
    pub mac: String,
    pub hostname: String,
    pub ip: String,
}

#[derive(Debug, Clone)]
pub struct ApConfig {
    pub ssid: String,
    pub password: String,
    pub band: String,    // "bg" (2.4 GHz) or "a" (5 GHz)
    pub channel: u32,    // 0 = auto
    pub enabled: bool,
}

impl Default for ApConfig {
    fn default() -> Self {
        Self {
            ssid: "lasvegas".to_string(),
            password: random_password(12),
            band: "bg".to_string(),
            channel: 0,
            enabled: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct KnownWifi {
    pub ssid: String,
    pub password: String,
    pub priority: i32,
    pub hidden: bool,
}

#[derive(Debug, Clone)]
pub enum EthMode {
    Dhcp,
    Static { ip: String, prefix: u8, gateway: String, dns: String },
}

#[derive(Debug, Clone)]
pub struct EthConfig {
    pub mode: EthMode,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct ScannedAp {
    pub ssid: String,
    pub signal: u8,
    pub security: String,
    pub in_use: bool,
}

// ── Persistence ───────────────────────────────────────────────────────

/// On-disk shape: { ap: {..}, known_wifis: [..] }
#[derive(Debug, Clone, Default)]
pub struct PersistedState {
    pub ap: Option<ApConfig>,
    pub known_wifis: Vec<KnownWifi>,
}

pub fn load_state() -> PersistedState {
    let path = Path::new(NETWORK_FILE);
    if !path.exists() {
        return PersistedState::default();
    }
    let contents = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return PersistedState::default(),
    };
    parse_state(&contents).unwrap_or_default()
}

pub fn save_state(state: &PersistedState) -> Result<(), String> {
    let ap_json = match &state.ap {
        Some(a) => format!(
            "{{\"ssid\":\"{}\",\"password\":\"{}\",\"band\":\"{}\",\"channel\":{},\"enabled\":{}}}",
            esc(&a.ssid), esc(&a.password), esc(&a.band), a.channel, a.enabled
        ),
        None => "null".to_string(),
    };
    let wifis: Vec<String> = state.known_wifis.iter().map(|w| {
        format!(
            "{{\"ssid\":\"{}\",\"password\":\"{}\",\"priority\":{},\"hidden\":{}}}",
            esc(&w.ssid), esc(&w.password), w.priority, w.hidden
        )
    }).collect();
    let json = format!(
        "{{\"ap\":{},\"known_wifis\":[{}]}}\n",
        ap_json,
        wifis.join(",")
    );
    fs::write(NETWORK_FILE, json).map_err(|e| e.to_string())
}

fn parse_state(s: &str) -> Option<PersistedState> {
    let mut out = PersistedState::default();

    // AP block
    if let Some(idx) = s.find("\"ap\"") {
        let after = &s[idx + 4..];
        if let Some(colon) = after.find(':') {
            let val = after[colon + 1..].trim_start();
            if !val.starts_with("null") {
                if let Some(block) = extract_object(val) {
                    out.ap = Some(ApConfig {
                        ssid: extract_string(block, "ssid").unwrap_or_else(|| "lasvegas".into()),
                        password: extract_string(block, "password").unwrap_or_default(),
                        band: extract_string(block, "band").unwrap_or_else(|| "bg".into()),
                        channel: extract_num(block, "channel").unwrap_or(0.0) as u32,
                        enabled: extract_bool(block, "enabled").unwrap_or(true),
                    });
                }
            }
        }
    }

    // Known wifis array
    if let Some(idx) = s.find("\"known_wifis\"") {
        let after = &s[idx + 13..];
        if let Some(open) = after.find('[') {
            let arr_rest = &after[open..];
            // Find matching close bracket at depth 0
            let mut depth = 0i32;
            let mut end = 0usize;
            for (i, c) in arr_rest.char_indices() {
                match c {
                    '[' => depth += 1,
                    ']' => { depth -= 1; if depth == 0 { end = i; break; } }
                    _ => {}
                }
            }
            if end > 0 {
                let arr_body = &arr_rest[1..end];
                // Each element is {...}; iterate brace-balanced.
                let mut rest = arr_body;
                while let Some(start) = rest.find('{') {
                    let sub = &rest[start..];
                    let mut depth2 = 0i32;
                    let mut e = 0usize;
                    for (i, c) in sub.char_indices() {
                        match c {
                            '{' => depth2 += 1,
                            '}' => { depth2 -= 1; if depth2 == 0 { e = i; break; } }
                            _ => {}
                        }
                    }
                    if e == 0 { break; }
                    let obj = &sub[..=e];
                    out.known_wifis.push(KnownWifi {
                        ssid: extract_string(obj, "ssid").unwrap_or_default(),
                        password: extract_string(obj, "password").unwrap_or_default(),
                        priority: extract_num(obj, "priority").unwrap_or(0.0) as i32,
                        hidden: extract_bool(obj, "hidden").unwrap_or(false),
                    });
                    rest = &sub[e + 1..];
                }
            }
        }
    }

    Some(out)
}

// ── Public API: status ────────────────────────────────────────────────

pub fn list_interfaces() -> Vec<InterfaceState> {
    let mut out = Vec::new();
    let raw = nmcli_t(&["-f", "DEVICE,TYPE,STATE,CONNECTION", "device", "status"]).unwrap_or_default();
    for line in raw.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 4 { continue; }
        let name = parts[0].to_string();
        let kind = parts[1].to_string();
        let state = parts[2].to_string();
        if kind == "loopback" || name.is_empty() { continue; }
        let role = if name == IFACE_AP {
            "ap".into()
        } else if name == IFACE_ETH {
            "eth-console".into()
        } else if name == IFACE_WIFI {
            "wifi-client".into()
        } else if kind == "ethernet" {
            "ethernet".into()
        } else if kind == "wifi" {
            "wifi-client".into()
        } else {
            "other".into()
        };

        let (ip4, ssid, signal) = if state == "connected" {
            let ip = device_ip4(&name);
            let (ssid, sig) = if kind == "wifi" {
                wifi_signal_for(&name)
            } else {
                (String::new(), 0)
            };
            (ip, ssid, sig)
        } else {
            (String::new(), String::new(), 0)
        };

        out.push(InterfaceState {
            name, kind, state, ssid, ip4, signal_pct: signal, role,
        });
    }
    out
}

pub fn ap_clients() -> Vec<ApClient> {
    let mut out = Vec::new();
    // NM "shared" runs an internal dnsmasq with leases at /var/lib/NetworkManager/dnsmasq-wlan_ap.leases
    let lease_path = format!("/var/lib/NetworkManager/dnsmasq-{}.leases", IFACE_AP);
    if let Ok(text) = fs::read_to_string(&lease_path) {
        for line in text.lines() {
            // Format: <expiry> <mac> <ip> <hostname> <client-id>
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                out.push(ApClient {
                    mac: parts[1].to_string(),
                    ip: parts[2].to_string(),
                    hostname: parts[3].to_string(),
                });
            }
        }
    }
    // Fallback / supplement: stations associated with the AP radio
    if out.is_empty() {
        if let Ok(o) = ProcessCommand::new("iw").args(["dev", IFACE_AP, "station", "dump"]).output() {
            let text = String::from_utf8_lossy(&o.stdout);
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("Station ") {
                    let mac = rest.split_whitespace().next().unwrap_or("").to_string();
                    if !mac.is_empty() {
                        out.push(ApClient { mac, hostname: String::new(), ip: String::new() });
                    }
                }
            }
        }
    }
    out
}

// ── Public API: AP ────────────────────────────────────────────────────

pub fn ap_get() -> ApConfig {
    let mut state = load_state();
    if let Some(cfg) = state.ap.take() {
        return cfg;
    }
    let cfg = ApConfig::default();
    let _ = save_state(&PersistedState { ap: Some(cfg.clone()), ..load_state() });
    cfg
}

pub fn ap_set(cfg: ApConfig) -> Result<(), String> {
    let mut state = load_state();
    state.ap = Some(cfg.clone());
    save_state(&state)?;

    // nmcli wants an empty string for "auto channel" — passing "0" errors.
    let channel_str = if cfg.channel == 0 { String::new() } else { cfg.channel.to_string() };

    // Write profile (create or modify).
    if profile_exists(PROFILE_AP) {
        nmcli(&[
            "con", "modify", PROFILE_AP,
            "802-11-wireless.ssid", &cfg.ssid,
            "802-11-wireless.mode", "ap",
            "802-11-wireless.band", &cfg.band,
            "802-11-wireless.channel", &channel_str,
            "ipv4.method", "shared",
            "ipv6.method", "ignore",
            "wifi-sec.key-mgmt", "wpa-psk",
            "wifi-sec.psk", &cfg.password,
            "connection.autoconnect", if cfg.enabled { "yes" } else { "no" },
        ])?;
    } else {
        nmcli(&[
            "con", "add", "type", "wifi",
            "ifname", IFACE_AP,
            "con-name", PROFILE_AP,
            "autoconnect", if cfg.enabled { "yes" } else { "no" },
            "ssid", &cfg.ssid,
            "--",
            "802-11-wireless.mode", "ap",
            "802-11-wireless.band", &cfg.band,
            "802-11-wireless.channel", &channel_str,
            "ipv4.method", "shared",
            "ipv6.method", "ignore",
            "wifi-sec.key-mgmt", "wpa-psk",
            "wifi-sec.psk", &cfg.password,
        ])?;
    }

    if cfg.enabled {
        let _ = ap_apply();
    } else {
        let _ = ap_down();
    }
    Ok(())
}

pub fn ap_apply() -> Result<(), String> {
    // up may fail if interface not present yet (TP-Link unplugged) — treat as ok.
    let _ = nmcli(&["con", "up", PROFILE_AP]);
    Ok(())
}

pub fn ap_down() -> Result<(), String> {
    let _ = nmcli(&["con", "down", PROFILE_AP]);
    Ok(())
}

pub fn ap_toggle(enabled: bool) -> Result<(), String> {
    let mut cfg = ap_get();
    cfg.enabled = enabled;
    ap_set(cfg)
}

// ── Public API: Wi-Fi client ──────────────────────────────────────────

pub fn known_wifis() -> Vec<KnownWifi> {
    load_state().known_wifis
}

pub fn known_wifi_upsert(w: KnownWifi) -> Result<(), String> {
    let mut state = load_state();
    if let Some(existing) = state.known_wifis.iter_mut().find(|k| k.ssid == w.ssid) {
        *existing = w.clone();
    } else {
        state.known_wifis.push(w.clone());
    }
    save_state(&state)?;
    apply_wifi_profile(&w)
}

pub fn known_wifi_remove(ssid: &str) -> Result<(), String> {
    let mut state = load_state();
    state.known_wifis.retain(|k| k.ssid != ssid);
    save_state(&state)?;
    let profile = client_profile_name(ssid);
    let _ = nmcli(&["con", "delete", &profile]);
    Ok(())
}

pub fn wifi_scan() -> Vec<ScannedAp> {
    let _ = nmcli(&["dev", "wifi", "rescan", "ifname", IFACE_WIFI]);
    let raw = nmcli_t(&["-f", "IN-USE,SSID,SIGNAL,SECURITY", "dev", "wifi", "list", "ifname", IFACE_WIFI]).unwrap_or_default();
    let mut out = Vec::new();
    for line in raw.lines() {
        let parts: Vec<&str> = line.splitn(4, ':').collect();
        if parts.len() < 4 { continue; }
        let in_use = parts[0].trim() == "*";
        let ssid = unescape_t(parts[1]).trim().to_string();
        if ssid.is_empty() || ssid == "--" { continue; }
        let signal: u8 = parts[2].trim().parse().unwrap_or(0);
        let security = parts[3].trim().to_string();
        out.push(ScannedAp { ssid, signal, security, in_use });
    }
    // Dedupe by SSID, keep strongest
    out.sort_by(|a, b| b.signal.cmp(&a.signal));
    let mut seen = std::collections::HashSet::new();
    out.retain(|a| seen.insert(a.ssid.clone()));
    out
}

pub fn wifi_connect_now(ssid: &str) -> Result<(), String> {
    let profile = client_profile_name(ssid);
    if profile_exists(&profile) {
        nmcli(&["con", "up", &profile])?;
    } else {
        return Err(format!("No saved profile for SSID '{}'", ssid));
    }
    Ok(())
}

fn apply_wifi_profile(w: &KnownWifi) -> Result<(), String> {
    let profile = client_profile_name(&w.ssid);
    let prio = w.priority.to_string();
    let hidden = if w.hidden { "yes" } else { "no" };
    if profile_exists(&profile) {
        nmcli(&[
            "con", "modify", &profile,
            "802-11-wireless.ssid", &w.ssid,
            "802-11-wireless.hidden", hidden,
            "wifi-sec.key-mgmt", if w.password.is_empty() { "none" } else { "wpa-psk" },
            "wifi-sec.psk", &w.password,
            "connection.autoconnect", "yes",
            "connection.autoconnect-priority", &prio,
            "ipv4.method", "auto",
        ])?;
    } else {
        let mut args: Vec<&str> = vec![
            "con", "add", "type", "wifi",
            "ifname", IFACE_WIFI,
            "con-name", &profile,
            "ssid", &w.ssid,
            "autoconnect", "yes",
            "--",
            "connection.autoconnect-priority", &prio,
            "802-11-wireless.hidden", hidden,
            "ipv4.method", "auto",
        ];
        if !w.password.is_empty() {
            args.extend(["wifi-sec.key-mgmt", "wpa-psk", "wifi-sec.psk", &w.password]);
        }
        nmcli(&args)?;
    }
    Ok(())
}

fn client_profile_name(ssid: &str) -> String {
    let sanitized: String = ssid.chars().map(|c| {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }
    }).collect();
    format!("{}{}", PROFILE_CLIENT_PREFIX, sanitized)
}

// ── Public API: Ethernet ──────────────────────────────────────────────

pub fn eth_get() -> EthConfig {
    let raw = nmcli_t(&["-f", "ipv4.method,ipv4.addresses,ipv4.gateway,ipv4.dns,connection.autoconnect", "con", "show", PROFILE_ETH])
        .or_else(|_| nmcli_t(&["-f", "ipv4.method,ipv4.addresses,ipv4.gateway,ipv4.dns,connection.autoconnect", "con", "show", "Wired connection 1"]))
        .unwrap_or_default();
    let mut method = "auto".to_string();
    let mut addresses = String::new();
    let mut gateway = String::new();
    let mut dns = String::new();
    let mut autoconnect = true;
    for line in raw.lines() {
        if let Some(v) = line.strip_prefix("ipv4.method:") { method = v.trim().to_string(); }
        else if let Some(v) = line.strip_prefix("ipv4.addresses:") { addresses = v.trim().to_string(); }
        else if let Some(v) = line.strip_prefix("ipv4.gateway:") { gateway = v.trim().to_string(); }
        else if let Some(v) = line.strip_prefix("ipv4.dns:") { dns = v.trim().to_string(); }
        else if let Some(v) = line.strip_prefix("connection.autoconnect:") { autoconnect = v.trim() == "yes"; }
    }
    let mode = if method == "manual" {
        let (ip, prefix) = if let Some(slash) = addresses.find('/') {
            let ip = addresses[..slash].to_string();
            let pfx = addresses[slash + 1..].split(',').next().unwrap_or("24").parse().unwrap_or(24u8);
            (ip, pfx)
        } else {
            (addresses.clone(), 24)
        };
        EthMode::Static { ip, prefix, gateway, dns }
    } else {
        EthMode::Dhcp
    };
    EthConfig { mode, enabled: autoconnect }
}

pub fn eth_set(cfg: EthConfig) -> Result<(), String> {
    let profile = PROFILE_ETH.to_string();
    if !profile_exists(&profile) {
        // Adopt an existing eth profile (e.g. "Wired connection 1") by renaming it,
        // or create a fresh one if NetworkManager hasn't auto-created one yet.
        if let Some(existing) = find_eth_profile() {
            let _ = nmcli(&["con", "modify", &existing, "connection.id", &profile]);
        } else {
            nmcli(&[
                "con", "add", "type", "ethernet",
                "ifname", IFACE_ETH,
                "con-name", &profile,
                "autoconnect", "yes",
            ])?;
        }
    }

    match &cfg.mode {
        EthMode::Dhcp => {
            nmcli(&[
                "con", "modify", &profile,
                "ipv4.method", "auto",
                "ipv4.addresses", "",
                "ipv4.gateway", "",
                "ipv4.dns", "",
                "connection.autoconnect", if cfg.enabled { "yes" } else { "no" },
            ])?;
        }
        EthMode::Static { ip, prefix, gateway, dns } => {
            let addrs = format!("{}/{}", ip, prefix);
            nmcli(&[
                "con", "modify", &profile,
                "ipv4.method", "manual",
                "ipv4.addresses", &addrs,
                "ipv4.gateway", gateway,
                "ipv4.dns", dns,
                "connection.autoconnect", if cfg.enabled { "yes" } else { "no" },
            ])?;
        }
    }
    let _ = nmcli(&["con", "up", &profile]);
    Ok(())
}

fn find_eth_profile() -> Option<String> {
    let raw = nmcli_t(&["-f", "NAME,TYPE,DEVICE", "con", "show"]).ok()?;
    for line in raw.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() >= 3 && parts[1] == "802-3-ethernet" {
            return Some(parts[0].to_string());
        }
    }
    None
}

// ── Apply-and-revert ──────────────────────────────────────────────────

/// One pending change. Holds a serialized profile snapshot we can restore.
#[derive(Debug)]
struct PendingChange {
    token: u32,
    profile: String,
    snapshot: Vec<(String, String)>,   // (key, value) from `nmcli con show --show-secrets`
    cancelled: Arc<AtomicBool>,
}

static PENDING: Mutex<Option<PendingChange>> = Mutex::new(None);
static TOKEN_COUNTER: AtomicU32 = AtomicU32::new(1);

/// Snapshot a profile's full settings via `nmcli con show --show-secrets`.
/// Returns `Vec<(key, value)>` we can replay through `nmcli con modify`.
fn snapshot_profile(profile: &str) -> Result<Vec<(String, String)>, String> {
    let raw = nmcli_t(&["--show-secrets", "con", "show", profile])?;
    let mut out = Vec::new();
    for line in raw.lines() {
        if let Some(colon) = line.find(':') {
            let key = line[..colon].trim().to_string();
            let val = line[colon + 1..].trim().to_string();
            // Skip read-only / runtime keys.
            if key.starts_with("GENERAL.") || key.starts_with("IP4.") || key.starts_with("IP6.")
                || key.starts_with("DHCP4.") || key.starts_with("DHCP6.") {
                continue;
            }
            // Skip flag-only keys (often appear as "(default)").
            if val.contains("(default)") { continue; }
            out.push((key, val));
        }
    }
    Ok(out)
}

fn restore_snapshot(profile: &str, snapshot: &[(String, String)]) {
    for (key, val) in snapshot {
        let v = if val == "--" || val == "(null)" { "" } else { val.as_str() };
        let _ = nmcli(&["con", "modify", profile, key, v]);
    }
    let _ = nmcli(&["con", "up", profile]);
}

/// Stage a change against `profile` with auto-revert after `STAGE_TIMEOUT_SECS`.
/// `apply` mutates the profile; we snapshot first so we can roll back.
/// Returns the token; emit `stage:pending:<token>:<secs>` to the UI.
pub fn stage_change<F>(profile: &str, apply: F) -> Result<u32, String>
where
    F: FnOnce() -> Result<(), String> + Send + 'static,
{
    // Cancel any prior pending change.
    {
        let mut slot = PENDING.lock().unwrap();
        if let Some(p) = slot.take() {
            p.cancelled.store(true, Ordering::Relaxed);
        }
    }

    let snapshot = snapshot_profile(profile)?;
    apply()?;

    let token = TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed);
    let cancelled = Arc::new(AtomicBool::new(false));
    let pending = PendingChange {
        token,
        profile: profile.to_string(),
        snapshot,
        cancelled: cancelled.clone(),
    };
    *PENDING.lock().unwrap() = Some(pending);

    let profile_owned = profile.to_string();
    thread::spawn(move || {
        // Polled sleep so cancellation is responsive.
        for _ in 0..STAGE_TIMEOUT_SECS {
            if cancelled.load(Ordering::Relaxed) { return; }
            thread::sleep(Duration::from_secs(1));
        }
        // Timed out — roll back if still the current pending.
        let snap = {
            let mut slot = PENDING.lock().unwrap();
            match slot.take() {
                Some(p) if p.token == token => Some(p.snapshot),
                Some(other) => { *slot = Some(other); None }
                None => None,
            }
        };
        if let Some(snap) = snap {
            eprintln!("Network: auto-reverting stage token {}", token);
            restore_snapshot(&profile_owned, &snap);
        }
    });

    Ok(token)
}

pub fn confirm_stage(token: u32) -> bool {
    let mut slot = PENDING.lock().unwrap();
    if let Some(p) = slot.as_ref() {
        if p.token == token {
            let p = slot.take().unwrap();
            p.cancelled.store(true, Ordering::Relaxed);
            return true;
        }
    }
    false
}

pub fn revert_stage(token: u32) -> bool {
    let snap_and_profile = {
        let mut slot = PENDING.lock().unwrap();
        if let Some(p) = slot.as_ref() {
            if p.token == token {
                let p = slot.take().unwrap();
                p.cancelled.store(true, Ordering::Relaxed);
                Some((p.profile, p.snapshot))
            } else { None }
        } else { None }
    };
    if let Some((profile, snap)) = snap_and_profile {
        restore_snapshot(&profile, &snap);
        true
    } else {
        false
    }
}

pub fn pending_stage_info() -> Option<(u32, String)> {
    let slot = PENDING.lock().unwrap();
    slot.as_ref().map(|p| (p.token, p.profile.clone()))
}

// ── JSON snapshot for the UI ──────────────────────────────────────────

pub fn status_json() -> String {
    let interfaces = list_interfaces();
    let ap = ap_get();
    let clients = ap_clients();
    let wifis = known_wifis();
    let eth = eth_get();
    let pending = pending_stage_info();

    let iface_json: Vec<String> = interfaces.iter().map(|i| format!(
        "{{\"name\":\"{}\",\"kind\":\"{}\",\"state\":\"{}\",\"ssid\":\"{}\",\"ip4\":\"{}\",\"signal\":{},\"role\":\"{}\"}}",
        esc(&i.name), esc(&i.kind), esc(&i.state), esc(&i.ssid), esc(&i.ip4), i.signal_pct, esc(&i.role)
    )).collect();

    let client_json: Vec<String> = clients.iter().map(|c| format!(
        "{{\"mac\":\"{}\",\"hostname\":\"{}\",\"ip\":\"{}\"}}",
        esc(&c.mac), esc(&c.hostname), esc(&c.ip)
    )).collect();

    let wifi_json: Vec<String> = wifis.iter().map(|w| format!(
        "{{\"ssid\":\"{}\",\"has_password\":{},\"priority\":{},\"hidden\":{}}}",
        esc(&w.ssid), !w.password.is_empty(), w.priority, w.hidden
    )).collect();

    let eth_json = match &eth.mode {
        EthMode::Dhcp => format!("{{\"mode\":\"dhcp\",\"enabled\":{}}}", eth.enabled),
        EthMode::Static { ip, prefix, gateway, dns } => format!(
            "{{\"mode\":\"static\",\"ip\":\"{}\",\"prefix\":{},\"gateway\":\"{}\",\"dns\":\"{}\",\"enabled\":{}}}",
            esc(ip), prefix, esc(gateway), esc(dns), eth.enabled
        ),
    };

    let ap_iface_present = interfaces.iter().any(|i| i.name == IFACE_AP);

    let ap_json = format!(
        "{{\"ssid\":\"{}\",\"password\":\"{}\",\"band\":\"{}\",\"channel\":{},\"enabled\":{},\"iface_present\":{}}}",
        esc(&ap.ssid), esc(&ap.password), esc(&ap.band), ap.channel, ap.enabled, ap_iface_present
    );

    let pending_json = match pending {
        Some((t, p)) => format!("{{\"token\":{},\"profile\":\"{}\",\"timeout_secs\":{}}}", t, esc(&p), STAGE_TIMEOUT_SECS),
        None => "null".to_string(),
    };

    format!(
        "{{\"interfaces\":[{}],\"ap\":{},\"ap_clients\":[{}],\"known_wifis\":[{}],\"eth\":{},\"pending\":{}}}",
        iface_json.join(","),
        ap_json,
        client_json.join(","),
        wifi_json.join(","),
        eth_json,
        pending_json
    )
}

// ── nmcli wrappers ────────────────────────────────────────────────────

fn nmcli(args: &[&str]) -> Result<String, String> {
    let out = ProcessCommand::new("nmcli")
        .args(args)
        .output()
        .map_err(|e| format!("nmcli not found: {}", e))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("nmcli {:?} failed: {}", args, stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// nmcli with `-t` (terse, colon-separated) output.
fn nmcli_t(args: &[&str]) -> Result<String, String> {
    let mut all: Vec<&str> = vec!["-t"];
    all.extend(args);
    nmcli(&all)
}

fn profile_exists(name: &str) -> bool {
    nmcli_t(&["-f", "NAME", "con", "show"])
        .map(|s| s.lines().any(|l| l.trim() == name))
        .unwrap_or(false)
}

fn device_ip4(device: &str) -> String {
    let raw = match nmcli_t(&["-f", "IP4.ADDRESS", "device", "show", device]) {
        Ok(r) => r,
        Err(_) => return String::new(),
    };
    for line in raw.lines() {
        if let Some(v) = line.strip_prefix("IP4.ADDRESS[1]:") {
            return v.trim().split('/').next().unwrap_or("").to_string();
        }
    }
    String::new()
}

fn wifi_signal_for(device: &str) -> (String, u8) {
    let raw = nmcli_t(&["-f", "ACTIVE,SSID,SIGNAL", "dev", "wifi", "list", "ifname", device]).unwrap_or_default();
    for line in raw.lines() {
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        if parts.len() < 3 { continue; }
        if parts[0].trim() == "yes" {
            return (unescape_t(parts[1]).trim().to_string(), parts[2].trim().parse().unwrap_or(0));
        }
    }
    (String::new(), 0)
}

// ── Tiny utilities ────────────────────────────────────────────────────

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn unescape_t(s: &str) -> String {
    // nmcli -t escapes `:` as `\:` and `\` as `\\`.
    let mut out = String::with_capacity(s.len());
    let mut iter = s.chars();
    while let Some(c) = iter.next() {
        if c == '\\' {
            if let Some(n) = iter.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn extract_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let sub = &s[start..];
    let mut depth = 0i32;
    let mut end = 0usize;
    for (i, c) in sub.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => { depth -= 1; if depth == 0 { end = i; break; } }
            _ => {}
        }
    }
    if end == 0 { return None; }
    Some(&sub[..=end])
}

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
    let end = after.find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
        .unwrap_or(after.len());
    after[..end].parse().ok()
}

fn extract_bool(s: &str, key: &str) -> Option<bool> {
    let pat = format!("\"{}\"", key);
    let idx = s.find(&pat)?;
    let rest = &s[idx + pat.len()..];
    let colon = rest.find(':')?;
    let after = rest[colon + 1..].trim_start();
    if after.starts_with("true") { Some(true) }
    else if after.starts_with("false") { Some(false) }
    else { None }
}

fn random_password(len: usize) -> String {
    // Cheap pseudo-randomness from system time; only used to seed first-boot AP.
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut seed = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
    let mut out = String::with_capacity(len);
    for _ in 0..len {
        // xorshift
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        out.push(ALPHABET[(seed as usize) % ALPHABET.len()] as char);
    }
    out
}

// ── Parsing helpers for WebSocket commands ────────────────────────────

/// Parse `KnownWifi` from a JSON object string.
pub fn known_wifi_from_json(s: &str) -> Option<KnownWifi> {
    Some(KnownWifi {
        ssid: extract_string(s, "ssid")?,
        password: extract_string(s, "password").unwrap_or_default(),
        priority: extract_num(s, "priority").unwrap_or(0.0) as i32,
        hidden: extract_bool(s, "hidden").unwrap_or(false),
    })
}

/// Parse `ApConfig` from a JSON object string.
pub fn ap_config_from_json(s: &str) -> Option<ApConfig> {
    Some(ApConfig {
        ssid: extract_string(s, "ssid")?,
        password: extract_string(s, "password").unwrap_or_default(),
        band: extract_string(s, "band").unwrap_or_else(|| "bg".into()),
        channel: extract_num(s, "channel").unwrap_or(0.0) as u32,
        enabled: extract_bool(s, "enabled").unwrap_or(true),
    })
}

/// Parse `EthConfig` from a JSON object string.
pub fn eth_config_from_json(s: &str) -> Option<EthConfig> {
    let mode_str = extract_string(s, "mode").unwrap_or_else(|| "dhcp".into());
    let enabled = extract_bool(s, "enabled").unwrap_or(true);
    let mode = if mode_str == "static" {
        EthMode::Static {
            ip: extract_string(s, "ip")?,
            prefix: extract_num(s, "prefix").unwrap_or(24.0) as u8,
            gateway: extract_string(s, "gateway").unwrap_or_default(),
            dns: extract_string(s, "dns").unwrap_or_default(),
        }
    } else {
        EthMode::Dhcp
    };
    Some(EthConfig { mode, enabled })
}
