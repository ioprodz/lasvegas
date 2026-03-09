use super::calibration::Calibration;
use rs_ws281x::{ChannelBuilder, ControllerBuilder, StripType};
use std::thread::sleep;
use std::time::Duration;

pub const LED_COUNT: usize = 60 * 6;
const LEDS_PER_ROW: usize = 60;
const GPIO_PIN: i32 = 18;
const FREQUENCY: u32 = 800_000;
const DMA_CHANNEL: i32 = 10;

pub struct LedStrip {
    controller: rs_ws281x::Controller,
}

impl LedStrip {
    pub fn new() -> Self {
        let controller = ControllerBuilder::new()
            .freq(FREQUENCY)
            .dma(DMA_CHANNEL)
            .channel(
                0,
                ChannelBuilder::new()
                    .pin(GPIO_PIN)
                    .count(LED_COUNT as i32)
                    .strip_type(StripType::Ws2811Gbr)
                    .brightness(255)
                    .build(),
            )
            .build()
            .unwrap();
        Self { controller }
    }

    pub fn len(&self) -> usize {
        LED_COUNT
    }

    pub fn set(&mut self, index: usize, color: [u8; 4]) {
        self.controller.leds_mut(0)[index] = color;
    }

    /// Direct read access to LED buffer (for blending operations).
    pub fn controller_leds(&mut self) -> &mut [[u8; 4]] {
        self.controller.leds_mut(0)
    }

    pub fn set_all(&mut self, color: [u8; 4]) {
        for led in self.controller.leds_mut(0) {
            *led = color;
        }
    }

    /// Render the current buffer to the hardware, applying calibration and zigzag correction.
    pub fn render(&mut self) {
        self.render_with_calibration(None);
    }

    /// Render with optional calibration applied before sending to hardware.
    pub fn render_calibrated(&mut self, cal: &Calibration) {
        self.render_with_calibration(Some(cal));
    }

    fn render_with_calibration(&mut self, cal: Option<&Calibration>) {
        let leds = self.controller.leds_mut(0);
        let num_rows = LED_COUNT / LEDS_PER_ROW;

        // Apply calibration: transform colors in-place, remember originals
        let originals: Option<Vec<[u8; 4]>> = cal.map(|c| {
            let orig: Vec<[u8; 4]> = leds.iter().copied().collect();
            for led in leds.iter_mut() {
                *led = c.apply(*led);
            }
            orig
        });

        // Apply zigzag: reverse odd rows to match physical wiring
        for row in 0..num_rows {
            if row % 2 == 1 {
                let start = row * LEDS_PER_ROW;
                let end = start + LEDS_PER_ROW;
                leds[start..end].reverse();
            }
        }

        self.controller.render().unwrap();

        // Reverse zigzag back
        let leds = self.controller.leds_mut(0);
        for row in 0..num_rows {
            if row % 2 == 1 {
                let start = row * LEDS_PER_ROW;
                let end = start + LEDS_PER_ROW;
                leds[start..end].reverse();
            }
        }

        // Restore original uncalibrated colors
        if let Some(orig) = originals {
            let leds = self.controller.leds_mut(0);
            for (led, o) in leds.iter_mut().zip(orig.iter()) {
                *led = *o;
            }
        }
    }

    /// Read current LED state as flat [r, g, b, r, g, b, ...] vec.
    pub fn read_state(&mut self) -> Vec<u8> {
        let leds = self.controller.leds_mut(0);
        let mut state = Vec::with_capacity(leds.len() * 3);
        for led in leds.iter() {
            state.push(led[0]); // r
            state.push(led[1]); // g
            state.push(led[2]); // b
        }
        state
    }

    pub fn startup_animation(&mut self) {
        println!("Running startup animation...");

        // 1. Fast white wipe — 6 LEDs at a time
        for i in (0..LED_COUNT).step_by(6) {
            for j in i..(i + 6).min(LED_COUNT) {
                self.set(j, [255, 255, 255, 0]);
            }
            self.render();
            sleep(Duration::from_millis(5));
        }

        // 2. R/G/B flash
        for &color in &[[255, 0, 0, 0], [0, 255, 0, 0], [0, 0, 255, 0]] {
            self.set_all(color);
            self.render();
            sleep(Duration::from_millis(200));
        }

        // 3. Quick rainbow sweep
        for offset in (0..360).step_by(3) {
            for i in 0..LED_COUNT {
                let hue = ((i + offset) % 360) as f32;
                self.set(i, hsv_to_rgb(hue, 1.0, 1.0));
            }
            self.render();
            sleep(Duration::from_millis(10));
        }

        // 4. Fast fade out
        for brightness in (0u8..=255).rev().step_by(15) {
            self.set_all([brightness, brightness, brightness, 0]);
            self.render();
            sleep(Duration::from_millis(20));
        }

        self.set_all([0, 0, 0, 0]);
        self.render();
        println!("Startup animation complete.");
    }
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 4] {
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
    [
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
        0,
    ]
}
