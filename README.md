# Amtrak Network Monitor

Everyone who rides Amtrak knows the Wi-Fi fails, and if you ride enough you might even know where coverage is worst. This program turns that knowledge into measurements. Run it in a terminal during your ride and it measures your connection quality, displays it onscreen for reference, shows where the train is, and logs this data in a private file.

The aim of the Amtrak Network Monitor (AmNetMon?) is evidence: with enough rides logged by enough people, we can show in plain numbers where the connection fails and how badly, so that riders and the people responsible for the network are seeing the same data. That's the goal, not where things stand. If you want this data to reach the level where it can publicly demonstrate the scale of this problem, please contribute. Sharing rides is limited to a small test group for now (see *Sharing a ride* below).

Note: It might sound like I'm taking Amtrak to task for this problem, but the bigger problem is the maintainers of our nationwide mobile network infrastructure have not invested in improving the many dead zones along rural stretches of major train lines. This is an egregious oversight given that train lines have always been the easiest routes for installing long-distance communications infrastructure (literally for nearly two centuries, going back to the fucking telegraph).

## What it measures

The program takes one sample every 20 seconds (but you can change the rate or contents of these samples in the settings):

- **Ping.** Three pings to Cloudflare (`1.1.1.1`). That lets us measure packet loss and the average round-trip time.
- **Test page.** One request for `http://captive.apple.com/hotspot-detect.html`, given 8 seconds to answer. The page counts as loaded when the reply status is 200. The program checks only the status, not the page contents.
- **Place.** Your position within about 100 meters, taken from the location helper on macOS or the system's location service on Linux. On macOS the town name comes from Apple's geocoder, so Apple does see your exact position (the rounding happens later, when your trip is saved). On Linux, the program does not look up a town, just your geo-coordinates, so the app's town line shows your position without a name.
- **Connection.** You're asked this question when you start the tool: are you on Amtrak's free Wi-Fi, or your own service, like a phone tether or hotspot. That label is what allows us to compare the Wi-Fi provided by Amtrak to the general mobile connection quality along the same train route.

The one-word verdict on screen for your _connection quality_ is a rule over those numbers. 
- *Clear* means the page loaded, loss is 5% or less, and the round trip is under 250 ms.
- *Uneven* means the page loaded, loss is under 40%, and the round trip is under 800 ms.
- *Poor* means loss is under 80%.
- Anything worse than that is considered *down*, and so is a connection with no address.

If you turn off either the ping or the test page, the tool uses the other one alone for determining network quality. With only page load time, *clear* means it loaded in under half a second. These cutoffs are a judgment, not based on any standard. They are in `src/model.rs`. You can edit them any way you want and even file a PR if you think you've hit on a better set of cutoffs.

The screen shows the current town, connection quality verdict, and a row of colored blocks displaying the last half hour (or less depending on the window size). One block is 30 seconds. The time written directly under a block, such as `2:20`, is there to help you keep track of the ups and downs in connection speed over the course of your trip (as you become ever more upset and increasingly convinced that someone should do something).

### What the numbers can and can't say

One laptop in one seat on a shared link is a sample of a ride, not a survey of the network. A phone or hotspot reading depends on your carrier as well as the train route. Read a single ride as one round of data gathering that needs more rounds to make a useful dataset.

Any Amtrak train works. The tool even works when you're not on a train. And frankly, you could just spoof the data to mess up this project, but I hope you won't. 

## Data privacy

When you start the program, it asks if you want to keep an anonymous copy next to the trip file in your local file system. It stays on your computer unless you share it. The copy is thinner than the trip file: the train line, the position, and the measurements, with no time, no town, and no connection label. It is what `share` sends.

A saved row has the time, the town (on macOS), a position rounded to about 100 meters, the measurements above, and which connection you named. It has no street address, no Wi-Fi name, and no IP address, and no setting saves them. The trip file is readable only by your user.

Caveat: none of these efforts to anonymize data actually guarantee anonymity (and this is true of any data you produce on any device). A route you ride often says something about you even without a name, so you might want to open the file and look at its contents before you send it. 

The monitor also talks to two servers of its own while it runs: Cloudflare for the pings and Apple for the test page. They see your connection like any other request, but the ping and the test page can be turned off in settings. It's all up to you.

## What you need

For now, only Rust and a terminal. The laptop can be on Amtrak Wi-Fi, tethered to your phone, or connected to a hotspot. Desktop and mobile builds are later work. Today you run this on your laptop, in a terminal.

On macOS, you need the Xcode command line tools as well, plus a small location helper app. A terminal program can't ask macOS for location data, so the helper does it for the monitor. It has its own section, *On a Mac*, below.

On Linux, `ip` (iproute2), `ping` (iputils), and `curl` are used for the connection check. Run the monitor in a terminal inside a live, logged-in graphical session. The program asks the location portal, and your system shows its own allow dialog. Over SSH, or with no portal, the town line says it is waiting for a location. `--loc PATH` still reads a location file you already have.

## Build

In this directory:

```bash
cargo build --release
```

On macOS, build the location helper next (see *On a Mac*, below).

## On a Mac: the location helper

Linux readers can skip this section.

The helper is a small app called **Amtrak Location**. It is not a version of the monitor. All it does is ask macOS for Location and write your position, about every 15 seconds, to a file that the monitor reads.

Build it once. It writes `~/Applications/Amtrak Location.app`:

```bash
bash location/build
```

Before each trip, open **Amtrak Location**, allow Location when macOS asks, and leave it open until you get off. Without it the monitor still records the connection, but it has no position and no town.

Two things to know:

- The helper keeps its own log of every fix, at `~/Library/Application Support/amtrak-network-monitor/location.tsv`. That log is not rounded, and it can include a street or place name from Apple's geocoder. The "no street address" promise above describes the trip file the monitor writes, not this log. It stays on your Mac, and you can delete it whenever you like.
- On a phone hotspot the helper can keep the last place it resolved. The town line then gains an age, such as `Albany, NY, 40 min ago`. The connection samples are still recorded, and a stuck place is not filled in as if the train stayed there.

## On the train

Start the monitor (on a Mac, with the location helper already open):

```bash
cargo run --release
```

It asks if you are riding Amtrak. If you are, you pick the line, or continue without one, and say whether you are on Amtrak Wi-Fi or your own. It then asks whether to keep the trip on this computer, and tells you how many kB an hour that is. If you keep it, it asks whether to keep the anonymous copy too.

Start it when you board and quit it when you get off. A gap is a gap. The program does not guess where a ride began or ended. `q` quits, and when saving is on, quitting prints the trip file's path.

## Settings

`s` opens settings. Up and down move, space changes a row, `s` saves, and `c` cancels. The rows are:

- whether to save this trip
- how often to check the connection
- on Linux, how often to ask for a location (applies the next time you start)

Ping, the test page, and the position can each be turned off.

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

It takes your newest anonymous copy, tells you how many rows it holds and where it would go, and asks `y/N` before sending. It sends with `curl`. The receiver only accepts the line names the picker offers, so a ride with any other line name will be refused. Please don't post a trip file anywhere public, such as a GitHub issue.

Like any web request, a send shows your network address to the server that receives it. The file itself does not contain it.

## Contributing

This was built on a Mac and tested on Linux, but it comes with no guarantees. If you find something broken, please fix it and open a pull request. For a bigger change, open an issue first so we can talk it through. That goes double for anything that changes what is measured or where the verdict cutoffs sit, because it changes what the numbers mean and makes new rides harder to compare with old ones.

## License

MIT, meaning this code is free and open in lots of ways. See `LICENSE`.
