//! Measure the default route when the shell sampler is not already running.
//! This never appends to the sampler's TSV. While that process is alive,
//! the utility follows the file and does not probe.

use crate::feed::LinkRow;
use chrono::Local;
use std::process::Command;

pub fn measure_default() -> Option<LinkRow> {
    let dev = default_dev()?;
    let ip = interface_addr(&dev);
    let (loss_pct, rtt_ms, http_code, http_s) = if ip.is_some() {
        let (loss, rtt) = ping_dev(&dev);
        let (code, secs) = http_dev(&dev);
        (loss, rtt, code, secs)
    } else {
        (None, None, None, None)
    };
    Some(LinkRow {
        at: Local::now(),
        path: link_kind(&dev, ip.as_deref()),
        dev,
        ip,
        loss_pct,
        rtt_ms,
        http_code,
        http_s,
        is_default: true,
    })
}

fn link_kind(dev: &str, ip: Option<&str>) -> String {
    let hotspot = ip.is_some_and(|ip| ip.starts_with("172.20.10."));
    if hotspot && dev != "en0" {
        "iphone-usb".to_string()
    } else if dev == "en0" || dev.starts_with("en") {
        "wifi".to_string()
    } else {
        "other".to_string()
    }
}

fn default_dev() -> Option<String> {
    let out = Command::new("route")
        .args(["-n", "get", "-inet", "default"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("interface:") {
            let dev = rest.trim();
            if !dev.is_empty() {
                return Some(dev.to_string());
            }
        }
    }
    None
}

fn interface_addr(dev: &str) -> Option<String> {
    let out = Command::new("ipconfig")
        .args(["getifaddr", dev])
        .output()
        .ok()?;
    let ip = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if ip.is_empty() {
        None
    } else {
        Some(ip)
    }
}

fn ping_dev(dev: &str) -> (Option<f64>, Option<f64>) {
    let out = Command::new("ping")
        .args(["-b", dev, "-c", "3", "-W", "1000", "1.1.1.1"])
        .output();
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
        if let Some(rest) = line.strip_prefix("round-trip ") {
            let avg = rest.split('=').nth(1)?.trim().split('/').nth(1)?;
            return avg.trim().parse().ok();
        }
    }
    None
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

pub fn location_helper_pid() -> Option<u32> {
    process_with(is_location_helper_arg)
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
        assert_eq!(link_kind("en0", Some("172.20.10.4")), "wifi");
        assert_eq!(link_kind("en7", Some("172.20.10.4")), "iphone-usb");
        assert_eq!(link_kind("en0", Some("192.168.1.20")), "wifi");
        assert_eq!(link_kind("bridge0", None), "other");
    }
}
