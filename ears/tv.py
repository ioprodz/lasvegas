"""LG webOS TV control for lasvegas-ears.

Power: "on" sends a Wake-on-LAN magic packet (the TV's "TV On With Mobile"
setting must be on); everything else goes over the TV's local SSAP
WebSocket API (wss://<tv>:3001). The TV's certificate is pinned by SHA-256
fingerprint instead of trusting it blindly.

Config lives next to this file in tv.json (not in git, mode 600):
    {"host": "<tv ip>", "mac": "<tv mac>",
     "cert_sha256": "<hex>", "client_key": "<set by pairing>"}

Pair with the TV on, and accept the prompt on the TV screen (again whenever
PERMISSIONS changes):
    venv/bin/python tv.py pair
Manual use:
    venv/bin/python tv.py on | off | state | apps | foreground
    venv/bin/python tv.py launch <app id>
    venv/bin/python tv.py media play|pause|stop|rewind|fastForward
    venv/bin/python tv.py button <remote button, e.g. PLAY, PAUSE, HOME, BACK>
"""

import hashlib
import itertools
import json
import os
import socket
import ssl
import sys
import time
from contextlib import contextmanager

from websockets.sync.client import connect

CONFIG = os.path.join(os.path.dirname(os.path.abspath(__file__)), "tv.json")
PERMISSIONS = [
    "CONTROL_POWER",
    "CONTROL_AUDIO",
    "CONTROL_TV_SCREEN",
    "READ_POWER_STATE",
    "LAUNCH",
    "READ_INSTALLED_APPS",
    "READ_RUNNING_APPS",
    "CONTROL_INPUT_MEDIA_PLAYBACK",
    "CONTROL_INPUT_JOYSTICK",
]
MEDIA = {"play", "pause", "stop", "rewind", "fastForward"}
_ids = itertools.count(1)


def load():
    with open(CONFIG) as f:
        return json.load(f)


def save(cfg):
    fd = os.open(CONFIG + ".tmp", os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        json.dump(cfg, f, indent=1)
    os.replace(CONFIG + ".tmp", CONFIG)


def wake(cfg=None):
    cfg = cfg or load()
    mac = bytes.fromhex(cfg["mac"].replace(":", ""))
    packet = b"\xff" * 6 + mac * 16
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
        s.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
        for _ in range(3):
            s.sendto(packet, ("255.255.255.255", 9))


@contextmanager
def _connect(cfg, url):
    """Open a wss connection to the TV and check the pinned certificate."""
    # The TV's certificate is signed by LG's own CA, not a public one: skip CA
    # validation, then compare the exact certificate against the pinned hash.
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    with connect(url, ssl=ctx, open_timeout=5, close_timeout=2, max_size=2**20) as ws:
        der = ws.socket.getpeercert(binary_form=True)
        if hashlib.sha256(der).hexdigest() != cfg["cert_sha256"].replace(":", "").lower():
            raise ConnectionError("TV certificate does not match the pinned fingerprint")
        yield ws


@contextmanager
def _session(cfg, timeout=10):
    """Connect and register; yields the socket."""
    with _connect(cfg, f"wss://{cfg['host']}:3001") as ws:
        _register(ws, cfg, timeout)
        yield ws


def _register(ws, cfg, timeout):
    payload = {"forcePairing": False, "pairingType": "PROMPT",
               "manifest": {"manifestVersion": 1, "permissions": PERMISSIONS}}
    if cfg.get("client_key"):
        payload["client-key"] = cfg["client_key"]
    ws.send(json.dumps({"type": "register", "id": "register_0", "payload": payload}))
    while True:
        msg = json.loads(ws.recv(timeout=timeout))
        if msg.get("type") == "registered":
            key = msg.get("payload", {}).get("client-key")
            if key and key != cfg.get("client_key"):
                cfg["client_key"] = key
                save(cfg)
            return
        if msg.get("type") == "error":
            raise ConnectionError(f"TV refused registration: {msg.get('error')}")
        # {"type": "response", "payload": {"pairingType": "PROMPT"}}: waiting for the user


def _call(ws, uri, payload=None, timeout=10):
    rid = f"req_{next(_ids)}"
    msg = {"type": "request", "id": rid, "uri": uri}
    if payload is not None:
        msg["payload"] = payload
    ws.send(json.dumps(msg))
    while True:
        reply = json.loads(ws.recv(timeout=timeout))
        if reply.get("id") == rid:
            if reply.get("type") == "error":
                raise ConnectionError(f"TV error for {uri}: {reply.get('error')}")
            return reply.get("payload", {})


def request(uri, payload=None, cfg=None):
    cfg = cfg or load()
    with _session(cfg) as ws:
        return _call(ws, uri, payload)


def off(cfg=None):
    return request("ssap://system/turnOff", cfg=cfg)


def state(cfg=None):
    """'Active', 'Screen Off', 'Active Standby', ... or 'Off' if unreachable."""
    try:
        return request("ssap://com.webos.service.tvpower/power/getPowerState", cfg=cfg).get("state", "?")
    except (OSError, TimeoutError):
        return "Off"


def ensure_on(cfg=None, wait=30):
    """Turn the TV on if needed and wait until it accepts commands."""
    cfg = cfg or load()
    s = state(cfg)
    if s == "Active":
        return False
    if s == "Screen Off":
        request("ssap://com.webos.service.tvpower/power/turnOnScreen", cfg=cfg)
        return True
    wake(cfg)
    deadline = time.time() + wait
    while time.time() < deadline:
        time.sleep(1)
        if state(cfg) == "Active":
            time.sleep(2)  # the launcher needs a moment after power-on
            return True
        wake(cfg)
    raise TimeoutError(f"TV did not turn on within {wait} s")


def launch(app_id, cfg=None):
    cfg = cfg or load()
    ensure_on(cfg)
    return request("ssap://system.launcher/launch", {"id": app_id}, cfg=cfg)


def apps(cfg=None):
    points = request("ssap://com.webos.applicationManager/listLaunchPoints", cfg=cfg)
    return sorted((p["title"], p["id"]) for p in points.get("launchPoints", []))


def foreground(cfg=None):
    return request("ssap://com.webos.applicationManager/getForegroundAppInfo", cfg=cfg).get("appId")


def media(cmd, cfg=None):
    if cmd not in MEDIA:
        raise ValueError(f"unknown media command {cmd!r}")
    return request(f"ssap://media.controls/{cmd}", cfg=cfg)


def button(name, cfg=None):
    """Press a remote-control button through the TV's pointer input socket."""
    cfg = cfg or load()
    with _session(cfg) as ws:
        path = _call(ws, "ssap://com.webos.service.networkinput/getPointerInputSocket")["socketPath"]
    with _connect(cfg, path) as pointer:
        pointer.send(f"type:button\nname:{name}\n\n")
        time.sleep(0.1)


def pair():
    cfg = load()
    cfg.pop("client_key", None)
    print("Accept the prompt on the TV (60 s)...", flush=True)
    with _session(cfg, timeout=60):
        pass
    print("Paired; key saved in", CONFIG)


if __name__ == "__main__":
    cmd, args = (sys.argv[1], sys.argv[2:]) if len(sys.argv) > 1 else ("", [])
    if cmd == "pair":
        pair()
    elif cmd == "on":
        wake()
    elif cmd == "off":
        print(off())
    elif cmd == "state":
        print(state())
    elif cmd == "apps":
        for title, app_id in apps():
            print(f"{app_id:40} {title}")
    elif cmd == "foreground":
        print(foreground())
    elif cmd == "launch" and args:
        print(launch(args[0]))
    elif cmd == "media" and args:
        print(media(args[0]))
    elif cmd == "button" and args:
        button(args[0])
    else:
        sys.exit(__doc__)
