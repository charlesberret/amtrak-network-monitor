//! Measure the default route when the shell sampler is not already running.
//! This never appends to the sampler's TSV. While that process is alive,
//! the utility follows the file and does not probe.

use crate::feed::LinkRow;
use chrono::Local;
use std::process::Command;

pub struct ProbePlan {
    pub ping: bool,
    pub http: bool,
}

pub fn measure_default(plan: ProbePlan) -> Option<LinkRow> {
    let dev = default_dev()?;
    let ip = interface_addr(&dev);
    let wireless = is_wireless(&dev);
    let (loss_pct, rtt_ms) = if plan.ping && ip.is_some() {
        ping_dev(&dev)
    } else {
        (None, None)
    };
    let (http_code, http_s) = if plan.http && ip.is_some() {
        http_dev(&dev)
    } else {
        (None, None)
    };
    Some(LinkRow {
        at: Local::now(),
        path: link_kind(&dev, ip.as_deref(), wireless),
        dev,
        ip,
        loss_pct,
        rtt_ms,
        http_code,
        http_s,
        is_default: true,
        ping_on: plan.ping,
        http_on: plan.http,
    })
}

/// `wireless` comes from sysfs on Linux. macOS has no sysfs, so the `en*` names stay.
pub fn link_kind(dev: &str, ip: Option<&str>, wireless: bool) -> String {
    if wireless {
        return "wifi".to_string();
    }
    let hotspot = ip.is_some_and(|ip| ip.starts_with("172.20.10."));
    if hotspot && dev != "en0" {
        "iphone-usb".to_string()
    } else if dev == "en0" || dev.starts_with("en") {
        "wifi".to_string()
    } else {
        "other".to_string()
    }
}

fn is_wireless(dev: &str) -> bool {
    let base = std::path::Path::new("/sys/class/net").join(dev);
    base.join("wireless").exists() || base.join("phy80211").exists()
}

/// macOS `route` prints `interface:`. Linux `ip route` prints `dev wlan0`.
pub fn parse_default_dev(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("interface:") {
            let dev = rest.trim();
            if !dev.is_empty() {
                return Some(dev.to_string());
            }
        }
    }
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        while let Some(tok) = parts.next() {
            if tok == "dev" {
                let dev = parts.next()?.trim();
                if !dev.is_empty() {
                    return Some(dev.to_string());
                }
            }
        }
    }
    None
}

/// `ip -o addr` prints `inet 192.0.2.4/24`. A bare address is the macOS `ipconfig` line.
pub fn parse_iface_addr(text: &str) -> Option<String> {
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        while let Some(tok) = parts.next() {
            if tok == "inet" {
                let addr = parts.next()?.split('/').next()?.trim();
                if !addr.is_empty() {
                    return Some(addr.to_string());
                }
            }
        }
    }
    let bare = text.trim();
    if bare.is_empty() || bare.contains(' ') {
        None
    } else {
        Some(bare.to_string())
    }
}

fn default_dev() -> Option<String> {
    if let Some(text) = command_stdout("ip", &["-4", "route", "show", "default"]) {
        if let Some(dev) = parse_default_dev(&text) {
            return Some(dev);
        }
    }
    let text = command_stdout("route", &["-n", "get", "-inet", "default"])?;
    parse_default_dev(&text)
}

fn interface_addr(dev: &str) -> Option<String> {
    if let Some(text) = command_stdout("ip", &["-4", "-o", "addr", "show", "dev", dev]) {
        if let Some(ip) = parse_iface_addr(&text) {
            return Some(ip);
        }
    }
    let text = command_stdout("ipconfig", &["getifaddr", dev])?;
    parse_iface_addr(&text)
}

/// Linux `-W` is seconds. macOS `-W` is milliseconds. The numbers must not be shared.
pub fn ping_args(os: &str, dev: &str) -> Vec<String> {
    let args: &[&str] = if os == "linux" {
        &["-I", dev, "-c", "3", "-W", "1", "-n", "1.1.1.1"]
    } else {
        &["-b", dev, "-c", "3", "-W", "1000", "1.1.1.1"]
    };
    args.iter().map(|arg| (*arg).to_string()).collect()
}

fn ping_dev(dev: &str) -> (Option<f64>, Option<f64>) {
    let args = ping_args(std::env::consts::OS, dev);
    let out = Command::new("ping").args(&args).output();
    let text = match out {
        Ok(out) => {
            String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr)
        }
        Err(_) => return (None, None),
    };
    (parse_loss(&text), parse_rtt(&text))
}

fn http_dev(dev: &str) -> (Option<u16>, Option<f64>) {
    let out = Command::new("curl")
        .args([
            "--interface",
            dev,
            "-4",
            "-sS",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code} %{time_total}",
            "--max-time",
            "8",
            "http://captive.apple.com/hotspot-detect.html",
        ])
        .output();
    let text = match out {
        Ok(out) => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        Err(_) => return (None, None),
    };
    let mut parts = text.split_whitespace();
    let code: u16 = parts.next().unwrap_or("0").parse().unwrap_or(0);
    let secs: Option<f64> = parts.next().and_then(|s| s.parse().ok());
    (if code == 0 { None } else { Some(code) }, secs)
}

pub fn parse_loss(text: &str) -> Option<f64> {
    for line in text.lines() {
        if let Some(idx) = line.find("% packet loss") {
            let head = line[..idx].trim();
            let num = head.split_whitespace().last()?;
            return num.parse().ok();
        }
    }
    None
}

pub fn parse_rtt(text: &str) -> Option<f64> {
    for line in text.lines() {
        let line = line.trim();
        let rest = line
            .strip_prefix("round-trip ")
            .or_else(|| line.strip_prefix("rtt "));
        if let Some(rest) = rest {
            let avg = rest.split('=').nth(1)?.trim().split('/').nth(1)?;
            return avg.trim().parse().ok();
        }
    }
    None
}

fn command_stdout(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    if !out.status.success() && out.stdout.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

/// The sampler this machine may already be running. Matched by the program
/// name, so a published build does not carry a home directory.
pub fn is_sampler_arg(arg: &str) -> bool {
    std::path::Path::new(arg)
        .file_name()
        .and_then(|name| name.to_str())
        == Some("link-quality")
}

pub fn is_location_helper_arg(arg: &str) -> bool {
    match std::path::Path::new(arg)
        .file_name()
        .and_then(|name| name.to_str())
    {
        Some("LocSample" | "AmtrakLocation") => true,
        _ => false,
    }
}

pub fn is_legacy_location_arg(arg: &str) -> bool {
    std::path::Path::new(arg)
        .file_name()
        .and_then(|name| name.to_str())
        == Some("LocSample")
}

pub fn sampler_pid() -> Option<u32> {
    process_with(is_sampler_arg)
}

/// The older helper writes `~/.cache/link-quality/loc-last.tsv`.
/// `AmtrakLocation` writes the shared app's own file and is not this.
pub fn legacy_location_pid() -> Option<u32> {
    process_with(is_legacy_location_arg)
}

fn process_with(hit: fn(&str) -> bool) -> Option<u32> {
    let out = std::process::Command::new("ps")
        .args(["-ax", "-o", "pid=,command="])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(pid) = parts.next().and_then(|token| token.parse().ok()) else {
            continue;
        };
        if parts.any(hit) {
            return Some(pid);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ping_summary() {
        let text = "\
3 packets transmitted, 3 packets received, 0.0% packet loss
round-trip min/avg/max/stddev = 39.181/49.963/78.854/16.2 ms
";
        assert_eq!(parse_loss(text), Some(0.0));
        assert_eq!(parse_rtt(text), Some(49.963));
    }

    #[test]
    fn linux_ping_summary_uses_rtt() {
        let text = "\
3 packets transmitted, 3 received, 0% packet loss, time 2002ms
rtt min/avg/max/mdev = 11.054/12.438/14.102/1.280 ms
";
        assert_eq!(parse_loss(text), Some(0.0));
        assert_eq!(parse_rtt(text), Some(12.438));
    }

    #[test]
    fn linux_route_and_address_parse() {
        let route = "default via 10.0.0.1 dev wlp3s0 proto dhcp metric 600\n";
        assert_eq!(parse_default_dev(route).as_deref(), Some("wlp3s0"));
        let addr = "3: wlp3s0    inet 192.0.2.8/24 brd 192.0.2.255 scope global dynamic wlp3s0\n";
        assert_eq!(parse_iface_addr(addr).as_deref(), Some("192.0.2.8"));
        assert_eq!(
            parse_default_dev("   interface: en0\n").as_deref(),
            Some("en0")
        );
        assert_eq!(parse_iface_addr("10.1.2.3\n").as_deref(), Some("10.1.2.3"));
    }

    #[test]
    fn ping_timeout_is_one_second_on_linux_and_milliseconds_on_macos() {
        assert_eq!(
            ping_args("linux", "wlp3s0"),
            ["-I", "wlp3s0", "-c", "3", "-W", "1", "-n", "1.1.1.1"]
        );
        assert_eq!(
            ping_args("macos", "en0"),
            ["-b", "en0", "-c", "3", "-W", "1000", "1.1.1.1"]
        );
    }

    #[test]
    fn sampler_name_is_exact() {
        assert!(is_sampler_arg("link-quality"));
        assert!(is_sampler_arg("/Users/someone/bin/link-quality"));
        assert!(!is_sampler_arg("link-quality-summary"));
        assert!(!is_sampler_arg("/Users/someone/bin/link-quality-summary"));
        assert!(is_location_helper_arg("LocSample"));
        assert!(is_location_helper_arg(
            "/tmp/LocSample.app/Contents/MacOS/LocSample"
        ));
        assert!(is_location_helper_arg("AmtrakLocation"));
        assert!(!is_location_helper_arg("amtrak-network-monitor"));
    }

    #[test]
    fn hotspot_on_wifi_stays_wifi() {
        assert_eq!(link_kind("en0", Some("172.20.10.4"), false), "wifi");
        assert_eq!(link_kind("en7", Some("172.20.10.4"), false), "iphone-usb");
        assert_eq!(link_kind("en0", Some("192.168.1.20"), false), "wifi");
        assert_eq!(link_kind("bridge0", None, false), "other");
        assert_eq!(link_kind("wlp3s0", Some("172.20.10.4"), true), "wifi");
        assert_eq!(link_kind("enx00", Some("172.20.10.4"), false), "iphone-usb");
        assert_eq!(link_kind("wlp3s0", Some("192.0.2.8"), true), "wifi");
    }
}
