use crate::hardware::led::{hsv_to_rgb, LedStrip, LED_COUNT};
use std::cell::RefCell;

const MAX_SPARKS: usize = 80;

struct Spark {
    pos: usize,
    life: f32,
    max_life: f32,
    hue: f32,
}

struct SparkleState {
    sparks: Vec<Spark>,
    seed: u32,
}

impl SparkleState {
    fn rand_u32(&mut self) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed
    }
}

thread_local! {
    static STATE: RefCell<SparkleState> = RefCell::new(SparkleState {
        sparks: Vec::new(),
        seed: 42,
    });
}

/// Sparkle rain — high frequencies spawn bright short-lived sparks at
/// random positions. Bass drives a dim background glow color. Mid
/// frequencies control spark color drift.
pub fn audio_sparkle(strip: &mut LedStrip, bands: &[u8; 8]) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();

        let bass = ((bands[0] as u16 + bands[1] as u16) / 2) as f32 / 255.0;
        let mid = ((bands[3] as u16 + bands[4] as u16) / 2) as f32 / 255.0;
        let high = ((bands[5] as u16 + bands[6] as u16 + bands[7] as u16) / 3) as f32 / 255.0;

        // Spawn sparks proportional to high-frequency energy
        let spawn_count = (high * 6.0) as usize;
        let mid_hue = mid * 360.0;

        for _ in 0..spawn_count {
            if s.sparks.len() >= MAX_SPARKS {
                break;
            }
            let pos = (s.rand_u32() as usize) % LED_COUNT;
            let life_variance = 10.0 + (s.rand_u32() % 25) as f32;
            let hue_jitter = (s.rand_u32() % 60) as f32 - 30.0;
            s.sparks.push(Spark {
                pos,
                life: 0.0,
                max_life: life_variance,
                hue: (mid_hue + hue_jitter).rem_euclid(360.0),
            });
        }

        let len = strip.len();

        // Background glow from bass: dim warm color
        let bg_val = bass * 0.15;
        let bg_hue = bass * 30.0;
        let bg = hsv_to_rgb(bg_hue, 0.8, bg_val);

        for i in 0..len {
            strip.set(i, bg);
        }

        // Render sparks on top
        for spark in s.sparks.iter() {
            let t = spark.life / spark.max_life;
            // Quick flash up then slow fade: triangle envelope peaking at 20%
            let brightness = if t < 0.2 {
                t / 0.2
            } else {
                1.0 - (t - 0.2) / 0.8
            };
            let brightness = brightness * brightness;

            let color = hsv_to_rgb(spark.hue, 0.7, brightness);
            strip.set(spark.pos, color);

            // Dim neighbors for soft glow
            if spark.pos > 0 {
                strip.set(spark.pos - 1, hsv_to_rgb(spark.hue, 0.7, brightness * 0.3));
            }
            if spark.pos + 1 < len {
                strip.set(spark.pos + 1, hsv_to_rgb(spark.hue, 0.7, brightness * 0.3));
            }
        }

        s.sparks.iter_mut().for_each(|sp| sp.life += 1.0);
        s.sparks.retain(|sp| sp.life < sp.max_life);
    });
}
