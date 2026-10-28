# Amtrak Network Monitor

A Mac program for a train ride. It records the connection the computer is using, and where the train is, for as long as you leave it running. One ride writes one file. Send that file back. It is not uploaded anywhere.

The train name is a label. Any Amtrak train works. `--line "Lake Shore Limited"` is an example, not a limit.

## What you need

A Mac on the train, with the Xcode command line tools and Rust. The laptop can be on Amtrak Wi-Fi or on a phone hotspot. The location helper has to stay open for the whole trip. This program does not run on a phone.

## Build

In this directory:

```bash
cargo build --release
bash location/build
```

The location helper is written to `~/Applications/Amtrak Location.app`.

## On the train

Open **Amtrak Location** and allow Location when macOS asks. Leave that app running.

Then start the monitor with the train you are on:

```bash
cargo run --release -- --line "Lake Shore Limited"
```

The screen shows the town, how the connection is doing right now, and a row of blocks for the last half hour. One block is 30 seconds. A time under a block, such as `2:20`, is that block. `q` quits.

Quit prints the trip file. It also appears in the footer as `recording …jsonl`. The file is under:

`~/Library/Application Support/amtrak-network-monitor/trips/`

Send that one file. It contains the time, coordinates, a place name, packet loss, round-trip time, and whether a small test page loaded. It does not contain a Wi-Fi name or an IP address.

## If the town stops changing

On a phone hotspot the Mac can keep the last place it resolved. The town line then gains an age, such as `Albany, NY, 40 min ago`. Leave Amtrak Location open. The connection samples are still recorded. A stuck place is not filled in as if the train stayed there.

## Leave this running

Start it when you board and quit it when you get off. A gap is a gap. The program does not guess the ends of the route.
