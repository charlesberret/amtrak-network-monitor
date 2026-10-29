//! Readers for the live sampler files.
//!
//! The connection log is local time, no zone.
//! The location log is UTC.
//! A row is skipped when a line is half-written during an append.

use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct LinkRow {
    pub at: DateTime<Local>,
    pub path: String,
    pub dev: String,
    pub ip: Option<String>,
    pub loss_pct: Option<f64>,
    pub rtt_ms: Option<f64>,
    pub http_code: Option<u16>,
    pub http_s: Option<f64>,
    pub is_default: bool,
    /// False when this row was taken with ping turned off.
    pub ping_on: bool,
    /// False when this row was taken with the test page turned off.
    pub http_on: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fix {
    pub at: DateTime<Utc>,
    pub lat: f64,
    pub lon: f64,
    pub acc_m: f64,
    pub place: String,
}

pub fn read_links(path: &Path) -> std::io::Result<Vec<LinkRow>> {
    let text = fs::read_to_string(path)?;
    Ok(parse_links(&text))
}

pub fn read_fixes(path: &Path) -> std::io::Result<Vec<Fix>> {
    let text = fs::read_to_string(path)?;
    Ok(parse_fixes(&text))
}

pub fn parse_links(text: &str) -> Vec<LinkRow> {
    text.lines().filter_map(parse_link_line).collect()
}

pub fn parse_fixes(text: &str) -> Vec<Fix> {
    text.lines().filter_map(parse_fix_line).collect()
}

fn parse_link_line(line: &str) -> Option<LinkRow> {
    if line.is_empty() || line.starts_with("time\t") {
        return None;
    }
    let cols: Vec<&str> = line.split('\t').collect();
    // time, path, dev, ssid, rssi, noise, tx, channel, ip, loss, rtt, then http, then default.
    // curl's code and seconds arrive as a real tab, so a full row has 14 fields.
    // A missing probe stores the two characters '\' and 't' in one field, so 13.
    if cols.len() < 13 {
        return None;
    }
    let at = parse_local(cols[0])?;
    let default_dev = *cols.last()?;
    let http = &cols[11..cols.len() - 1];
    let (http_code, http_s) = match http {
        [code, secs] => (parse_http_code(code), parse_f64(secs)),
        [one] => {
            if let Some((code, secs)) = one.split_once('\t') {
                (parse_http_code(code), parse_f64(secs))
            } else {
                (parse_http_code(one), None)
            }
        }
        _ => (None, None),
    };
    Some(LinkRow {
        at,
        path: cols[1].to_string(),
        dev: cols[2].to_string(),
        ip: nonempty(cols[8]),
        loss_pct: parse_f64(cols[9]),
        rtt_ms: parse_f64(cols[10]),
        http_code,
        http_s,
        is_default: cols[2] == default_dev && default_dev != "-",
        ping_on: true,
        http_on: true,
    })
}

fn parse_fix_line(line: &str) -> Option<Fix> {
    if line.is_empty() || line.starts_with("time\t") {
        return None;
    }
    let cols: Vec<&str> = line.split('\t').collect();
    if cols.len() < 6 {
        return None;
    }
    let at = DateTime::parse_from_rfc3339(cols[0])
        .ok()?
        .with_timezone(&Utc);
    Some(Fix {
        at,
        lat: cols[1].parse().ok()?,
        lon: cols[2].parse().ok()?,
        acc_m: cols[3].parse().ok()?,
        place: cols[5].trim().to_string(),
    })
}

fn parse_local(s: &str) -> Option<DateTime<Local>> {
    let naive = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok()?;
    Local.from_local_datetime(&naive).single()
}

fn nonempty(s: &str) -> Option<String> {
    if s.is_empty() || s == "-" {
        None
    } else {
        Some(s.to_string())
    }
}

fn parse_f64(s: &str) -> Option<f64> {
    if s.is_empty() || s == "-" {
        return None;
    }
    s.parse().ok()
}

/// `000` is the logger's way of saying the fetch failed. It is not an HTTP status.
fn parse_http_code(s: &str) -> Option<u16> {
    let n: u16 = s.parse().ok()?;
    if n == 0 {
        None
    } else {
        Some(n)
    }
}

/// The row on the machine's default route at the newest timestamp.
pub fn default_row(rows: &[LinkRow]) -> Option<&LinkRow> {
    let newest = rows.iter().map(|r| r.at).max()?;
    rows.iter()
        .rev()
        .find(|r| r.at == newest && r.is_default)
        .or_else(|| rows.iter().rev().find(|r| r.at == newest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_usb_default_and_wifi_without_an_address() {
        let text = "\
time\tlink\tdev\tssid\trssi_dbm\tnoise_dbm\ttx_mbps\tchannel\tip\tping_loss_pct\tping_avg_ms\thttp_code\thttp_s\tdefault
2026-10-21 13:57:43\twifi\ten0\t-\t0\t0\t0\t1\t-\t-\t-\t-\\t-\ten7
2026-10-21 13:57:43\tiphone-usb\ten7\t-\t-\t-\t-\t-\t172.20.10.3\t0.0\t49.963\t200\t0.089702\ten7
";
        let rows = parse_links(text);
        assert_eq!(rows.len(), 2);
        assert!(!rows[0].is_default);
        assert!(rows[0].ip.is_none());
        assert!(rows[0].http_code.is_none());
        assert!(rows[1].is_default);
        assert_eq!(rows[1].ip.as_deref(), Some("172.20.10.3"));
        assert_eq!(rows[1].http_code, Some(200));
        assert_eq!(rows[1].loss_pct, Some(0.0));
        let chosen = default_row(&rows).unwrap();
        assert_eq!(chosen.dev, "en7");
    }

    #[test]
    fn parses_a_fix() {
        let text = "time\tlat\tlon\tacc_m\tspeed_mps\tplace
2026-10-21T17:58:00Z\t40.588751\t-77.579547\t46\t-\t150 Helen St, Lewistown, PA
";
        let fixes = parse_fixes(text);
        assert_eq!(fixes.len(), 1);
        assert!((fixes[0].lat - 40.588751).abs() < 1e-6);
        assert_eq!(fixes[0].place, "150 Helen St, Lewistown, PA");
    }
}
