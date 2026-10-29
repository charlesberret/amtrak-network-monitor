# Receiver

A small service that takes the anonymous copies riders choose to send with `amtrak-network-monitor share`. It is Python 3 standard library only, and it listens on `127.0.0.1` behind a web server that does HTTPS.

It is for a handful of testers, not the public. Each tester has their own token. Nothing it receives is merged into anything: every accepted upload is one file in `quarantine/<tester>/`, kept until a person has looked at it.

## What it accepts

`POST /amtrak/upload` with the `-share.jsonl` file as the body and `Authorization: Bearer <token>`.

Every row must have exactly these fields: `line`, `lat`, `lon`, `tone`, `loss_pct`, `rtt_ms`, `http_code`, `http_s`. A position must be rounded to three decimals (about 100 meters) and inside North America. The line must be empty or exactly one of the names the monitor offers (the table in `src/lines.rs`, copied into `KNOWN_LINES`), because a line is free text on the rider's side and would otherwise be a place to put an address. A repeated field, any other field, or any row that fails refuses the whole file.

What is kept is each checked row written out again, not the bytes as sent, so nothing the parser ignored reaches the disk.

Limits: 1 MB and 10,000 rows per upload, 10 uploads per tester per day, 100 per day in all, and 200 MB stored. A repeat of the same rows is refused, however the spacing differs. A file whose rows are all identical is refused. A file with no position, or one place only, is kept and flagged in `index.jsonl` for a person to glance at.

## What it keeps

The file, its size, the tester name, the time, and a hash. It does not log addresses, tokens, or request lines. The web server in front of it sees the sender's address like any web request, and what it logs is up to that server's configuration.

Tokens are stored only as hashes. `share.json` for a tester is written once, mode 600, and holds the address and the token.

## Commands

```
receiver.py serve                          run it
receiver.py add-tester NAME URL OUTFILE    make a token; write that tester's share.json
receiver.py revoke NAME                    stop accepting a tester
receiver.py list                           show what has arrived
```

`AMTRAK_RECEIVER_DIR` (default `/var/lib/amtrak-receiver`) and `AMTRAK_RECEIVER_PORT` (default `8765`) set where it keeps files and where it listens. Run the commands as the same user as the service, so the files keep one owner.

## Files here

- `receiver.py` the service.
- `test_receiver.py` its tests. From this folder: `python3 -m unittest -v test_receiver`.
- `amtrak-receiver.service` a systemd unit with the service locked down to its own folder.
- `apache-amtrak.conf` the lines a web server needs to put it behind HTTPS.
