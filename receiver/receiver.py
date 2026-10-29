#!/usr/bin/env python3
"""Receiver for anonymous Amtrak Network Monitor trip copies.

One endpoint: POST /amtrak/upload, body is the "-share.jsonl" file the
monitor wrote, header `Authorization: Bearer <tester token>`.

It is meant to sit on 127.0.0.1 behind a web server that speaks HTTPS.
It never merges what it receives into anything. Every accepted upload is
one file in a quarantine folder, named for the tester, until a person has
looked at it.

    receiver.py serve                          run the receiver
    receiver.py add-tester NAME URL OUTFILE    make a token; write share.json
    receiver.py revoke NAME                    stop accepting a tester
    receiver.py list                           show what has arrived

Settings come from the environment:
    AMTRAK_RECEIVER_DIR   data folder (default /var/lib/amtrak-receiver)
    AMTRAK_RECEIVER_PORT  port on 127.0.0.1 (default 8765)
"""
import hashlib
import hmac
import json
import os
import re
import secrets
import sys
import threading
import time
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

MAX_BODY = 1_000_000
MAX_ROWS = 10_000
MAX_UPLOADS_PER_TESTER_PER_DAY = 10
MAX_UPLOADS_PER_DAY = 100
MAX_STORE_BYTES = 200_000_000
SOCKET_TIMEOUT_S = 15

PATH_UPLOAD = "/amtrak/upload"
PATH_HEALTH = "/amtrak/health"

KEYS = {"line", "lat", "lon", "tone", "loss_pct", "rtt_ms", "http_code", "http_s"}
TONES = {"clear", "uneven", "poor", "down", "no recent sample"}
# The names the monitor offers (src/lines.rs). A line is free text on the
# rider's side, so anything else is refused rather than stored.
KNOWN_LINES = {
    "Acela", "Adirondack", "Auto Train", "Berkshire Flyer", "Blue Water", "Borealis",
    "California Zephyr", "Capitol Corridor", "Cardinal", "Carl Sandburg", "Carolinian",
    "Cascades", "City of New Orleans", "Coast Starlight", "Crescent", "Downeaster",
    "Empire Builder", "Empire Service", "Ethan Allen Express", "Floridian", "Gold Runner",
    "Hartford Line", "Heartland Flyer", "Hiawatha", "Illini", "Illinois Zephyr",
    "Keystone Service", "Lake Shore Limited", "Lincoln Service", "Maple Leaf", "Mardi Gras",
    "Missouri River Runner", "Northeast Regional", "Pacific Surfliner", "Palmetto",
    "Pennsylvanian", "Pere Marquette", "Piedmont", "Saluki", "Silver Meteor",
    "Southwest Chief", "Sunset Limited", "Texas Eagle", "Valley Flyer", "Vermonter",
    "Winter Park Express", "Wolverine",
}
TESTER_RE = re.compile(r"[a-z0-9-]{1,32}")
LAT_RANGE = (24.0, 50.0)
LON_RANGE = (-125.0, -66.0)

_lock = threading.Lock()


def data_dir() -> Path:
    return Path(os.environ.get("AMTRAK_RECEIVER_DIR", "/var/lib/amtrak-receiver"))


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def private_dir(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)
    os.chmod(path, 0o700)


def write_private(path: Path, data: bytes) -> None:
    """Write a file that only this user can read, never half-written.

    It does not change the folder's mode: callers make their own folders with
    private_dir, and add-tester may be pointed at a folder that is not ours."""
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".tmp")
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "wb") as handle:
        handle.write(data)
    os.replace(tmp, path)


# ---- tokens -----------------------------------------------------------------


def tokens_path(base: Path) -> Path:
    return base / "tokens.json"


def load_tokens(base: Path) -> dict:
    try:
        return json.loads(tokens_path(base).read_text())
    except FileNotFoundError:
        return {}


def save_tokens(base: Path, tokens: dict) -> None:
    private_dir(base)
    write_private(tokens_path(base), (json.dumps(tokens, indent=2) + "\n").encode())


def find_tester(tokens: dict, presented: str):
    """The tester name for a live token, else None. Only hashes are stored."""
    digest = sha256(presented.encode())
    found = None
    for stored, entry in tokens.items():
        if hmac.compare_digest(stored, digest) and not entry.get("revoked"):
            found = entry["tester"]
    return found


# ---- validation -------------------------------------------------------------


class Reject(Exception):
    def __init__(self, status: int, reason: str):
        super().__init__(reason)
        self.status = status
        self.reason = reason


def is_number(value) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def in_range(value, low, high) -> bool:
    return is_number(value) and value == value and low <= value <= high


def check_row(row, number: int) -> None:
    def bad(why):
        raise Reject(422, f"row {number}: {why}")

    if not isinstance(row, dict) or set(row) != KEYS:
        bad("wrong fields")
    line = row["line"]
    if line is not None and not (isinstance(line, str) and line in KNOWN_LINES):
        bad("line name")
    lat, lon = row["lat"], row["lon"]
    if (lat is None) != (lon is None):
        bad("position is half missing")
    if lat is not None:
        if not (in_range(lat, *LAT_RANGE) and in_range(lon, *LON_RANGE)):
            bad("position out of range")
        if round(lat, 3) != lat or round(lon, 3) != lon:
            bad("position is more exact than 100 meters")
    if row["tone"] not in TONES:
        bad("tone")
    for key, high in (("loss_pct", 100), ("rtt_ms", 60_000), ("http_s", 600)):
        value = row[key]
        if value is not None and not in_range(value, 0, high):
            bad(key)
    code = row["http_code"]
    if code is not None and not (
        isinstance(code, int) and not isinstance(code, bool) and 100 <= code <= 599
    ):
        bad("http_code")


def no_repeats(pairs):
    keys = [key for key, _ in pairs]
    if len(set(keys)) != len(keys):
        raise ValueError("repeated field")
    return dict(pairs)


def check_body(body: bytes):
    """Parse and check an upload.

    Returns (what to keep, row count, flags worth a person's glance). What is
    kept is the checked rows written out again, never the bytes as sent, so
    nothing the parser ignored can reach the disk."""
    try:
        text = body.decode("utf-8")
    except UnicodeDecodeError:
        raise Reject(422, "not utf-8")
    lines = text.split("\n")
    if lines and lines[-1] == "":
        lines.pop()
    if not lines:
        raise Reject(422, "empty")
    if len(lines) > MAX_ROWS:
        raise Reject(413, "too many rows")
    rows = []
    for number, raw in enumerate(lines, 1):
        try:
            row = json.loads(raw, object_pairs_hook=no_repeats)
        except (ValueError, RecursionError):
            raise Reject(422, f"row {number}: not json, or repeats a field")
        check_row(row, number)
        rows.append(row)
    flags = []
    if len({json.dumps(row, sort_keys=True) for row in rows}) == 1 and len(rows) > 1:
        raise Reject(422, "every row is the same")
    places = {(row["lat"], row["lon"]) for row in rows if row["lat"] is not None}
    if not places:
        flags.append("no_position")
    elif len(places) == 1 and len(rows) > 5:
        flags.append("one_place")
    if len({row["line"] for row in rows}) > 1:
        flags.append("several_lines")
    keep = "".join(
        json.dumps(row, sort_keys=True, separators=(",", ":"), allow_nan=False) + "\n" for row in rows
    ).encode()
    return keep, len(rows), flags


# ---- storage ----------------------------------------------------------------


def index_path(base: Path) -> Path:
    return base / "index.jsonl"


def read_index(base: Path) -> list:
    try:
        text = index_path(base).read_text()
    except FileNotFoundError:
        return []
    entries = []
    for line in text.splitlines():
        try:
            entries.append(json.loads(line))
        except ValueError:
            sys.stderr.write("index.jsonl: skipped a line that does not parse\n")
    return entries


def store_size(base: Path) -> int:
    folder = base / "quarantine"
    if not folder.is_dir():
        return 0
    return sum(p.stat().st_size for p in folder.rglob("*") if p.is_file())


def accept(base: Path, tester: str, body: bytes, now: datetime) -> dict:
    """Check the limits, then keep the upload. One caller at a time."""
    keep, rows, flags = check_body(body)
    digest = sha256(keep)
    with _lock:
        entries = read_index(base)
        if any(entry["sha256"] == digest for entry in entries):
            raise Reject(409, "already received")
        since = (now - timedelta(days=1)).isoformat()
        recent = [entry for entry in entries if entry["at"] >= since]
        if len(recent) >= MAX_UPLOADS_PER_DAY:
            raise Reject(429, "receiver is full for today")
        if sum(1 for entry in recent if entry["tester"] == tester) >= MAX_UPLOADS_PER_TESTER_PER_DAY:
            raise Reject(429, "too many uploads today")
        if store_size(base) + len(keep) > MAX_STORE_BYTES:
            raise Reject(507, "receiver is full")
        stamp = now.strftime("%Y%m%dT%H%M%SZ")
        name = f"{stamp}-{secrets.token_hex(4)}.jsonl"
        private_dir(base / "quarantine")
        private_dir(base / "quarantine" / tester)
        write_private(base / "quarantine" / tester / name, keep)
        entry = {
            "at": now.isoformat(),
            "tester": tester,
            "file": f"{tester}/{name}",
            "rows": rows,
            "bytes": len(keep),
            "sha256": digest,
            "flags": flags,
        }
        line = (json.dumps(entry) + "\n").encode()
        fd = os.open(index_path(base), os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
        with os.fdopen(fd, "ab") as handle:
            handle.write(line)
        return entry


# ---- http -------------------------------------------------------------------


class Handler(BaseHTTPRequestHandler):
    server_version = "amtrak-receiver"
    sys_version = ""
    protocol_version = "HTTP/1.1"
    timeout = SOCKET_TIMEOUT_S

    def reply(self, status: int, payload: dict) -> None:
        body = (json.dumps(payload) + "\n").encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)
        self.close_connection = True

    def fail(self, status: int, reason: str) -> None:
        self.reply(status, {"ok": False, "error": reason})

    def do_GET(self):
        if self.path == PATH_HEALTH:
            self.reply(200, {"ok": True})
        else:
            self.fail(404, "not found")

    def do_POST(self):
        if self.path != PATH_UPLOAD:
            return self.fail(404, "not found")
        base = self.server.base  # pyright: ignore[reportAttributeAccessIssue]
        header = self.headers.get("Authorization", "")
        presented = header[7:] if header.startswith("Bearer ") else ""
        tester = find_tester(load_tokens(base), presented) if presented else None
        if tester is None:
            return self.fail(401, "token not accepted")
        if self.headers.get("Transfer-Encoding"):
            return self.fail(411, "length required")
        declared = self.headers.get("Content-Length", "")
        if not (declared.isascii() and declared.isdigit()):
            return self.fail(411, "length required")
        length = int(declared)
        if length > MAX_BODY:
            return self.fail(413, "too large")
        body = self.rfile.read(length)
        if len(body) != length:
            return self.fail(400, "short body")
        try:
            entry = accept(base, tester, body, datetime.now(timezone.utc))
        except Reject as err:
            self.log_line(tester, err.status)
            return self.fail(err.status, err.reason)
        except Exception as err:  # a bug or a full disk must not drop the connection
            sys.stderr.write(f"{tester} 500 {type(err).__name__}\n")
            return self.fail(500, "receiver error")
        self.log_line(tester, 201, entry["rows"])
        self.reply(201, {"ok": True, "rows": entry["rows"]})

    def log_line(self, tester: str, status: int, rows: int = 0) -> None:
        sys.stderr.write(f"{time.strftime('%FT%TZ', time.gmtime())} {tester} {status} rows={rows}\n")

    def log_message(self, format, *args):
        pass  # no addresses, no paths, no tokens in the service log


class Server(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, address, base: Path):
        super().__init__(address, Handler)
        self.base = base


def serve() -> None:
    base = data_dir()
    private_dir(base)
    port = int(os.environ.get("AMTRAK_RECEIVER_PORT", "8765"))
    server = Server(("127.0.0.1", port), base)
    sys.stderr.write(f"listening on 127.0.0.1:{port}, data in {base}\n")
    server.serve_forever()


# ---- the person's side ------------------------------------------------------


def add_tester(name: str, url: str, outfile: str) -> None:
    if not TESTER_RE.fullmatch(name):
        sys.exit("tester name: 1 to 32 characters of a-z, 0-9, and -")
    if not url.startswith("https://") or re.search(r"[\s\"]", url):
        sys.exit("url must be an https:// address")
    base = data_dir()
    tokens = load_tokens(base)
    if any(entry["tester"] == name and not entry.get("revoked") for entry in tokens.values()):
        sys.exit(f"{name} already has a live token; revoke it first")
    token = secrets.token_urlsafe(32)
    tokens[sha256(token.encode())] = {
        "tester": name,
        "made": datetime.now(timezone.utc).isoformat(),
        "revoked": False,
    }
    save_tokens(base, tokens)
    write_private(Path(outfile), (json.dumps({"url": url, "token": token}, indent=2) + "\n").encode())
    print(f"wrote {outfile} (mode 600) for {name}")


def revoke(name: str) -> None:
    base = data_dir()
    tokens = load_tokens(base)
    hit = 0
    for entry in tokens.values():
        if entry["tester"] == name and not entry.get("revoked"):
            entry["revoked"] = True
            hit += 1
    if hit:
        save_tokens(base, tokens)
    print(f"revoked {hit} token(s) for {name}")


def list_uploads() -> None:
    for entry in read_index(data_dir()):
        flags = ",".join(entry["flags"]) or "-"
        print(f"{entry['at'][:19]}  {entry['tester']:<12} rows={entry['rows']:<5} flags={flags}  {entry['file']}")


def main(argv: list) -> None:
    command = argv[1] if len(argv) > 1 else ""
    if command == "serve" and len(argv) == 2:
        serve()
    elif command == "add-tester" and len(argv) == 5:
        add_tester(argv[2], argv[3], argv[4])
    elif command == "revoke" and len(argv) == 3:
        revoke(argv[2])
    elif command == "list" and len(argv) == 2:
        list_uploads()
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv)
