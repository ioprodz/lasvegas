"""LG webOS TV power for lasvegas-ears.

"tv on" sends a Wake-on-LAN magic packet (the TV's "TV On With Mobile"
setting must be on); "tv off" asks the TV to turn off over its local
SSAP WebSocket API (wss://<tv>:3001). The TV's certificate is pinned by
SHA-256 fingerprint instead of trusting it blindly.

Config lives next to this file in tv.json (not in git, mode 600):
    {"host": "<tv ip>", "mac": "<tv mac>",
     "cert_sha256": "<hex>", "client_key": "<set by pairing>"}

Pair once, with the TV on, and accept the prompt on the TV screen:
    venv/bin/python tv.py pair
Manual use: venv/bin/python tv.py on|off
"""

import hashlib
import json
import os
import socket
import ssl
import sys
from contextlib import contextmanager

from websockets.sync.client import connect

CONFIG = os.path.join(os.path.dirname(os.path.abspath(__file__)), "tv.json")
PERMISSIONS = ["CONTROL_POWER", "CONTROL_AUDIO"]


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
def _session(cfg, timeout):
    """Connect, check the pinned certificate, register; yields the socket."""
    # The TV's certificate is signed by LG's own CA, not a public one: skip CA
    # validation, then compare the exact certificate against the pinned hash.
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    with connect(f"wss://{cfg['host']}:3001", ssl=ctx, open_timeout=5,
                 close_timeout=2, max_size=2**20) as ws:
        der = ws.socket.getpeercert(binary_form=True)
        if hashlib.sha256(der).hexdigest() != cfg["cert_sha256"].replace(":", "").lower():
            raise ConnectionError("TV certificate does not match the pinned fingerprint")
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


def request(uri, cfg=None, timeout=10):
    cfg = cfg or load()
    with _session(cfg, timeout) as ws:
        ws.send(json.dumps({"type": "request", "id": "req_1", "uri": uri}))
        while True:
            msg = json.loads(ws.recv(timeout=timeout))
            if msg.get("id") == "req_1":
                if msg.get("type") == "error":
                    raise ConnectionError(f"TV error: {msg.get('error')}")
                return msg.get("payload", {})


def off(cfg=None):
    return request("ssap://system/turnOff", cfg)


def pair():
    cfg = load()
    cfg.pop("client_key", None)
    print("Accept the prompt on the TV (60 s)...", flush=True)
    with _session(cfg, timeout=60):
        pass
    print("Paired; key saved in", CONFIG)


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    if cmd == "pair":
        pair()
    elif cmd == "on":
        wake()
    elif cmd == "off":
        print(off())
    else:
        sys.exit("usage: tv.py pair|on|off")
