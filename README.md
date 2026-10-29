# Amtrak Network Monitor

Everyone who rides Amtrak knows the Wi-Fi fails. This program turns that knowledge into measurements. Run it in a terminal during your ride and it logs how good your connection is, and where the train is, every 20 seconds or at a rate you choose, into one private file.

Right now the limits of Amtrak's network are described mostly by anecdote. The aim here is evidence: enough rides, from enough people, to show in plain numbers where the connection fails and how badly, so that riders and the people responsible for the network are arguing from the same data. That is the goal, not where things stand. Today the program records on your own computer, and sharing a ride happens one rider at a time, by hand.

## What it measures

Every 20 seconds, or every 10, 30, or 60 if you change it in settings, the program takes one sample:

- **Ping.** Three pings to `1.1.1.1` (Cloudflare). From them it keeps packet loss and the average round-trip time.
- **Test page.** One request for `http://captive.apple.com/hotspot-detect.html`, given 8 seconds to answer. The page counts as loaded when the reply status is 200. The program checks the status, not what the page says.
- **Place.** Your position to about 100 meters, from the location helper on macOS or the system's location service on Linux. On macOS the town name comes from Apple's geocoder, so Apple sees your position. On Linux the program does not look up a town, and the town line shows your position without a name.
- **Connection.** Which one you told it you were on: Amtrak's free Wi-Fi, or your own, such as a phone hotspot. That label is what lets the two be compared.

The one-word verdict on screen is a rule over those numbers. *Clear* means the page loaded, loss is 5% or less, and the round trip is under 250 ms. *Uneven* means the page loaded, loss is under 40%, and the round trip is under 800 ms. *Poor* means loss is under 80%. Anything worse is *down*, as is a connection with no address. If you turn the ping or the test page off, the rule uses the other one alone, and for the page alone *clear* means it loaded in under half a second. These cutoffs are a judgment, not a standard. They are in `src/model.rs`.

The screen shows the town, the verdict right now, and a row of blocks for the last half hour. One block is 30 seconds. A time under a block, such as `2:20`, is that block.

### What the numbers can and can't say

One laptop in one seat on a shared link is a sample of a ride, not a survey of the network. A hotspot reading depends on your carrier as well as on the train. Read a single ride as one data point.

Any Amtrak train works. The train name is a label, and `--line "Lake Shore Limited"` is one example. If you say you are not on Amtrak, it is still a connection monitor.

## What it keeps, and what it doesn't

A saved row has the time, the town (on macOS), a position rounded to about 100 meters, the measurements above, and which connection you named. It has no street address, no Wi-Fi name, and no IP address, and no setting saves them. The trip file is readable only by your user.

If you say yes at the start, the program also keeps an anonymous copy next to the trip file. It is thinner: the train line, the position, and the measurements. It has no time, no town, and no connection label. It stays on your computer unless you share it.

Rounding is not a guarantee. A route you ride often says something about you even without a name, so open a file and look before you send it. The monitor also talks to two servers of its own while it runs: Cloudflare for the pings and Apple for the test page. They see your connection like any other request. Ping and the test page can each be turned off in settings.

## What you need

For now, only Rust and a terminal. The laptop can be on Amtrak Wi-Fi, tethered to your phone, or connected to a hotspot. Desktop and mobile builds are later work. Today you run this on your laptop, in a terminal.

On macOS, you need the Xcode command line tools as well. Position comes from a small helper, because a terminal program cannot ask macOS for location data. The helper is not a version of the monitor. It only writes your location to a file that the monitor reads. If you're running this on a Mac, leave the helper open for the whole trip.

On Linux, `ip` (iproute2), `ping` (iputils), and `curl` are used for the connection check. Run the monitor in a terminal inside a live, logged-in graphical session. The program asks the location portal, and your system shows its own allow dialog. Over SSH, or with no portal, the town line says it is waiting for a location. `--loc PATH` still reads a location file you already have.

## Build

In this directory:

```bash
cargo build --release
```

On macOS, also build the location helper. It writes `~/Applications/Amtrak Location.app`.

```bash
bash location/build
```

## On the train

On macOS, open **Amtrak Location**, allow Location when macOS asks, and leave it open for the whole trip. Then start the monitor:

```bash
cargo run --release
```

It asks if you are riding Amtrak. If you are, you pick the line, or continue without one, and say whether you are on Amtrak Wi-Fi or your own. It then asks whether to keep the trip on this computer, and tells you how many kB an hour that is. If you keep it, it asks whether to keep the anonymous copy too.

Start it when you board and quit it when you get off. A gap is a gap. The program does not guess where a ride began or ended. `q` quits, and when saving is on, quitting prints the trip file's path.

If the town stops changing on a phone hotspot, the helper has kept the last place it resolved. The town line then gains an age, such as `Albany, NY, 40 min ago`. Leave Amtrak Location open. The connection samples are still recorded, and a stuck place is not filled in as if the train stayed there.

## Settings

`s` opens settings. Up and down move, space changes a row, `s` saves, and `c` cancels. The rows are:

- whether to save this trip
- how often to check the connection
- on Linux, how often to ask for a location (applies the next time you start)

Ping, the test page, and the position can each be turned off. The macOS helper writes about every 15 seconds regardless.

## Where the files go

| System | Folder (trips are in `trips` inside it) |
|---|---|
| macOS | `~/Library/Application Support/amtrak-network-monitor` |
| Linux | `$XDG_STATE_HOME/amtrak-network-monitor`, or `~/.local/state/amtrak-network-monitor` if that is unset |

## Sharing a ride

The program never sends anything on its own. To contribute a ride you have to ask it to, after the ride, once you have seen what would be sent.

Sharing only works if someone running a receiver has given you a `share.json` file. Today that is a small test group, so if you don't have one, keep your files. The `share.json` holds the address to send to and a token that is yours. Put it in the folder from the table above, next to `settings.json`, and keep it private (`chmod 600`). Then, in this directory:

```bash
cargo run --release -- share
```

It takes your newest anonymous copy, tells you how many rows it holds and where it would go, and asks `y/N` before sending. It sends with `curl`. The receiver only takes the line names this program offers, so a ride whose line you typed by hand with `--line` will be refused. Please don't post a trip file anywhere public, such as a GitHub issue.

Like any web request, a send shows your network address to the server that receives it. The file itself does not contain it.

## Contributing

This was built on a Mac and tested on Linux, and it comes with no guarantees. If you find something broken, please try to fix it and open a pull request. For a bigger change, open an issue first so we can talk it through. That goes double for anything that changes what is measured or where the verdict cutoffs sit, because it changes what the numbers mean and makes new rides harder to compare with old ones.

## License

MIT. See `LICENSE`.
