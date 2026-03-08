use crate::hardware::led::{hsv_to_rgb, LedStrip};
use std::cell::RefCell;

const COLS: usize = 60;
const ROWS: usize = 6;
const HISTORY_LEN: usize = COLS;

thread_local! {
    static HISTORY: RefCell<[[u8; 8]; HISTORY_LEN]> = RefCell::new([[0u8; 8]; HISTORY_LEN]);
}

/// Scrolling spectrogram — each column is a time slice, rows map to
/// frequency bands. New audio data enters on the right and scrolls left.
/// Creates a waterfall / spectrogram effect across the 60x6 grid.
pub fn audio_waterfall(strip: &mut LedStrip, bands: &[u8; 8]) {
    HISTORY.with(|hist| {
        let h = &mut *hist.borrow_mut();

        // Shift history left, push new bands on the right
        for i in 0..(HISTORY_LEN - 1) {
            h[i] = h[i + 1];
        }
        h[HISTORY_LEN - 1] = *bands;

        // Map 8 bands to 6 rows
        let band_to_row: [&[usize]; 6] = [
            &[0, 1], // sub + bass
            &[2],    // low
            &[3],    // mid
            &[4],    // upper mid
            &[5, 6], // presence + brilliance
            &[7],    // treble
        ];

        let row_hues: [f32; 6] = [0.0, 30.0, 60.0, 150.0, 220.0, 280.0];

        for col in 0..COLS {
            let snapshot = &h[col];
            for row in 0..ROWS {
                let indices = band_to_row[row];
                let avg: u16 = indices.iter().map(|&i| snapshot[i] as u16).sum::<u16>()
                    / indices.len() as u16;
                let val = avg as f32 / 255.0;

                let sat = 0.6 + val * 0.4;
                let color = hsv_to_rgb(row_hues[row], sat, val * val);

                let led_idx = row * COLS + col;
                strip.set(led_idx, color);
            }
        }
    });
}
