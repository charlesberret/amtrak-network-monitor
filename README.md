# Amtrak Network Monitor

A terminal utility for a train ride. It records the connection the computer is using, and where the train is, for as long as you leave it running. One ride writes one file. Send that file back. It is not uploaded anywhere.

The thing to get right is a simple Unix tool. You run it in a terminal. Linux is a place it should run, and so is a Mac. A macOS app and an iOS app would come later, so someone can start the same record without a terminal.

The train name is a label. Any Amtrak train works. `--line "Lake Shore Limited"` is an example, not a limit.

## What you need

Rust, and a terminal. The laptop can be on Amtrak Wi-Fi or on a phone hotspot. A phone build is later work. Today you run this on a laptop.

On macOS, the Xcode command line tools as well. Position comes from a small helper, because a terminal program cannot ask macOS for Location. Leave that helper open for the whole trip.

On Linux, `ip` (iproute2), `ping` (iputils), and `curl` are the connection check. In a desktop session the program asks the location portal, and the desktop shows its own allow dialog. Over SSH, or with no portal, the town line says it is waiting for a location. `--loc PATH` still reads a location file you already have. Trip files go in `.local/state/amtrak-network-monitor/trips` under your home directory, unless `XDG_STATE_HOME` is set.

## Build

In this directory:

```bash
cargo build --release
```

On macOS, also build the location helper:

```bash
bash location/build
```

That writes `~/Applications/Amtrak Location.app`.

## On the train

On macOS, open **Amtrak Location** and allow Location when macOS asks. Leave that helper running.

Then start the monitor:

```bash
cargo run --release
```

It asks if you are riding Amtrak.

If you are not, it says so, and the title becomes **Network Monitor (Not on Amtrak)**. It is still a connection monitor.

If you are, you pick the line, or continue without one, and say whether you are on free Amtrak Wi-Fi or on your own Wi-Fi. It then asks whether to keep the trip on this computer. The question says how many kB an hour that is, at the rate in settings. A saved trip is what a later ride can use to warn you before a dead zone, and what a comparison of Amtrak Wi-Fi and a personal connection uses. Nothing is uploaded. The warning is not on this screen yet.

If you keep the trip, it asks whether to share an anonymous copy: the train line, the location to about 100 meters, and the connection measurement. There is no public database to send that to yet. A yes keeps the anonymous copy on this computer, next to the trip file.

The screen shows the town, how the connection is doing right now, and a row of blocks for the last half hour. One block is 30 seconds. A time under a block, such as `2:20`, is that block. `s` opens settings. `q` quits.

`s` opens settings. Up and down move. Space changes the row. `s` saves. `c` cancels. The rows are: save this trip, how often to check the connection, and on Linux how often to ask for a location. Ping, the test page, and the position can each be turned off. A saved position stays about 100 meters. A street address, a Wi-Fi name, and an IP address are not saved, and settings has no switch that saves them. The Mac location helper still writes about every 15 seconds. The Linux location rate applies the next time you start.

Quit prints the trip file when saving is on. On a Mac that folder is `Library/Application Support/amtrak-network-monitor/trips` under your home directory. On Linux it is `.local/state/amtrak-network-monitor/trips` under your home directory, unless `XDG_STATE_HOME` is set. Only your user can read that folder. A saved row has the time, the town, a position to about 100 meters, packet loss, round-trip time, and whether a small test page loaded. It also records which of the two connections you named. A street address, a Wi-Fi name, and an IP address are not saved. If you agreed to the anonymous copy, that is a second file next to the trip file. It has the train line, the location, and the connection measurement, and not the town or which connection you used. The file is not uploaded.

## If the town stops changing

On a phone hotspot the location helper can keep the last place it resolved. The town line then gains an age, such as `Albany, NY, 40 min ago`. Leave Amtrak Location open. The connection samples are still recorded. A stuck place is not filled in as if the train stayed there.

## Leave this running

Start it when you board and quit it when you get off. A gap is a gap. The program does not guess the ends of the route.

## Later

A macOS app and an iOS app would start this same record without a terminal.

A later version could do this for in-flight wireless: people on a plane record the connection, and an anonymous copy is what a picture of that network would be built from. There is no receiver for that yet. Nothing in this program uploads a file.

The license is MIT. See `LICENSE`.
