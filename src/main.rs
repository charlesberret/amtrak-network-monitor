//! Follows the sampler already running on this Mac, or measures the default
//! route itself when that sampler is absent. It does not write the sampler's
//! files, and it does not decide where a route starts or ends.

mod feed;
mod model;
mod probe;
mod ui;

use crate::feed::{default_row, read_fixes, read_links, Fix, LinkRow};
use crate::model::{
    fix_age, minute_lights, motion_between, pace_line, page_line, path_line, stored_place,
    stored_position, tone_of, Snapshot, Tone,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Local, TimeZone, Utc};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

struct Args {
    once: bool,
    line: Option<String>,
    train: Option<PathBuf>,
    loc: PathBuf,
    log: PathBuf,
    train_set: bool,
    loc_set: bool,
    log_set: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Machine {
    sampler: bool,
    legacy_location: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Gather {
    save_trip: bool,
}

fn gather_path(home: &Path) -> PathBuf {
    support_dir(home).join("settings.json")
}

fn load_gather(home: &Path) -> Gather {
    let text = fs::read_to_string(gather_path(home)).unwrap_or_default();
    let save_trip = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|value| value.get("save_trip").and_then(|v| v.as_bool()))
        .unwrap_or(true);
    Gather { save_trip }
}

fn store_gather(home: &Path, gather: Gather) -> Result<()> {
    let path = gather_path(home);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
    }
    let body = json!({"save_trip": gather.save_trip}).to_string();
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)?;
    writeln!(file, "{body}")?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Where a published build keeps the trip file and the location helper's log.
fn support_dir(home: &Path) -> PathBuf {
    home.join("Library/Application Support/amtrak-network-monitor")
}

fn choose_inputs(
    machine: Machine,
    train_flag: Option<PathBuf>,
    loc_flag: Option<PathBuf>,
    home: &Path,
) -> (Option<PathBuf>, PathBuf) {
    let legacy = home.join(".cache/link-quality");
    let train = match train_flag {
        Some(path) => Some(path),
        None if machine.sampler => Some(legacy.join("train.tsv")),
        None => None,
    };
    let loc = match loc_flag {
        Some(path) => path,
        None if machine.sampler || machine.legacy_location => legacy.join("loc-last.tsv"),
        None => support_dir(home).join("location.tsv"),
    };
    (train, loc)
}

fn trip_log_path(home: &Path, line: Option<&str>, now: DateTime<Utc>) -> PathBuf {
    let stamp = now.format("%Y-%m-%dT%H%MZ");
    let name = slug(line.unwrap_or("trip"));
    support_dir(home)
        .join("trips")
        .join(format!("{stamp}-{name}.jsonl"))
}

fn slug(line: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in line.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "trip".to_string()
    } else {
        out
    }
}

fn following(args: &Args) -> bool {
    match &args.train {
        Some(_) if args.train_set => true,
        Some(_) => probe::sampler_pid().is_some(),
        None => false,
    }
}

fn resolve_inputs(args: &mut Args) {
    let machine = Machine {
        sampler: probe::sampler_pid().is_some(),
        legacy_location: probe::legacy_location_pid().is_some(),
    };
    let home = home_dir();
    let loc_flag = args.loc_set.then(|| args.loc.clone());
    let (train, loc) = choose_inputs(machine, args.train.clone(), loc_flag, &home);
    args.train = train;
    args.loc = loc;
    if !args.log_set {
        args.log = trip_log_path(&home, args.line.as_deref(), Utc::now());
    }
}

fn main() -> Result<()> {
    let mut args = parse_args()?;
    resolve_inputs(&mut args);
    let mut gather = load_gather(&home_dir());
    if args.once {
        let mut last_probe = None;
        let mut measured = Vec::new();
        let mut loaded = load(&args, &mut last_probe, &mut measured)?;
        loaded.view.saving = gather.save_trip;
        print_once(&loaded.view);
        if gather.save_trip {
            println!("Trip log: {}", args.log.display());
            log_sample_as(&args.log, &loaded, "check")?;
        } else {
            println!("not saving");
        }
        return Ok(());
    }

    if gather.save_trip {
        mark_trip(&args.log, "started", args.line.as_deref())?;
    }

    let mut terminal = ratatui::init();
    let run_result = run(&mut terminal, &args, &mut gather);
    ratatui::restore();
    if matches!(run_result, Ok(SessionEnd::Reload)) {
        return Err(relaunch());
    }
    if gather.save_trip {
        let _ = mark_trip(&args.log, "stopped", None);
    }
    if matches!(run_result, Ok(SessionEnd::Quit)) && gather.save_trip {
        println!("Trip log: {}", args.log.display());
    }
    run_result.map(|_| ())
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    args: &Args,
    gather: &mut Gather,
) -> Result<SessionEnd> {
    let mut last_load = Instant::now() - Duration::from_secs(2);
    let mut last_sample_at: Option<DateTime<Local>> = None;
    let mut last_probe: Option<(Instant, LinkRow)> = None;
    let mut measured: Vec<LinkRow> = Vec::new();
    let mut settings = false;
    let mut snap = load(args, &mut last_probe, &mut measured)?;
    snap.view.saving = gather.save_trip;
    let running_mtime = exe_modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let mut pending_build: Option<(SystemTime, Instant)> = None;

    loop {
        if let Some(on_disk) = exe_modified() {
            if reload_due(running_mtime, on_disk, &mut pending_build, Instant::now()) {
                return Ok(SessionEnd::Reload);
            }
        }
        if last_load.elapsed() >= Duration::from_secs(1) {
            if let Ok(mut next) = load(args, &mut last_probe, &mut measured) {
                next.view.saving = gather.save_trip;
                snap = next;
            }
            last_load = Instant::now();
            if gather.save_trip && snap.sample_stamp != last_sample_at {
                last_sample_at = snap.sample_stamp;
                log_sample(&args.log, &snap)?;
            }
        }

        terminal.draw(|frame| {
            if settings {
                ui::draw_settings(frame, &snap.view);
            } else {
                ui::draw(frame, &snap.view);
            }
        })?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') => return Ok(SessionEnd::Quit),
                    KeyCode::Char('s') => settings = !settings,
                    KeyCode::Char(' ') if settings => {
                        gather.save_trip = !gather.save_trip;
                        snap.view.saving = gather.save_trip;
                        let _ = store_gather(&home_dir(), *gather);
                        if gather.save_trip {
                            let _ = mark_trip(&args.log, "started", args.line.as_deref());
                        } else {
                            let _ = mark_trip(&args.log, "stopped", None);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionEnd {
    Quit,
    Reload,
}

/// A newer binary must sit unchanged for a second, so a build still writing
/// is not executed.
fn reload_due(
    running_mtime: SystemTime,
    on_disk: SystemTime,
    pending: &mut Option<(SystemTime, Instant)>,
    now: Instant,
) -> bool {
    if on_disk <= running_mtime {
        *pending = None;
        return false;
    }
    match *pending {
        Some((seen, at)) if seen == on_disk => {
            now.saturating_duration_since(at) >= Duration::from_secs(1)
        }
        _ => {
            *pending = Some((on_disk, now));
            false
        }
    }
}

fn relaunch() -> anyhow::Error {
    let path = match std::env::current_exe() {
        Ok(path) => path,
        Err(err) => return err.into(),
    };
    let err = std::process::Command::new(&path)
        .args(std::env::args_os().skip(1))
        .exec();
    err.into()
}

fn exe_modified() -> Option<SystemTime> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.metadata().ok())
        .and_then(|meta| meta.modified().ok())
}

struct Loaded {
    view: Snapshot,
    sample_stamp: Option<DateTime<Local>>,
    row: Option<LinkRow>,
}

fn load(
    args: &Args,
    last_probe: &mut Option<(Instant, LinkRow)>,
    measured: &mut Vec<LinkRow>,
) -> Result<Loaded> {
    let now_local = Local::now();
    let now_utc = Utc::now();
    let links = match &args.train {
        Some(path) => read_links(path).unwrap_or_default(),
        None => Vec::new(),
    };
    let fixes = read_fixes(&args.loc).unwrap_or_default();
    load_owned(
        args,
        now_local,
        now_utc,
        &links,
        &fixes,
        last_probe,
        measured,
    )
}

fn remember(measured: &mut Vec<LinkRow>, row: LinkRow, now: DateTime<Local>) {
    if measured.last().map(|kept| kept.at) != Some(row.at) {
        measured.push(row);
    }
    let cutoff = now - chrono::Duration::minutes(45);
    measured.retain(|kept| kept.at >= cutoff);
}

fn load_owned(
    args: &Args,
    now_local: DateTime<Local>,
    now_utc: DateTime<Utc>,
    links: &[LinkRow],
    fixes: &[Fix],
    last_probe: &mut Option<(Instant, LinkRow)>,
    measured: &mut Vec<LinkRow>,
) -> Result<Loaded> {
    let follow = following(args);
    let probed: Option<LinkRow> = if follow {
        *last_probe = None;
        None
    } else {
        let due = last_probe
            .as_ref()
            .map(|(at, _)| at.elapsed() >= Duration::from_secs(20))
            .unwrap_or(true);
        if due {
            let fresh = probe::measure_default();
            if let Some(row) = fresh.clone() {
                *last_probe = Some((Instant::now(), row.clone()));
                remember(measured, row, now_local);
            }
            fresh
        } else {
            last_probe.as_ref().map(|(_, row)| row.clone())
        }
    };

    let followed = default_row(links).cloned();
    let row = if follow {
        followed
    } else if probed.is_some() {
        probed
    } else {
        followed
    };

    let sample_stamp = row.as_ref().map(|r| r.at);
    let sample_age = row
        .as_ref()
        .and_then(|r| now_local.signed_duration_since(r.at).to_std().ok());
    let age = sample_age.unwrap_or(Duration::from_secs(10_000));

    let (tone, pace, page, path, is_default) = match row.as_ref() {
        Some(row) => (
            tone_of(row, age),
            pace_line(row),
            page_line(row),
            path_line(row),
            row.is_default,
        ),
        None => (
            Tone::NoSample,
            "no connection sample yet".to_string(),
            String::new(),
            String::new(),
            false,
        ),
    };

    let latest = fixes.last();
    let place = latest
        .map(|f| stored_place(&f.place))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "waiting for a location".to_string());
    let position = latest.map(|f| stored_position(f.lat, f.lon));
    let fix_age_d = latest.and_then(|f| fix_age(f, now_utc));
    let motion = latest.and_then(|newer| {
        fixes
            .iter()
            .rev()
            .find(|older| newer.at.signed_duration_since(older.at).num_seconds() >= 30)
            .and_then(|older| motion_between(older, newer))
    });

    let chart: &[LinkRow] = if follow { links } else { measured };
    let view = Snapshot {
        at: now_local,
        line: args.line.clone(),
        place,
        position,
        motion,
        tone,
        pace,
        page,
        path,
        is_default,
        history: minute_lights(chart, now_local, 30),
        saving: true,
        sample_age,
        fix_age: fix_age_d,
    };
    Ok(Loaded {
        view,
        sample_stamp,
        row: row.clone(),
    })
}

fn log_sample(path: &Path, loaded: &Loaded) -> Result<()> {
    log_sample_as(path, loaded, "sample")
}

fn mark_trip(path: &Path, event: &str, line: Option<&str>) -> Result<()> {
    append(
        path,
        &json!({
            "event": event,
            "at": Utc::now().to_rfc3339(),
            "line": line,
        }),
    )
}

fn sample_record(loaded: &Loaded, event: &str) -> serde_json::Value {
    let snap = &loaded.view;
    let place = if snap.place == "waiting for a location" {
        String::new()
    } else {
        stored_place(&snap.place)
    };
    let (lat, lon) = snap
        .position
        .map(|(lat, lon)| stored_position(lat, lon))
        .unzip();
    json!({
        "event": event,
        "at": Utc::now().to_rfc3339(),
        "tone": snap.tone.word(),
        "place": place,
        "lat": lat,
        "lon": lon,
        "path": snap.path,
        "loss_pct": loaded.row.as_ref().and_then(|row| row.loss_pct),
        "rtt_ms": loaded.row.as_ref().and_then(|row| row.rtt_ms),
        "http_code": loaded.row.as_ref().and_then(|row| row.http_code),
        "http_s": loaded.row.as_ref().and_then(|row| row.http_s),
        "line": snap.line,
    })
}

fn log_sample_as(path: &Path, loaded: &Loaded, event: &str) -> Result<()> {
    append(path, &sample_record(loaded, event))
}

fn append(path: &Path, value: &serde_json::Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?;
    writeln!(file, "{value}")?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn print_once(snap: &Snapshot) {
    println!("{}", snap.place);
    if let Some((lat, lon)) = snap.position {
        println!("{lat:.6}, {lon:.6}");
    }
    if let Some(motion) = &snap.motion {
        println!("{motion}");
    }
    println!("{}", snap.tone.word());
    println!("{}", snap.pace);
    if !snap.page.is_empty() {
        println!("{}", snap.page);
    }
    if !snap.path.is_empty() {
        println!("{}", snap.path);
    }
    if snap.is_default {
        println!("default route");
    }
    if let Some(age) = snap.sample_age {
        println!("sample age {}s", age.as_secs());
    }
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        once: false,
        line: None,
        train: None,
        loc: PathBuf::new(),
        log: PathBuf::new(),
        train_set: false,
        loc_set: false,
        log_set: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--once" => args.once = true,
            "--line" => {
                args.line = Some(it.next().context("--line needs the train name")?);
            }
            "--train" => {
                args.train = Some(PathBuf::from(it.next().context("--train needs a path")?));
                args.train_set = true;
            }
            "--loc" => {
                args.loc = PathBuf::from(it.next().context("--loc needs a path")?);
                args.loc_set = true;
            }
            "--log" => {
                args.log = PathBuf::from(it.next().context("--log needs a path")?);
                args.log_set = true;
            }
            "--help" | "-h" => {
                println!(
                    "\
amtrak-network-monitor

Records the connection this computer is using, and where the train is
when the location helper is running. One run writes one trip file.

  --line NAME    train name in the title and the file name. Any Amtrak train.
  --once         print one reading and exit
  --train PATH   follow this connection log instead of measuring
  --loc PATH     read location fixes from this file
  --log PATH     write the trip file here

The trip file is created under ~/Library/Application Support/amtrak-network-monitor/trips/.
It keeps the town and a position to about 100 m. It does not keep a street, a Wi-Fi name, or an IP address.
s opens settings. space there turns saving on or off. q quits.
"
                );
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument {other}. Try --help"),
        }
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rider_without_the_sampler_measures_and_reads_their_own_location_file() {
        let home = PathBuf::from("/tmp/rider");
        let (train, loc) = choose_inputs(
            Machine {
                sampler: false,
                legacy_location: false,
            },
            None,
            None,
            &home,
        );
        assert!(train.is_none());
        assert_eq!(
            loc,
            home.join("Library/Application Support/amtrak-network-monitor/location.tsv")
        );
    }

    #[test]
    fn an_already_running_sampler_is_followed_and_not_required_by_name_of_a_home() {
        let home = PathBuf::from("/tmp/rider");
        let (train, loc) = choose_inputs(
            Machine {
                sampler: true,
                legacy_location: true,
            },
            None,
            None,
            &home,
        );
        assert_eq!(train, Some(home.join(".cache/link-quality/train.tsv")));
        assert_eq!(loc, home.join(".cache/link-quality/loc-last.tsv"));
    }

    #[test]
    fn an_explicit_train_file_is_kept() {
        let home = PathBuf::from("/tmp/rider");
        let chosen = PathBuf::from("/tmp/rider/own.tsv");
        let (train, _) = choose_inputs(
            Machine {
                sampler: false,
                legacy_location: false,
            },
            Some(chosen.clone()),
            None,
            &home,
        );
        assert_eq!(train, Some(chosen));
    }

    #[test]
    fn the_trip_file_names_the_train_and_the_hour() {
        let home = PathBuf::from("/tmp/rider");
        let now = Utc.with_ymd_and_hms(2026, 10, 30, 22, 5, 0).unwrap();
        let path = trip_log_path(&home, Some("Lake Shore Limited"), now);
        assert_eq!(
            path,
            home.join(
                "Library/Application Support/amtrak-network-monitor/trips/2026-10-30T2205Z-lake-shore-limited.jsonl"
            )
        );
        assert_eq!(slug("Northeast Regional"), "northeast-regional");
        assert_eq!(slug("   "), "trip");
    }

    #[test]
    fn a_saved_sample_has_no_street_network_name_or_precise_position() {
        let loaded = Loaded {
            view: Snapshot {
                at: Local.with_ymd_and_hms(2026, 10, 28, 12, 0, 0).unwrap(),
                line: Some("Lake Shore Limited".into()),
                place: "New York, NY".into(),
                position: Some((40.760052, -73.966937)),
                motion: None,
                tone: Tone::Clear,
                pace: "19 ms, no loss".into(),
                page: "test page loaded in 0.08 s".into(),
                path: "iPhone hotspot over Wi-Fi".into(),
                is_default: true,
                history: Vec::new(),
                saving: true,
                sample_age: Some(Duration::from_secs(2)),
                fix_age: Some(Duration::from_secs(5)),
            },
            sample_stamp: None,
            row: None,
        };
        let value = sample_record(&loaded, "sample");
        let text = value.to_string();
        assert!(text.contains("New York, NY"));
        assert!(text.contains("40.76"));
        assert!(!text.contains("40.760052"));
        assert!(!text.contains("73.966937"));
        assert!(!text.contains("ssid"));
        assert!(!text.contains("ip"));
        assert!(!text.contains("acc_m"));
        assert!(!text.contains("source"));
        assert!(!text.contains("sampler"));
        assert!(value.get("loss_pct").is_some());
    }

    #[test]
    fn the_settings_file_is_private_to_the_user() {
        let dir = std::env::temp_dir().join(format!("amtrak-nm-settings-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let home_file = path.clone();
        // store_gather writes under the support dir. Write through append's rules instead.
        append(&home_file, &json!({"save_trip": false})).unwrap();
        let mode = fs::metadata(&home_file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let parent_mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(parent_mode, 0o700);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_waits_until_the_new_binary_settles() {
        let running = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let newer = SystemTime::UNIX_EPOCH + Duration::from_secs(20);
        let mut pending = None;
        let t0 = Instant::now();
        assert!(!reload_due(running, newer, &mut pending, t0));
        assert!(!reload_due(
            running,
            newer,
            &mut pending,
            t0 + Duration::from_millis(400)
        ));
        let replaced = SystemTime::UNIX_EPOCH + Duration::from_secs(30);
        assert!(!reload_due(
            running,
            replaced,
            &mut pending,
            t0 + Duration::from_secs(2)
        ));
        assert!(reload_due(
            running,
            replaced,
            &mut pending,
            t0 + Duration::from_secs(3)
        ));
    }

    #[test]
    fn the_binary_already_running_is_left_alone() {
        let running = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let mut pending = Some((running, Instant::now()));
        assert!(!reload_due(running, running, &mut pending, Instant::now()));
        assert!(pending.is_none());
    }
}
