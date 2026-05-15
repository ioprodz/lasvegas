# Building a rhythm-locked animation

This document is a prompt for a Claude Code session. Paste it (or reference
it) when you want to design a new audio-reactive animation that reuses the
timing/audio scaffolding proven in `src/hardware/animation/audio_strobubble3.rs`,
but with a different visual idea.

The goal: you (the user) describe a new visual concept. Claude implements it
using the mechanics below. The visual is free; the rhythm/state machinery is
not.

---

## What you're building

A `fn audio_<name>(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis)`
that gets called once per frame (~30 fps) on a 60-wide × 6-tall LED grid.
Per frame: read audio, advance internal state, compute target positions or
field values, draw to the strip, apply onset overlays.

The rhythm structure is per-bar (4 beats nominal). Each bar is split into
named segments (e.g. MOVE/FLIP, or RISE/HOLD/DROP, or whatever fits the
visual). A continuous `bar_t ∈ [0, 1)` parameter drives the geometry; it
wraps at 1.0 and increments a per-bar state flag.

---

## Files you will touch

1. **Create** `src/hardware/animation/audio_<name>.rs` — the animation.
2. **Edit** `src/hardware/animation/mod.rs` — add `mod audio_<name>;` and
   `pub use audio_<name>::audio_<name>;`.
3. **Edit** `src/main.rs` — add a match arm in the dispatcher (search for
   `"audio_strobubble3"` to find the block).
4. **Edit** `static/index.html` — add an `<button class="anim-card" data-anim="audio_<name>" ...>` card; copy an existing one and rewrite the title + description. Keep the `data-audio="1"` and `draggable="true"` attributes.

After editing, run `cross build --target aarch64-unknown-linux-musl` to
verify it compiles. Do not attempt `cargo build` on macOS — it fails on the
`rs_ws281x` build script (Linux-only headers).

---

## The LED strip

- Geometry: 60 columns × 6 rows = 360 LEDs. Row 0 is top, row 5 is bottom.
- Index formula: `idx = row * 60 + col`.
- API on `LedStrip`:
  - `strip.set(idx, [r, g, b, 0])` — write one LED. The `0` is the W channel
    and is unused; always pass 0.
  - `strip.set_all([r, g, b, 0])` — fill the whole strip.
  - `strip.controller_leds() -> &mut [[u8; 4]]` — raw mutable slice for
    read-modify-write blending.
- Helper: `crate::hardware::led::hsv_to_rgb(h_deg: f32, s: 0..1, v: 0..1) -> [u8; 4]`.
- Centre row coordinate: `CY = (ROWS - 1) / 2.0 = 2.5`.
- Off-grid pixels are simply clipped — you can let shapes extend past the
  top/bottom edge and just `continue` when `row < 0 || row >= ROWS`.

---

## The audio signal

```rust
pub struct AudioAnalysis {
    pub bands: [u8; 8],   // 8 frequency bands, low → high
    pub kick: u8,         // kick-drum energy
    pub snare: u8,        // snare energy
    pub hihat: u8,        // hi-hat energy
    pub vocals: u8,       // vocal presence
    pub bass_line: u8,    // sustained bass
    pub bpm: u16,         // detected BPM (often noisy; prefer beat_phase)
    pub beat_phase: u8,   // 0–255 within current beat (wraps each beat)
    pub note_midi: u8,    // dominant pitch (MIDI #), 0 = none
    pub chord_root: u8,   // 0–11 (C..B), 255 = none
    pub chord_quality: u8,// 0 maj, 1 min, 2 dim, 3 aug, 4 7, 5 maj7, 6 m7,
                          // 7 sus2, 8 sus4, 9 power-5, 255 unknown
}
```

All `u8` fields are 0–255 ranges that you'll typically normalise to
`f32 / 255.0`.

### Smoothing values

Energy bands are noisy; smooth them with an attack/decay one-pole filter:

```rust
fn smooth(current: &mut f32, target: f32, attack: f32, decay: f32) {
    if target > *current { *current += (target - *current) * attack; }
    else                 { *current += (target - *current) * decay; }
}
```

Typical values: `attack ≈ 0.3–0.4`, `decay ≈ 0.05–0.1` (smoothed bass/mid).

### Onset detection

A clean way to fire "an event just happened":

```rust
let kick_onset = a.kick > s.prev_kick.saturating_add(THRESHOLD);
s.prev_kick = a.kick;
if kick_onset { s.kick_t = 1.0; /* spawn / explode / flash */ }
s.kick_t *= 0.60; // decay each frame
```

Typical thresholds: kick ≈ 25, snare ≈ 30, hihat ≈ 35. The decay factor
controls how long the visual after-effect lasts (0.60 ≈ 5–6 frames).

### Beat-onset detection (BPM-derived)

```rust
let curr_phase = a.beat_phase;
let beat_onset = (s.prev_beat_phase as i16) > 180 && (curr_phase as i16) < 60;
s.prev_beat_phase = curr_phase;
```

This fires once per detected beat. Useful for counting beats to drive a
bar-cycling state (or as a sanity check against `bar_t`).

### Chord-driven palette

```rust
const NOTE_HUES: [f32; 12] = [
    0.0, 30.0, 55.0, 80.0, 120.0, 160.0, 195.0, 220.0, 260.0, 285.0, 320.0, 345.0,
];

if a.chord_root < 12 {
    s.chord_hue_target = (NOTE_HUES[a.chord_root as usize] + s.palette_offset) % 360.0;
}
s.chord_hue = lerp_hue(s.chord_hue, s.chord_hue_target, 0.08);

fn lerp_hue(a: f32, b: f32, t: f32) -> f32 {
    let mut diff = b - a;
    if diff >  180.0 { diff -= 360.0; }
    if diff < -180.0 { diff += 360.0; }
    (a + diff * t).rem_euclid(360.0)
}
```

`palette_offset` is a per-progression offset you can derive from a hash of
the last N chord roots if you want each progression to feel slightly
different (see `compute_fingerprint` / `apply_fingerprint` in
`audio_strobubble3.rs`). Optional.

---

## The timing scaffold

```rust
const FRAMES_PER_BEAT_NOMINAL: f32 = 15.0;   // ~120 BPM at 30 fps
const BEATS_PER_BAR: usize = 4;
const BAR_T_PER_FRAME: f32 = 1.0 / (FRAMES_PER_BEAT_NOMINAL * BEATS_PER_BAR as f32);

// In the per-frame body:
let speed_mult = 1.0 + s.mid_smooth * 0.40 + s.bass_smooth * 0.20;
let new_bar_t = s.bar_t + BAR_T_PER_FRAME * speed_mult;
if new_bar_t >= 1.0 {
    s.bar_count += 1;            // or toggle a swap flag, etc.
}
s.bar_t = new_bar_t.rem_euclid(1.0);
```

`bar_t` advances continuously (not beat-stepped). The reason: stepping
geometry only on beats makes elements teleport between fixed positions and
look static between them. Continuous advance reads as actual motion.

Audio energy modulates the rate slightly so motion breathes with the music.
Coefficients 0.40 / 0.20 are tuned values — keep them small or motion
becomes erratic.

### Splitting the bar into segments

Pick segment boundaries as fractions of `bar_t`. For example, the proven
3-beats-then-1-beat split:

```rust
const SEG_A_END: f32 = 0.75;   // 3 of 4 beats
// SEG_B runs from 0.75 to 1.0
```

You can use any split (`0.5/0.5`, `0.25/0.5/0.25`, etc.). Beat-aligned
boundaries (multiples of `0.25`) are usually what feels right musically.

### Per-bar accumulating state

If a segment's motion ends in a state different from where the next bar's
first segment should start, you need a flag that flips at bar wrap. The
`swapped: bool` in `audio_strobubble3.rs` is an example — without it, the
position would jump at the bar boundary. Pattern:

```rust
struct State { ..., swapped: bool, ... }

if new_bar_t >= 1.0 { s.swapped = !s.swapped; }

// In the target function, derive an "effective" identity from swap state:
let eff_side = if swapped { 1.0 - side } else { side };
```

---

## Continuity rules (this is where animations glitch)

**Position continuity**: the position at the end of segment A must equal
the position at the start of segment B. Always check this on paper.

**Velocity continuity**: the *derivative* at the end of A must equal the
derivative at the start of B. If sin/cos curves meet at a zero-crossing
both segments will be at the same point, but the derivative there is at
its maximum — and the sign can differ between segments. That's the
classic glitch.

Concrete example (Strobubble V3, MOVE → FLIP at `bar_t = 0.75`):

- MOVE uses `y = CY + A · h_sign · sin(phase)` with `phase = (bar_t / 0.75) · 3π`.
  At `phase = 3π`: `sin = 0` (position OK), `cos = -1`, so
  `dy/d(bar_t) = -4π · A · h_sign`.
- FLIP uses `y = CY + R · sin(angle)` with `angle = start_angle + flip_progress · π`.
  At `flip_progress = 0`: `sin = 0`, `cos = ±1`, so
  `dy/d(bar_t) = ±4π · R`.

For the velocities to match:

1. Set the amplitude `A` equal to the orbit radius `R`. Then both
   derivative magnitudes are `4π · R`.
2. Match the signs. In Strobubble V3 this works automatically in the
   "unswapped" state but inverts in the "swapped" state, so the FLIP
   rotation direction is multiplied by `if swapped { -1.0 } else { 1.0 }`.

When you design a new animation, write out the parametric form of each
segment, take `d/d(bar_t)` at the seam, and check both sides agree (with
the right sign). If they don't, either:

- Choose segment counts/phases that align (e.g. 4 half-cycles instead of 3),
- Make geometric parameters match (`A = R` trick), or
- Introduce a per-bar accumulating state that flips a sign at the seam.

If exact matching isn't possible, **ease both segments to zero velocity at
the boundary** (`pulse = (1 - cos(p · 2π)) / 2` per beat, or smoothstep on
the segment parameter). Smooth-stop at every seam removes glitches at the
cost of a different motion feel.

---

## State persistence

Animations are called from a single thread; per-animation state lives in a
`thread_local!` `RefCell<State>`:

```rust
struct State {
    initialized: bool,
    frame: usize,
    seed: u32,
    // ... your state ...
    bar_t: f32,
    // smoothed audio:
    bass_smooth: f32,
    mid_smooth: f32,
    // chord:
    chord_hue: f32,
    chord_hue_target: f32,
    // onset trackers:
    prev_kick: u8,
    kick_t: f32,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State {
        initialized: false,
        // ... defaults ...
    });
}

pub fn audio_<name>(strip: &mut LedStrip, _frame: usize, a: &AudioAnalysis) {
    STATE.with(|state| {
        let s = &mut *state.borrow_mut();
        s.frame = s.frame.wrapping_add(1);
        if !s.initialized { /* one-time setup */ s.initialized = true; }
        // ... per-frame work ...
    });
}
```

A simple xorshift for randomness when you need it:

```rust
fn rand(&mut self) -> u32 {
    self.seed ^= self.seed << 13;
    self.seed ^= self.seed >> 17;
    self.seed ^= self.seed << 5;
    self.seed
}
```

---

## Render pattern

Per frame, in this order:

1. **Clear**: `strip.set_all([0, 0, 0, 0])` — black canvas. (If you want a
   wash, paint it first instead.)
2. **Draw your foreground** at positions derived from `bar_t` and audio
   state. Use max-blending when multiple shapes overlap:
   ```rust
   let base = strip.controller_leds()[idx];
   strip.set(idx, [base[0].max(c[0]), base[1].max(c[1]), base[2].max(c[2]), 0]);
   ```
3. **Onset overlays** (last, so they sit on top):
   - Hi-hat coloured micro-strobe (additive max-blend):
     ```rust
     if s.hihat_t > 0.12 {
         let c = hsv_to_rgb((chord_hue + 60.0) % 360.0, 0.70, s.hihat_t * 0.45);
         /* max-blend across all LEDs */
     }
     ```
   - Kick white flash (lerp toward white by `kick_t`):
     ```rust
     if s.kick_t > 0.15 {
         for i in 0..NUM_LEDS {
             let e = leds[i];
             let mix = s.kick_t;
             leds[i] = [
                 (e[0] as f32 * (1.0 - mix) + 255.0 * mix) as u8,
                 (e[1] as f32 * (1.0 - mix) + 255.0 * mix) as u8,
                 (e[2] as f32 * (1.0 - mix) + 255.0 * mix) as u8,
                 0,
             ];
         }
     }
     ```

Helpers for soft-edged shapes (copy from `audio_strobubble3.rs`):
- `draw_circle(strip, cx, cy, radius, hue, sat, brightness)` — disc with
  a solid 70 % core and linear taper to the edge.
- `draw_ring(strip, cx, cy, ring_r, thickness, hue, sat, brightness)` —
  hollow ring, useful for explosion effects on kick onsets.

---

## Reference implementation

`src/hardware/animation/audio_strobubble3.rs` is the canonical example of
all the mechanics above. Read it before starting. The visual decisions there
(circles, pair structure, MOVE/FLIP segmentation, complementary hues,
explosion-on-kick) are *visual choices* — you're free to replace any of
them.

What is *not* a visual choice and should be kept:

- The `bar_t` driver and its wrap-detection.
- The audio smoothing and onset-detection patterns.
- The chord-hue tracker.
- The `thread_local!` state pattern.
- The render order (clear → foreground → hihat overlay → kick flash).
- The position-and-velocity continuity discipline at segment seams.

---

## Designing a new animation — the brief

Tell Claude:

1. **The visual**: what is on the strip? (particles? a wave? bars? a face?
   text? falling rain? a horizon line?)
2. **The rhythm structure**: how is a bar split? What happens in each
   segment? Beat-aligned where?
3. **Audio reactivity** beyond the standard kick-flash/hihat-strobe: what
   should respond to bass, what to chord changes, what to onsets?
4. **Anything that should accumulate over multiple bars** (palette
   rotations, side-swaps, mode changes, etc.)?

Claude will write the animation following this scaffold, do the velocity
check on paper at every seam, then `cross build` to verify.
