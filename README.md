# lasvegas

A Rust WebSocket server for controlling WS2811 LED strips on a Raspberry Pi. Connect over WebSocket and send RGB colors to light up 360 LEDs simultaneously.

## How It Works

The application starts a WebSocket server on port `8080` and initializes a WS2811 LED strip controller via GPIO pin 18. Clients connect over WebSocket and send binary messages to set the color of all LEDs at once.

### LED Hardware Configuration

| Parameter   | Value              |
|-------------|--------------------|
| Strip type  | WS2811 (GBR order) |
| GPIO pin    | 18                 |
| LED count   | 360 (60 x 6)      |
| Frequency   | 800 kHz            |
| DMA channel | 10                 |
| Brightness  | 255 (max)          |

### WebSocket Protocol

- **Server address:** `ws://<raspberry-pi-ip>:8080`
- **Text messages:** Logged to console (no LED effect)
- **Binary messages:** Bytes `[_, R, G, B, ...]` — byte 0 is ignored, bytes 1-3 set the RGB color for all LEDs

Example (JavaScript):
```js
const ws = new WebSocket('ws://192.168.1.100:8080');
ws.onopen = () => {
  // Send red: [padding, R, G, B]
  ws.send(new Uint8Array([0, 255, 0, 0]));
};
```

## Project Structure

```
src/main.rs          — All application logic (WebSocket server + LED control)
Cargo.toml           — Dependencies: rs_ws281x, simple-websockets
docs/cross-compile.md — Notes on cross-compiling from macOS to ARM
.cargo/              — Disabled cross-compilation config
```

## Dependencies

- **[rs_ws281x](https://crates.io/crates/rs_ws281x)** (0.4.4) — Rust bindings for the rpi_ws281x C library
- **[simple-websockets](https://crates.io/crates/simple-websockets)** (0.1.5) — WebSocket server built on Tokio

## Building & Running

### On the Raspberry Pi directly

```sh
cargo build --release
sudo ./target/release/lasvegas
```

> `sudo` is required for GPIO/DMA access.

### Cross-compiling from macOS

1. Add the ARM target:
   ```sh
   rustup target add armv7-unknown-linux-musleabihf
   ```

2. Install toolchain:
   ```sh
   brew install arm-linux-gnueabihf-binutils
   brew install llvm
   ```

3. Set environment:
   ```sh
   export DYLD_LIBRARY_PATH=/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/
   ```

4. Rename `.cargo/ignore_config.toml` to `.cargo/config.toml` to enable the cross-compilation target, then:
   ```sh
   cargo build --target armv7-unknown-linux-musleabihf --release
   ```

5. Copy the binary to the Pi and run with `sudo`.

## Architecture Notes

- Single-threaded event loop: the WebSocket event hub is polled in a blocking loop
- All 360 LEDs are always set to the same color (no individual LED addressing exposed via WebSocket yet)
- On connect, the server sends a `"test"` text message to the client
- Initial LED color on startup: `[255, 0, 50, 0]` (reddish)
