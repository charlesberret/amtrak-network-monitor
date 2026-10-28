//! What the screen shows. The words describe the latest sample only.
//! They are not a rule about a stretch of track, and they do not decide
//! where a route begins or ends.

use crate::feed::{Fix, LinkRow};
use chrono::{DateTime, Local, Timelike, Utc};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Clear,
    Uneven,
    Poor,
    Down,
    NoSample,
}

impl Tone {
    pub fn word(self) -> &'static str {
        match self {
            Tone::Clear => "clear",
            Tone::Uneven => "uneven",
            Tone::Poor => "poor",
            Tone::Down => "down",
            Tone::NoSample => "no recent sample",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub at: DateTime<Local>,
    pub line: Option<String>,
    pub place: String,
    pub position: Option<(f64, f64)>,
    pub accuracy_m: Option<f64>,
    pub motion: Option<String>,
    pub tone: Tone,
    pub pace: String,
    pub page: String,
    pub path: String,
    pub is_default: bool,
    pub history: Vec<MinuteLight>,
    pub source: String,
    pub recording: String,
    pub sample_age: Option<Duration>,
    pub fix_age: Option<Duration>,
}

pub fn tone_of(row: &LinkRow, age: Duration) -> Tone {
    if age > Duration::from_secs(90) {
        return Tone::NoSample;
    }
    if row.ip.is_none() {
        return Tone::Down;
    }
    let loss = row.loss_pct.unwrap_or(100.0);
    let rtt = row.rtt_ms.unwrap_or(10_000.0);
    let page_ok = row.http_code == Some(200);
    if page_ok && loss <= 5.0 && rtt < 250.0 {
        Tone::Clear
    } else if page_ok && loss < 40.0 && rtt < 800.0 {
        Tone::Uneven
    } else if loss < 80.0 {
        Tone::Poor
    } else {
        Tone::Down
    }
}

/// Round trip and loss, without the test-page timing.
pub fn pace_line(row: &LinkRow) -> String {
    if row.ip.is_none() {
        return "no address on this connection".to_string();
    }
    let rtt = match row.rtt_ms {
        Some(ms) => format!("{ms:.0} ms"),
        None => "no round trip".to_string(),
    };
    let loss = match row.loss_pct {
        Some(p) if p == 0.0 => "no loss".to_string(),
        Some(p) => format!("{p:.0}% loss"),
        None => "loss unknown".to_string(),
    };
    format!("{rtt}, {loss}")
}

/// How long a small test page took to load. Empty when there is nothing to say.
pub fn page_line(row: &LinkRow) -> String {
    if row.ip.is_none() {
        return String::new();
    }
    match (row.http_code, row.http_s) {
        (Some(200), Some(s)) => format!("test page loaded in {s:.2} s"),
        (Some(code), _) => format!("test page came back as http {code}"),
        (None, _) => "test page did not load".to_string(),
    }
}

pub fn path_line(row: &LinkRow) -> String {
    match (row.path.as_str(), row.ip.as_deref()) {
        ("iphone-usb", Some(ip)) if ip.starts_with("172.20.10.") => "iPhone USB".to_string(),
        ("wifi", Some(ip)) if ip.starts_with("172.20.10.") => {
            "iPhone hotspot over Wi-Fi".to_string()
        }
        ("iphone-usb", Some(_)) => "iPhone USB".to_string(),
        ("wifi", Some(_)) => "Wi-Fi".to_string(),
        ("iphone-usb", None) => "iPhone USB, no address".to_string(),
        ("wifi", None) => "Wi-Fi, no address".to_string(),
        ("other", _) => "another network".to_string(),
        (other, _) => other.to_string(),
    }
}

/// "150 Helen St, Lewistown, PA" becomes "Lewistown, PA".
/// A landmark name is kept when the place mentions a curve, tunnel, gap, or summit.
pub fn short_place(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() || raw == "-" {
        return String::new();
    }
    let parts: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    for part in &parts {
        let lower = part.to_ascii_lowercase();
        let landmark = ["curve", "tunnel", "gap", "summit"]
            .iter()
            .any(|w| lower.contains(w));
        if landmark {
            return match parts.last() {
                Some(st) if parts.len() >= 2 && *st != *part => format!("{part}, {st}"),
                _ => (*part).to_string(),
            };
        }
    }
    match parts.as_slice() {
        [] => String::new(),
        [only] => (*only).to_string(),
        [.., town, st] => format!("{town}, {st}"),
    }
}

pub fn motion_between(older: &Fix, newer: &Fix) -> Option<String> {
    let dt = newer.at.signed_duration_since(older.at).num_seconds();
    if !(30..=300).contains(&dt) {
        return None;
    }
    let meters = haversine_m(older.lat, older.lon, newer.lat, newer.lon);
    if !(150.0..=8_000.0).contains(&meters) {
        return None;
    }
    let bearing = bearing_deg(older.lat, older.lon, newer.lat, newer.lon);
    let compass = [
        "north",
        "northeast",
        "east",
        "southeast",
        "south",
        "southwest",
        "west",
        "northwest",
    ];
    let idx = ((bearing + 22.5) / 45.0).floor() as usize % 8;
    Some(format!("moving {}", compass[idx]))
}

/// One 30-second reading. `label` is set on the block that starts a ten-minute mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinuteLight {
    pub tone: Tone,
    pub label: Option<String>,
}

/// Two blocks per minute, oldest first, through the current 30 seconds.
/// Each block keeps the worst sample in its half minute, so a drop stays visible.
pub fn minute_lights(rows: &[LinkRow], now: DateTime<Local>, minutes: i64) -> Vec<MinuteLight> {
    let count = minutes.max(1) * 2;
    let end = half_floor(now);
    (0..count)
        .map(|i| {
            let start = end - chrono::Duration::seconds(30 * (count - 1 - i));
            let tone = half_tone(rows, start, start + chrono::Duration::seconds(30));
            let label = (start.second() == 0 && start.minute() % 10 == 0).then(|| clock_hm(start));
            MinuteLight { tone, label }
        })
        .collect()
}

fn half_floor(t: DateTime<Local>) -> DateTime<Local> {
    let minute = minute_floor(t);
    if t.second() >= 30 {
        minute + chrono::Duration::seconds(30)
    } else {
        minute
    }
}

fn half_tone(rows: &[LinkRow], start: DateTime<Local>, end: DateTime<Local>) -> Tone {
    rows.iter()
        .filter(|r| r.is_default && r.at >= start && r.at < end)
        .map(|r| tone_of(r, Duration::from_secs(0)))
        .fold(Tone::NoSample, worse)
}

fn worse(a: Tone, b: Tone) -> Tone {
    fn rank(tone: Tone) -> u8 {
        match tone {
            Tone::NoSample => 0,
            Tone::Clear => 1,
            Tone::Uneven => 2,
            Tone::Poor => 3,
            Tone::Down => 4,
        }
    }
    if rank(a) >= rank(b) {
        a
    } else {
        b
    }
}

fn minute_floor(t: DateTime<Local>) -> DateTime<Local> {
    t.with_second(0)
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(t)
}

pub fn clock_hm(t: DateTime<Local>) -> String {
    let h = match t.hour() % 12 {
        0 => 12,
        n => n,
    };
    format!("{h}:{:02}", t.minute())
}

fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6_371_000.0;
    let p1 = lat1.to_radians();
    let p2 = lat2.to_radians();
    let dp = (lat2 - lat1).to_radians();
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * r * a.sqrt().asin()
}

fn bearing_deg(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let p1 = lat1.to_radians();
    let p2 = lat2.to_radians();
    let dl = (lon2 - lon1).to_radians();
    let y = dl.sin() * p2.cos();
    let x = p1.cos() * p2.sin() - p1.sin() * p2.cos() * dl.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

pub fn fix_age(fix: &Fix, now: DateTime<Utc>) -> Option<Duration> {
    now.signed_duration_since(fix.at).to_std().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn row(ip: Option<&str>, loss: Option<f64>, rtt: Option<f64>, http: Option<u16>) -> LinkRow {
        LinkRow {
            at: Local::now(),
            path: "iphone-usb".into(),
            dev: "en7".into(),
            ip: ip.map(str::to_string),
            loss_pct: loss,
            rtt_ms: rtt,
            http_code: http,
            http_s: Some(0.09),
            is_default: true,
        }
    }

    #[test]
    fn clear_is_a_fast_page_with_little_loss() {
        let row = row(Some("172.20.10.3"), Some(0.0), Some(50.0), Some(200));
        assert_eq!(tone_of(&row, Duration::from_secs(5)), Tone::Clear);
    }

    #[test]
    fn a_stale_row_is_not_called_down() {
        let row = row(None, None, None, None);
        assert_eq!(tone_of(&row, Duration::from_secs(120)), Tone::NoSample);
    }

    #[test]
    fn place_keeps_the_town_and_a_landmark() {
        assert_eq!(short_place("150 Helen St, Lewistown, PA"), "Lewistown, PA");
        assert_eq!(
            short_place("Horseshoe Curve, Altoona, PA"),
            "Horseshoe Curve, PA"
        );
    }

    #[test]
    fn motion_needs_a_real_move_inside_a_few_minutes() {
        let older = Fix {
            at: Utc.with_ymd_and_hms(2026, 10, 21, 17, 50, 0).unwrap(),
            lat: 40.50,
            lon: -77.80,
            acc_m: 40.0,
            place: String::new(),
        };
        let newer = Fix {
            at: Utc.with_ymd_and_hms(2026, 10, 21, 17, 52, 0).unwrap(),
            lat: 40.51,
            lon: -77.76,
            acc_m: 40.0,
            place: String::new(),
        };
        let motion = motion_between(&older, &newer).unwrap();
        assert!(motion.contains("east"), "{motion}");
    }

    #[test]
    fn lights_mark_each_ten_minute_point() {
        let now = Local.with_ymd_and_hms(2026, 10, 21, 14, 23, 40).unwrap();
        let mut sample = row(Some("172.20.10.3"), Some(0.0), Some(50.0), Some(200));
        sample.at = Local.with_ymd_and_hms(2026, 10, 21, 14, 20, 10).unwrap();
        sample.is_default = true;
        let mut dropped = sample.clone();
        dropped.at = Local.with_ymd_and_hms(2026, 10, 21, 14, 20, 40).unwrap();
        dropped.ip = None;
        let lights = minute_lights(&[sample, dropped], now, 10);
        assert_eq!(lights.len(), 20);
        let mark_at = lights
            .iter()
            .position(|l| l.label.as_deref() == Some("2:20"))
            .expect("2:20");
        assert_eq!(lights[mark_at].tone, Tone::Clear);
        assert_eq!(lights[mark_at + 1].tone, Tone::Down);
        assert!(lights[mark_at + 1].label.is_none());
    }
}
