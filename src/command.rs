use crate::hardware::calibration::Calibration;

/// Extended audio analysis sent from the browser.
#[derive(Debug, Clone)]
pub struct AudioAnalysis {
    pub bands: [u8; 8],
    pub kick: u8,
    pub snare: u8,
    pub hihat: u8,
    pub vocals: u8,
    pub bass_line: u8,
    #[allow(dead_code)]
    pub bpm: u16,
    pub beat_phase: u8,    // 0–255 maps to 0.0–1.0
    pub note_midi: u8,     // MIDI note number, 0 = none
    pub chord_root: u8,    // 0–11 = C–B, 255 = none
    #[allow(dead_code)]
    pub chord_quality: u8, // 0=maj 1=min 2=dim 3=aug 4=7 5=maj7 6=m7 7=sus2 8=sus4 9=5 255=?
}

impl Default for AudioAnalysis {
    fn default() -> Self {
        Self {
            bands: [0; 8],
            kick: 0, snare: 0, hihat: 0, vocals: 0, bass_line: 0,
            bpm: 0, beat_phase: 0, note_midi: 0,
            chord_root: 255, chord_quality: 255,
        }
    }
}

/// Commands sent from WebSocket clients to the main loop.
#[derive(Debug)]
pub enum Command {
    SetColor { r: u8, g: u8, b: u8 },
    StartAnimation { name: String },
    StopAnimation,
    AudioData { bands: Vec<u8> },
    ExtendedAudioData(AudioAnalysis),
    SetCalibration(Calibration),
    SaveCalibration,
    GetCalibration,
}

/// State updates sent from the main loop to WebSocket clients.
#[derive(Debug, Clone)]
pub enum StateUpdate {
    /// Full LED state: flat array of [r, g, b, r, g, b, ...]
    LedState(Vec<u8>),
    /// Calibration data as JSON string
    CalibrationData(String),
}
