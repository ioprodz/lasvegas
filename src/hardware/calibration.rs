use std::fs;
use std::path::Path;

const CALIBRATION_FILE: &str = "calibration.json";

#[derive(Debug, Clone)]
pub struct ChannelCalibration {
    pub min: u8,
    pub max: u8,
    pub gamma: f32,
}

impl Default for ChannelCalibration {
    fn default() -> Self {
        Self {
            min: 0,
            max: 255,
            gamma: 1.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Calibration {
    pub r: ChannelCalibration,
    pub g: ChannelCalibration,
    pub b: ChannelCalibration,
}

impl Default for Calibration {
    fn default() -> Self {
        Self {
            r: ChannelCalibration::default(),
            g: ChannelCalibration::default(),
            b: ChannelCalibration::default(),
        }
    }
}

impl Calibration {
    /// Apply calibration to a single color value for one channel.
    /// Maps input 0-255 through gamma curve, then scales to min..max range.
    fn calibrate_channel(value: u8, ch: &ChannelCalibration) -> u8 {
        if value == 0 {
            return 0;
        }
        let normalized = value as f32 / 255.0;
        let gamma_corrected = normalized.powf(ch.gamma);
        let range = ch.max as f32 - ch.min as f32;
        let output = ch.min as f32 + gamma_corrected * range;
        output.round().clamp(0.0, 255.0) as u8
    }

    /// Apply calibration to an RGB color.
    pub fn apply(&self, color: [u8; 4]) -> [u8; 4] {
        [
            Self::calibrate_channel(color[0], &self.r),
            Self::calibrate_channel(color[1], &self.g),
            Self::calibrate_channel(color[2], &self.b),
            color[3],
        ]
    }

    /// Load calibration from file, or return default if file doesn't exist.
    pub fn load() -> Self {
        let path = Path::new(CALIBRATION_FILE);
        if !path.exists() {
            return Self::default();
        }
        match fs::read_to_string(path) {
            Ok(contents) => Self::parse_json(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Save calibration to file.
    pub fn save(&self) -> Result<(), String> {
        let json = format!(
            "{{\n  \"r\": {{ \"min\": {}, \"max\": {}, \"gamma\": {:.2} }},\n  \"g\": {{ \"min\": {}, \"max\": {}, \"gamma\": {:.2} }},\n  \"b\": {{ \"min\": {}, \"max\": {}, \"gamma\": {:.2} }}\n}}",
            self.r.min, self.r.max, self.r.gamma,
            self.g.min, self.g.max, self.g.gamma,
            self.b.min, self.b.max, self.b.gamma,
        );
        fs::write(CALIBRATION_FILE, json).map_err(|e| e.to_string())
    }

    /// Serialize to JSON string for sending to clients.
    pub fn to_json(&self) -> String {
        format!(
            "{{\"r\":{{\"min\":{},\"max\":{},\"gamma\":{:.2}}},\"g\":{{\"min\":{},\"max\":{},\"gamma\":{:.2}}},\"b\":{{\"min\":{},\"max\":{},\"gamma\":{:.2}}}}}",
            self.r.min, self.r.max, self.r.gamma,
            self.g.min, self.g.max, self.g.gamma,
            self.b.min, self.b.max, self.b.gamma,
        )
    }

    /// Parse calibration from a simple JSON string (no external JSON crate needed).
    fn parse_json(s: &str) -> Option<Self> {
        fn extract_channel(s: &str, key: &str) -> Option<ChannelCalibration> {
            let key_pat = format!("\"{}\"", key);
            let start = s.find(&key_pat)?;
            let rest = &s[start..];
            let brace_start = rest.find('{')?;
            let brace_end = rest[brace_start..].find('}')? + brace_start;
            let block = &rest[brace_start..=brace_end];

            let min = extract_num(block, "min")? as u8;
            let max = extract_num(block, "max")? as u8;
            let gamma = extract_num(block, "gamma")? as f32;

            Some(ChannelCalibration { min, max, gamma })
        }

        fn extract_num(s: &str, key: &str) -> Option<f64> {
            let key_pat = format!("\"{}\"", key);
            let start = s.find(&key_pat)?;
            let rest = &s[start + key_pat.len()..];
            let colon = rest.find(':')?;
            let after_colon = rest[colon + 1..].trim_start();
            let end = after_colon.find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
                .unwrap_or(after_colon.len());
            after_colon[..end].parse().ok()
        }

        Some(Self {
            r: extract_channel(s, "r")?,
            g: extract_channel(s, "g")?,
            b: extract_channel(s, "b")?,
        })
    }

    /// Parse calibration update from WebSocket text command.
    /// Format: "calibrate:r_min,r_max,r_gamma,g_min,g_max,g_gamma,b_min,b_max,b_gamma"
    pub fn from_command(params: &str) -> Option<Self> {
        let parts: Vec<&str> = params.split(',').collect();
        if parts.len() != 9 {
            return None;
        }
        let vals: Vec<f32> = parts.iter().filter_map(|p| p.trim().parse().ok()).collect();
        if vals.len() != 9 {
            return None;
        }
        Some(Self {
            r: ChannelCalibration { min: vals[0] as u8, max: vals[1] as u8, gamma: vals[2] },
            g: ChannelCalibration { min: vals[3] as u8, max: vals[4] as u8, gamma: vals[5] },
            b: ChannelCalibration { min: vals[6] as u8, max: vals[7] as u8, gamma: vals[8] },
        })
    }
}
