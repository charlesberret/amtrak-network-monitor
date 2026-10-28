//! Follows the sampler already running on this Mac, or measures the default
//! route itself when that sampler is absent. It does not write the sampler's
//! files, and it does not decide where a route starts or ends.

mod feed;
mod model;
mod probe;
mod ui;

use crate::feed::{default_row, read_fixes, read_links, Fix, LinkRow};
use crate::model::{
    fix_age, minute_lights, motion_between, pace_line, page_line, path_line, short_place, tone_of,
    Snapshot, Tone,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Local, TimeZone, Utc};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::Write;
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
    if let Some(parent) = args.log.parent() {
        fs::create_dir_all(parent)?;
    }
    if args.once {
        let mut last_probe = None;
        let mut measured = Vec::new();
        let loaded = load(&args, &mut last_probe, &mut measured)?;
        print_once(&loaded.view);
        println!("Trip log: {}", args.log.display());
        append(
            &args.log,
            &json!({
                "event": "check",
                "at": Local::now().to_rfc3339(),
                "tone": loaded.view.tone.word(),
                "place": loaded.view.place,
                "source": loaded.view.source,
            }),
        )?;
        return Ok(());
    }

    append(
        &args.log,
        &json!({
            "event": "started",
            "at": Local::now().to_rfc3339(),
            "line": args.line,
            "train": args.train,
            "loc": args.loc,
            "sampler_pid": probe::sampler_pid(),
            "location_helper_pid": probe::location_helper_pid(),
            "following": following(&args),
            "exe_mtime": exe_mtime(),
        }),
    )?;

    let mut terminal = ratatui::init();
    let run_result = run(&mut terminal, &args);
    ratatui::restore();
    if matches!(run_result, Ok(SessionEnd::Reload)) {
        let _ = append(
            &args.log,
            &json!({
                "event": "reloaded",
                "at": Local::now().to_rfc3339(),
                "exe_mtime": exe_mtime(),
            }),
        );
        return Err(relaunch());
    }
    let _ = append(
        &args.log,
        &json!({
            "event": "stopped",
            "at": Local::now().to_rfc3339(),
        }),
    );
    if matches!(run_result, Ok(SessionEnd::Quit)) {
        println!("Trip log: {}", args.log.display());
    }
    run_result.map(|_| ())
}

fn run(terminal: &mut ratatui::DefaultTerminal, args: &Args) -> Result<SessionEnd> {
    let mut last_load = Instant::now() - Duration::from_secs(2);
    let mut last_beat = Instant::now();
    let mut last_sample_at: Option<DateTime<Local>> = None;
    let mut last_probe: Option<(Instant, LinkRow)> = None;
    let mut measured: Vec<LinkRow> = Vec::new();
    let mut snap = load(args, &mut last_probe, &mut measured)?;
    let running_mtime = exe_modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let mut pending_build: Option<(SystemTime, Instant)> = None;

    loop {
        if let Some(on_disk) = exe_modified() {
            if reload_due(running_mtime, on_disk, &mut pending_build, Instant::now()) {
                return Ok(SessionEnd::Reload);
            }
        }
        if last_load.elapsed() >= Duration::from_secs(1) {
            match load(args, &mut last_probe, &mut measured) {
                Ok(next) => snap = next,
                Err(err) => {
                    let _ = append(
                        &args.log,
                        &json!({
                            "event": "error",
                            "at": Local::now().to_rfc3339(),
                            "message": err.to_string(),
                        }),
                    );
                }
            }
            last_load = Instant::now();
            if snap.sample_stamp != last_sample_at {
                last_sample_at = snap.sample_stamp;
                log_sample(&args.log, &snap)?;
            }
        }

        terminal.draw(|frame| ui::draw(frame, &snap.view))?;

        if last_beat.elapsed() >= Duration::from_secs(30) {
            log_sample_as(&args.log, &snap, "heartbeat")?;
            last_beat = Instant::now();
        }

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press && matches!(key.code, KeyCode::Char('q')) {
                    return Ok(SessionEnd::Quit);
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
    let (row, mut source) = if follow {
        (followed, "Using the sampler already running".to_string())
    } else if probed.is_some() {
        (
            probed,
            "Measuring the connection on this computer".to_string(),
        )
    } else {
        (followed, "No connection measurement right now".to_string())
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
        .map(|f| short_place(&f.place))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "waiting for a location".to_string());
    let position = latest.map(|f| (f.lat, f.lon));
    let accuracy_m = latest.map(|f| f.acc_m);
    let fix_age_d = latest.and_then(|f| fix_age(f, now_utc));
    let motion = latest.and_then(|newer| {
        fixes
            .iter()
            .rev()
            .find(|older| newer.at.signed_duration_since(older.at).num_seconds() >= 30)
            .and_then(|older| motion_between(older, newer))
    });

    if probe::location_helper_pid().is_none()
        && fix_age_d
            .map(|d| d > Duration::from_secs(180))
            .unwrap_or(true)
    {
        source = format!("{source}. No location coming in");
    }

    let chart: &[LinkRow] = if follow { links } else { measured };
    let recording = args
        .log
        .file_name()
        .map(|name| format!("recording {}", name.to_string_lossy()))
        .unwrap_or_default();
    let view = Snapshot {
        at: now_local,
        line: args.line.clone(),
        place,
        position,
        accuracy_m,
        motion,
        tone,
        pace,
        page,
        path,
        is_default,
        history: minute_lights(chart, now_local, 30),
        source,
        recording,
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

fn log_sample_as(path: &Path, loaded: &Loaded, event: &str) -> Result<()> {
    let snap = &loaded.view;
    let (lat, lon) = snap.position.unzip();
    append(
        path,
        &json!({
            "event": event,
            "at": Local::now().to_rfc3339(),
            "sample_at": loaded.sample_stamp.map(|t| t.to_rfc3339()),
            "tone": snap.tone.word(),
            "place": snap.place,
            "lat": lat,
            "lon": lon,
            "pace": snap.pace,
            "page": snap.page,
            "path": snap.path,
            "loss_pct": loaded.row.as_ref().and_then(|row| row.loss_pct),
            "rtt_ms": loaded.row.as_ref().and_then(|row| row.rtt_ms),
            "http_code": loaded.row.as_ref().and_then(|row| row.http_code),
            "http_s": loaded.row.as_ref().and_then(|row| row.http_s),
            "acc_m": snap.accuracy_m,
            "line": snap.line,
            "is_default": snap.is_default,
            "motion": snap.motion,
            "source": snap.source,
            "sample_age_s": snap.sample_age.map(|d| d.as_secs()),
            "fix_age_s": snap.fix_age.map(|d| d.as_secs()),
        }),
    )
}

fn append(path: &Path, value: &serde_json::Value) -> Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{value}")?;
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
    println!("{}", snap.source);
    if let Some(age) = snap.sample_age {
        println!("sample age {}s", age.as_secs());
    }
}

fn exe_mtime() -> Option<String> {
    let path = std::env::current_exe().ok()?;
    let modified = path.metadata().ok()?.modified().ok()?;
    let dt: DateTime<Local> = modified.into();
    Some(dt.to_rfc3339())
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
Leave the location helper running for the whole trip. q quits.
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
