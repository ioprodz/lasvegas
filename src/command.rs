use crate::hardware::calibration::Calibration;

/// Commands sent from WebSocket clients to the main loop.
#[derive(Debug)]
pub enum Command {
    SetColor { r: u8, g: u8, b: u8 },
    StartAnimation { name: String },
    StopAnimation,
    AudioData { bands: Vec<u8> },
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
