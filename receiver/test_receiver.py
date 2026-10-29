"""Tests for receiver.py. Run from the receiver folder: python3 -m unittest -v test_receiver"""
import json
import os
import stat
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
from pathlib import Path

import receiver


def row(**over):
    base = {
        "line": "Pennsylvanian",
        "lat": 40.262,
        "lon": -76.884,
        "tone": "clear",
        "loss_pct": 0.0,
        "rtt_ms": 54.2,
        "http_code": 200,
        "http_s": 0.22,
    }
    base.update(over)
    return base


def body(*rows):
    return "".join(json.dumps(r) + "\n" for r in rows).encode()


def canonical(data):
    """What the receiver keeps: each row written out again, compact, keys sorted."""
    rows = [json.loads(line) for line in data.decode().splitlines()]
    return "".join(json.dumps(r, sort_keys=True, separators=(",", ":")) + "\n" for r in rows).encode()


def ride(n=8, line="Pennsylvanian"):
    return body(*[row(line=line, lat=40.0 + i / 1000, rtt_ms=50 + i) for i in range(n)])


class Receiver(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.base = Path(self.tmp.name) / "data"
        os.environ["AMTRAK_RECEIVER_DIR"] = str(self.base)
        receiver.private_dir(self.base)
        self.config = Path(self.tmp.name) / "share.json"
        receiver.add_tester("friend", "https://example.test/amtrak/upload", str(self.config))
        self.token = json.loads(self.config.read_text())["token"]
        self.server = receiver.Server(("127.0.0.1", 0), self.base)
        self.port = self.server.server_address[1]
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.tmp.cleanup()

    def post(self, data, token="__own__", path="/amtrak/upload"):
        token = self.token if token == "__own__" else token
        req = urllib.request.Request(f"http://127.0.0.1:{self.port}{path}", data=data, method="POST")
        if token is not None:
            req.add_header("Authorization", f"Bearer {token}")
        try:
            with urllib.request.urlopen(req) as resp:
                return resp.status, json.loads(resp.read())
        except urllib.error.HTTPError as err:
            return err.code, json.loads(err.read())

    def test_a_good_ride_is_kept_in_quarantine(self):
        status, reply = self.post(ride())
        self.assertEqual((status, reply), (201, {"ok": True, "rows": 8}))
        files = list((self.base / "quarantine" / "friend").glob("*.jsonl"))
        self.assertEqual(len(files), 1)
        self.assertEqual(files[0].read_bytes(), canonical(ride()))
        self.assertEqual(stat.S_IMODE(files[0].stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(files[0].parent.stat().st_mode), 0o700)
        index = receiver.read_index(self.base)
        self.assertEqual(index[0]["tester"], "friend")
        self.assertEqual(index[0]["rows"], 8)

    def test_no_token_or_a_wrong_token_is_refused(self):
        self.assertEqual(self.post(ride(), token=None)[0], 401)
        self.assertEqual(self.post(ride(), token="nope")[0], 401)
        self.assertFalse((self.base / "quarantine").exists())

    def test_a_revoked_token_is_refused(self):
        receiver.revoke("friend")
        self.assertEqual(self.post(ride())[0], 401)

    def test_tokens_are_stored_as_hashes(self):
        stored = (self.base / "tokens.json").read_text()
        self.assertNotIn(self.token, stored)
        self.assertEqual(stat.S_IMODE((self.base / "tokens.json").stat().st_mode), 0o600)

    def test_share_json_is_private_and_holds_the_token(self):
        self.assertEqual(stat.S_IMODE(self.config.stat().st_mode), 0o600)
        self.assertEqual(json.loads(self.config.read_text())["url"], "https://example.test/amtrak/upload")

    def test_the_same_ride_twice_is_refused(self):
        self.assertEqual(self.post(ride())[0], 201)
        self.assertEqual(self.post(ride())[0], 409)

    def test_a_position_finer_than_100_meters_is_refused(self):
        status, reply = self.post(body(row(lat=40.26212)))
        self.assertEqual(status, 422)
        self.assertIn("100 meters", reply["error"])

    def test_extra_fields_are_refused(self):
        for extra in ("place", "path", "at", "ip", "ssid"):
            r = row()
            r[extra] = "Harrisburg"
            self.assertEqual(self.post(body(r))[0], 422, extra)

    def test_a_missing_field_is_refused(self):
        r = row()
        del r["tone"]
        self.assertEqual(self.post(body(r))[0], 422)

    def test_a_line_name_must_be_one_the_monitor_offers(self):
        for bad in ("12 Elm St Apt 4", "Pennsylvanian\n", "pennsylvanian", "", ["Acela"], {"a": 1}, 7):
            self.assertEqual(self.post(body(row(line=bad)))[0], 422, bad)
        self.assertEqual(self.post(body(row(line="Lake Shore Limited", rtt_ms=9)))[0], 201)

    def test_every_line_the_monitor_offers_is_accepted(self):
        import re
        names = re.findall(r'name: "([^"]+)"', (Path(__file__).parent.parent / "src" / "lines.rs").read_text())
        self.assertGreater(len(names), 30)
        self.assertEqual(set(names), receiver.KNOWN_LINES)

    def test_a_repeated_field_cannot_smuggle_a_value_past_the_checks(self):
        text = json.dumps(row(rtt_ms=51.0))
        dup = text.replace('"line": "Pennsylvanian"', '"line": "12 Elm St Apt 4 Harrisburg PA", "line": "Pennsylvanian"')
        self.assertNotEqual(dup, text)
        self.assertEqual(self.post((dup + "\n").encode())[0], 422)
        dup_lat = text.replace('"lat": 40.262', '"lat": 40.2621234, "lat": 40.262')
        self.assertNotEqual(dup_lat, text)
        self.assertEqual(self.post((dup_lat + "\n").encode())[0], 422)
        self.assertFalse((self.base / "quarantine").exists())

    def test_only_the_checked_rows_are_kept_not_the_bytes_as_sent(self):
        long_digits = json.dumps(row(rtt_ms=52.0)).replace("40.262", "40.26200000000000000001234567")
        padded = (long_digits + "   \n").encode()
        self.assertEqual(self.post(padded)[0], 201)
        kept = next((self.base / "quarantine" / "friend").glob("*.jsonl")).read_text()
        self.assertNotIn("0000000000", kept)
        self.assertNotIn("   ", kept)
        self.assertEqual(json.loads(kept)["lat"], 40.262)

    def test_the_same_ride_with_different_spacing_is_still_a_repeat(self):
        text = ride()
        self.assertEqual(self.post(text)[0], 201)
        respaced = text.replace(b", ", b",  ").replace(b": ", b":  ")
        self.assertNotEqual(respaced, text)
        self.assertEqual(self.post(respaced)[0], 409)

    def test_a_torn_index_line_does_not_stop_uploads(self):
        self.assertEqual(self.post(ride(6))[0], 201)
        with open(self.base / "index.jsonl", "ab") as handle:
            handle.write(b'{"at": "2026-10-29T1')
        self.assertEqual(self.post(ride(7))[0], 201)

    def test_an_unexpected_error_is_a_500_not_a_dropped_connection(self):
        original = receiver.store_size
        receiver.store_size = lambda base: 1 / 0
        try:
            status, reply = self.post(ride(5))
        finally:
            receiver.store_size = original
        self.assertEqual((status, reply["error"]), (500, "receiver error"))
        self.assertEqual(self.post(ride(6))[0], 201)

    def test_add_tester_leaves_the_output_folder_alone(self):
        folder = Path(self.tmp.name) / "shared"
        folder.mkdir()
        os.chmod(folder, 0o755)
        receiver.add_tester("other", "https://example.test/amtrak/upload", str(folder / "o.json"))
        self.assertEqual(stat.S_IMODE(folder.stat().st_mode), 0o755)
        self.assertEqual(stat.S_IMODE((folder / "o.json").stat().st_mode), 0o600)

    def test_a_content_length_that_is_not_plain_digits_is_refused(self):
        import http.client

        for odd in ("+5", "1_000", "-1", "5.0"):
            conn = http.client.HTTPConnection("127.0.0.1", self.port)
            conn.putrequest("POST", "/amtrak/upload")
            conn.putheader("Authorization", f"Bearer {self.token}")
            conn.putheader("Content-Length", odd)
            conn.endheaders()
            self.assertEqual(conn.getresponse().status, 411, odd)

    def test_nulls_are_fine_where_the_monitor_writes_them(self):
        r = row(lat=None, lon=None, loss_pct=None, rtt_ms=None, http_code=None, http_s=None, line=None, tone="down")
        status, reply = self.post(body(r, row(tone="down", lat=None, lon=None, rtt_ms=None)))
        self.assertEqual(status, 201, reply)

    def test_numbers_must_be_plausible(self):
        for bad in (
            row(lat=80.0),
            row(lon=10.0),
            row(lat=40.262, lon=None),
            row(loss_pct=101),
            row(loss_pct=-1),
            row(rtt_ms=1e9),
            row(http_code=99),
            row(http_code=True),
            row(tone="great"),
            row(loss_pct="0"),
        ):
            self.assertEqual(self.post(body(bad))[0], 422, bad)

    def test_garbage_is_refused(self):
        for data in (b"", b"\n", b"not json\n", b"[1,2]\n", b"\xff\xfe\n", ride() + b"\n\n"):
            self.assertEqual(self.post(data)[0], 422, data)

    def test_rows_that_are_all_the_same_look_made_up(self):
        status, reply = self.post(body(*[row()] * 10))
        self.assertEqual(status, 422)
        self.assertIn("same", reply["error"])

    def test_oversize_and_too_many_rows_are_refused(self):
        import http.client

        conn = http.client.HTTPConnection("127.0.0.1", self.port)
        conn.putrequest("POST", "/amtrak/upload")
        conn.putheader("Authorization", f"Bearer {self.token}")
        conn.putheader("Content-Length", str(receiver.MAX_BODY + 1))
        conn.endheaders()
        self.assertEqual(conn.getresponse().status, 413)
        old = receiver.MAX_ROWS
        receiver.MAX_ROWS = 3
        try:
            self.assertEqual(self.post(ride(5))[0], 413)
        finally:
            receiver.MAX_ROWS = old

    def test_flags_mark_what_a_person_should_glance_at(self):
        stuck = body(*[row(rtt_ms=50 + i) for i in range(8)])
        self.assertEqual(self.post(stuck)[0], 201)
        self.assertEqual(receiver.read_index(self.base)[-1]["flags"], ["one_place"])
        self.assertEqual(self.post(body(*[row(lat=None, lon=None, rtt_ms=60 + i) for i in range(3)]))[0], 201)
        self.assertEqual(receiver.read_index(self.base)[-1]["flags"], ["no_position"])

    def test_one_tester_is_rate_limited(self):
        old = receiver.MAX_UPLOADS_PER_TESTER_PER_DAY
        receiver.MAX_UPLOADS_PER_TESTER_PER_DAY = 2
        try:
            self.assertEqual(self.post(ride(6))[0], 201)
            self.assertEqual(self.post(ride(7))[0], 201)
            self.assertEqual(self.post(ride(8))[0], 429)
        finally:
            receiver.MAX_UPLOADS_PER_TESTER_PER_DAY = old

    def test_the_store_has_a_ceiling(self):
        old = receiver.MAX_STORE_BYTES
        receiver.MAX_STORE_BYTES = len(ride(6)) + 10
        try:
            self.assertEqual(self.post(ride(6))[0], 201)
            self.assertEqual(self.post(ride(7))[0], 507)
        finally:
            receiver.MAX_STORE_BYTES = old

    def test_only_the_upload_path_answers(self):
        self.assertEqual(self.post(ride(), path="/other")[0], 404)
        with urllib.request.urlopen(f"http://127.0.0.1:{self.port}/amtrak/health") as resp:
            self.assertEqual(json.loads(resp.read()), {"ok": True})

    def test_a_body_without_a_length_is_refused(self):
        import http.client

        conn = http.client.HTTPConnection("127.0.0.1", self.port)
        conn.putrequest("POST", "/amtrak/upload")
        conn.putheader("Authorization", f"Bearer {self.token}")
        conn.putheader("Transfer-Encoding", "chunked")
        conn.endheaders()
        self.assertEqual(conn.getresponse().status, 411)

    def test_a_tester_name_cannot_walk_out_of_the_folder(self):
        for name in ("../x", "a/b", "A", "", "x" * 33, "friend\n"):
            with self.assertRaises(SystemExit):
                receiver.add_tester(name, "https://example.test/amtrak/upload", str(Path(self.tmp.name) / "o.json"))


if __name__ == "__main__":
    unittest.main()
