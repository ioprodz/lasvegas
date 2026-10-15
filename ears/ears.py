"""lasvegas-ears: voice commands for the LED strip.

Reads the raw mic stream that the LED server re-publishes on 127.0.0.1:8081
(S16_LE, 44.1 kHz, mono), recognises a fixed list of English phrases with
Vosk (offline, small English model), and sends the matching command to the
LED server's WebSocket (ws://127.0.0.1:8080).

Every command starts with the wake phrase (default "room light"), e.g.
"room light rainbow", "room light off", "room light red". The wake phrase
alone toggles the lights.

"mini pc start" wakes the mini-pc desktop (Wake-on-LAN magic packet);
"mini pc shutdown" / "mini pc restart" power it off or reboot it over SSH
with a key that mini-pc only lets run poweroff/reboot. These have their own
recognition confidence threshold (EARS_PC_MIN_CONF).

"tv on" / "tv off" power the LG TV (see tv.py; Wake-on-LAN and its local API).
"tv youtube|netflix|jellyfin" opens an app (turning the TV on first);
"tv play|pause|stop|rewind|forward" control playback. Every TV command but
"tv on" needs a high confidence (EARS_TV_MIN_CONF): the TV's own audio
produced a false "tv off" at 0.80.
"""

import base64
import json
import os
import socket
import subprocess
import sys
import threading
import time

from vosk import KaldiRecognizer, Model, SetLogLevel

import tv

TAP = ("127.0.0.1", 8081)
WS = ("127.0.0.1", 8080)
RATE = 44100
WAKE = os.environ.get("EARS_WAKE", "room light")
MODEL_DIR = os.environ.get("EARS_MODEL", os.path.join(os.path.dirname(__file__), "model"))
# Vosk word confidence (0–1) every word of a phrase must reach.
MIN_CONF = float(os.environ.get("EARS_MIN_CONF", "0.6"))

PC_WAKE = os.environ.get("EARS_PC_WAKE", "mini pc")
PC_HOST = os.environ.get("EARS_PC_HOST", "osminosm@mini-pc.local")
PC_MAC = os.environ.get("EARS_PC_MAC", "38:05:25:30:31:d0")
PC_KEY = os.path.expanduser(os.environ.get("EARS_PC_KEY", "~/.ssh/minipc_power"))
PC_MIN_CONF = float(os.environ.get("EARS_PC_MIN_CONF", "0.55"))

ANIMATIONS = {
    "rainbow": "rainbow",
    "pulse": "pulse",
    "chase": "chase",
    "fire": "audio_fire",
    "party": "audio_headliner",
    "party mode": "audio_headliner",
    "music": "audio_harmonic5",
    "music mode": "audio_harmonic5",
    "tunnel": "audio_tunnel",
    "sparkle": "audio_sparkle",
    "milky way": "audio_milkyway",
    "arcade": "retro_arcade",
    "game of life": "game_of_life",
}

COLORS = {
    "red": (255, 0, 0),
    "green": (0, 255, 0),
    "blue": (0, 0, 255),
    "white": (255, 255, 255),
    "warm": (255, 140, 40),
    "warm white": (255, 140, 40),
    "purple": (160, 0, 255),
    "pink": (255, 40, 120),
    "orange": (255, 100, 0),
    "yellow": (255, 200, 0),
    "cyan": (0, 255, 255),
}

TEXT_COMMANDS = {
    "lights on": "lights:on",
    "on": "lights:on",
    "lights off": "lights:off",
    "off": "lights:off",
    "stop": "lights:off",
}


TV_WAKE = os.environ.get("EARS_TV_WAKE", "tv")
TV_MIN_CONF = float(os.environ.get("EARS_TV_MIN_CONF", "0.9"))

TV_APPS = {
    "youtube": "youtube.leanback.v4",
    "you tube": "youtube.leanback.v4",
    "netflix": "netflix",
    "jelly fin": "org.jellyfin.webos",
    "jelly": "org.jellyfin.webos",
}

TV_MEDIA = {
    "play": "play",
    "resume": "play",
    "pause": "pause",
    "stop": "stop",
    "rewind": "rewind",
    "forward": "fastForward",
    "fast forward": "fastForward",
}

PC_COMMANDS = {
    "start": "wake",
    "wake up": "wake",
    "shutdown": "poweroff",
    "shut down": "poweroff",
    "restart": "reboot",
    "reboot": "reboot",
}


def log(*args):
    print(*args, flush=True)


def phrases():
    return (
        [WAKE]
        + [f"{WAKE} {p}" for p in [*TEXT_COMMANDS, *ANIMATIONS, *COLORS]]
        + [f"{PC_WAKE} {p}" for p in PC_COMMANDS]
        + [f"{TV_WAKE} {p}" for p in ["on", "off", *TV_APPS, *TV_MEDIA]]
    )


def min_conf(text):
    """Confidence a phrase needs: per-command overrides, else MIN_CONF."""
    if text.startswith(TV_WAKE + " ") and text != f"{TV_WAKE} on":
        return TV_MIN_CONF
    if text.startswith(PC_WAKE + " "):
        return PC_MIN_CONF
    return MIN_CONF


def wake_pc():
    """Broadcast a Wake-on-LAN magic packet for mini-pc's Ethernet port."""
    mac = bytes.fromhex(PC_MAC.replace(":", ""))
    packet = b"\xff" * 6 + mac * 16
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
        s.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
        s.sendto(packet, ("255.255.255.255", 9))


def power_pc(action):
    """Ask mini-pc to power off or reboot; runs in the background and logs."""

    def run():
        try:
            r = subprocess.run(
                ["ssh", "-i", PC_KEY, "-o", "BatchMode=yes", "-o", "ConnectTimeout=5",
                 "-o", "HostKeyAlias=mini-pc", PC_HOST, action],
                capture_output=True, text=True, timeout=20,
            )
            # A poweroff/reboot may drop the connection (exit 255): that's success.
            if r.returncode not in (0, 255) or "denied" in r.stderr.lower():
                log(f"mini-pc {action} failed ({r.returncode}): {r.stderr.strip()}")
        except (OSError, subprocess.TimeoutExpired) as e:
            log(f"mini-pc {action} failed: {e}")

    threading.Thread(target=run, daemon=True).start()


def ws_send(payload):
    """Open a WebSocket, send one frame (str = text, bytes = binary), close.

    A connection per command keeps this simple: the server broadcasts LED
    frames to every client, so a long-lived idle client would have to drain them.
    """
    data = payload.encode() if isinstance(payload, str) else payload
    opcode = 0x1 if isinstance(payload, str) else 0x2
    with socket.create_connection(WS, timeout=3) as s:
        key = base64.b64encode(os.urandom(16)).decode()
        s.sendall(
            (
                f"GET / HTTP/1.1\r\nHost: {WS[0]}:{WS[1]}\r\nUpgrade: websocket\r\n"
                f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
                "Sec-WebSocket-Version: 13\r\n\r\n"
            ).encode()
        )
        resp = b""
        while b"\r\n\r\n" not in resp:
            chunk = s.recv(1024)
            if not chunk:
                raise ConnectionError("WebSocket handshake failed")
            resp += chunk
        if b" 101 " not in resp.split(b"\r\n", 1)[0]:
            raise ConnectionError("WebSocket upgrade refused")
        mask = os.urandom(4)
        header = bytes([0x80 | opcode, 0x80 | len(data)])  # commands are < 126 bytes
        s.sendall(header + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(data)))
        s.sendall(bytes([0x88, 0x80]) + os.urandom(4))  # close frame
        time.sleep(0.05)


def background(label, fn, *args):
    """Run a TV call without blocking the listener; log failures."""

    def run():
        try:
            fn(*args)
        except Exception as e:  # network, TLS or websockets errors: log, keep listening
            log(f"{label} failed: {e!r}")

    threading.Thread(target=run, daemon=True).start()


def handle(text):
    """Map a recognised phrase to a command. Returns a description or None."""
    if text == f"{TV_WAKE} on":
        tv.wake()
        return "tv wake"
    if text == f"{TV_WAKE} off":
        background("tv off", tv.off)
        return "tv off"
    if text.startswith(TV_WAKE + " "):
        what = text[len(TV_WAKE) + 1:]
        if what in TV_APPS:
            background(f"tv launch {TV_APPS[what]}", tv.launch, TV_APPS[what])
            return f"tv launch {TV_APPS[what]}"
        if what in TV_MEDIA:
            background(f"tv {TV_MEDIA[what]}", tv.media, TV_MEDIA[what])
            return f"tv {TV_MEDIA[what]}"
    if text.startswith(PC_WAKE + " "):
        action = PC_COMMANDS.get(text[len(PC_WAKE) + 1:])
        if action == "wake":
            wake_pc()
            return "mini-pc wake"
        if action:
            power_pc(action)
            return f"mini-pc {action}"
        return None
    if text == WAKE:
        ws_send("lights:toggle")
        return "lights:toggle"
    if not text.startswith(WAKE + " "):
        return None
    what = text[len(WAKE) + 1:]
    if what in TEXT_COMMANDS:
        ws_send(TEXT_COMMANDS[what])
        return TEXT_COMMANDS[what]
    if what in ANIMATIONS:
        ws_send("animate:" + ANIMATIONS[what])
        return "animate:" + ANIMATIONS[what]
    if what in COLORS:
        r, g, b = COLORS[what]
        ws_send(bytes([0x01, r, g, b]))
        return f"color {r},{g},{b}"
    return None


def listen(model):
    grammar = json.dumps(phrases() + ["[unk]"])
    while True:
        try:
            with socket.create_connection(TAP, timeout=10) as tap:
                log(f"Connected to audio tap {TAP[0]}:{TAP[1]}")
                rec = KaldiRecognizer(model, RATE, grammar)
                rec.SetWords(True)
                while True:
                    chunk = tap.recv(8192)
                    if not chunk:
                        raise ConnectionError("audio tap closed")
                    if not rec.AcceptWaveform(chunk):
                        continue
                    result = json.loads(rec.Result())
                    text = result.get("text", "")
                    if not text or text == "[unk]":
                        continue
                    words = result.get("result", [])
                    conf = min((w.get("conf", 0) for w in words), default=0)
                    if conf < min_conf(text):
                        log(f"Heard '{text}' (conf {conf:.2f}), ignored")
                        continue
                    try:
                        done = handle(text)
                    except OSError as e:
                        log(f"Heard '{text}' but sending failed: {e}")
                        continue
                    if done:
                        log(f"Heard '{text}' (conf {conf:.2f}) -> {done}")
        except (OSError, ConnectionError) as e:
            # No audio while the LED server restarts or has no mic open.
            log(f"Audio tap unavailable ({e}); retrying in 5 s")
            time.sleep(5)


def main():
    SetLogLevel(-1)
    if not os.path.isdir(MODEL_DIR):
        sys.exit(f"Vosk model not found in {MODEL_DIR}")
    log(f"Loading model from {MODEL_DIR}")
    model = Model(MODEL_DIR)
    log(f"Ready, {len(phrases())} phrases, wake word '{WAKE}'")
    listen(model)


if __name__ == "__main__":
    main()
