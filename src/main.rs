//! Follows a connection log already being written, or measures the default
//! route itself when that log is absent. It does not write the sampler's
//! files, and it does not decide where a route starts or ends.

mod feed;
mod lines;
mod load;
mod locate;
mod model;
mod probe;
mod settings;
mod share;
mod ui;

use crate::feed::{default_row, read_fixes, read_links, Fix, LinkRow};
use crate::load::{about_kb, Board, LinkKind, Outcome, Press, Wizard};
use crate::model::{
    fix_age, minute_lights, motion_between, pace_line, page_line, path_line, stored_place,
    stored_position, tone_of, Snapshot, Tone, NO_TOWN, WAITING_PLACE,
};
use crate::probe::ProbePlan;
use crate::settings::{Gather, Nudge, SettingsDraft};
use anyhow::{Context, Result};
use chrono::{DateTime, Local, Utc};
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

fn gather_path(home: &Path) -> PathBuf {
    support_dir(home).join("settings.json")
}

fn load_gather(home: &Path) -> Gather {
    let text = fs::read_to_string(gather_path(home)).unwrap_or_default();
    Gather::from_json(&text)
}

fn store_gather(home: &Path, gather: Gather) -> Result<()> {
    write_private(&gather_path(home), &gather.to_json())
}

fn write_private(path: &Path, body: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    writeln!(file, "{body}")?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Where a published build keeps the trip file and the location log.
/// macOS stays in Application Support so the Swift helper and existing trips agree.
/// Everywhere else follows the XDG state directory.
fn support_dir(home: &Path) -> PathBuf {
    let xdg = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from);
    state_dir(home, std::env::consts::OS, xdg.as_deref())
}

fn state_dir(home: &Path, os: &str, xdg: Option<&Path>) -> PathBuf {
    if os == "macos" {
        return home.join("Library/Application Support/amtrak-network-monitor");
    }
    let base = xdg
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".local/state"));
    base.join("amtrak-network-monitor")
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
}

fn ensure_log(args: &mut Args) {
    if !args.log_set {
        args.log = trip_log_path(&home_dir(), args.line.as_deref(), Utc::now());
    }
}

const RESUME_ENV: &str = "AMTRAK_NM_RESUME";

struct Ride {
    on_amtrak: bool,
    link: Option<LinkKind>,
    share: bool,
}

struct Resume {
    board: Board,
    log: Option<PathBuf>,
}

fn session_path(home: &Path) -> PathBuf {
    support_dir(home).join("session.json")
}

/// Set by relaunch so a rebuilt binary does not ask the load-screen questions
/// again in the middle of a ride.
fn resumed(home: &Path) -> Option<Resume> {
    if std::env::var_os(RESUME_ENV).is_none() {
        return None;
    }
    let text = fs::read_to_string(session_path(home)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let on_amtrak = value.get("on_amtrak")?.as_bool()?;
    let line = value
        .get("line")
        .and_then(|item| item.as_str())
        .map(str::to_string);
    let link = value
        .get("link")
        .and_then(|item| item.as_str())
        .and_then(LinkKind::parse);
    let save = value
        .get("save_trip")
        .and_then(|item| item.as_bool())
        .unwrap_or(false);
    let share = value
        .get("share")
        .and_then(|item| item.as_bool())
        .unwrap_or(false);
    let log = value
        .get("log")
        .and_then(|item| item.as_str())
        .map(PathBuf::from);
    Some(Resume {
        board: Board {
            on_amtrak,
            line,
            link,
            save,
            share,
        },
        log,
    })
}

fn accept_log(home: &Path, log: Option<PathBuf>) -> Option<PathBuf> {
    let path = log?;
    let kept = path.starts_with(support_dir(home))
        && path.extension().and_then(|ext| ext.to_str()) == Some("jsonl");
    kept.then_some(path)
}

fn apply_board(args: &mut Args, gather: &mut Gather, board: &Board) -> Ride {
    if board.on_amtrak {
        if let Some(line) = &board.line {
            args.line = Some(line.clone());
        }
    } else {
        args.line = None;
    }
    gather.save_trip = board.save;
    if board.on_amtrak {
        let _ = store_gather(&home_dir(), *gather);
    }
    Ride {
        on_amtrak: board.on_amtrak,
        link: board.link,
        share: board.share && board.save,
    }
}

fn remember_session(args: &Args, gather: &Gather, ride: &Ride) {
    let body = json!({
        "on_amtrak": ride.on_amtrak,
        "line": args.line,
        "link": ride.link.map(LinkKind::stored),
        "save_trip": gather.save_trip,
        "share": ride.share,
        "log": args.log.display().to_string(),
    })
    .to_string();
    let _ = write_private(&session_path(&home_dir()), &body);
}

fn main() -> Result<()> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().map(String::as_str) == Some("share") {
        let stdin = std::io::stdin();
        return share::run(&raw[1..], &support_dir(&home_dir()), &mut stdin.lock());
    }
    let mut args = parse_args()?;
    resolve_inputs(&mut args);
    let mut gather = load_gather(&home_dir());
    if args.once {
        ensure_log(&mut args);
        let mut last_probe = None;
        let mut measured = Vec::new();
        let mut loaded = load(&args, &gather, &mut last_probe, &mut measured)?;
        loaded.view.saving = gather.save_trip;
        print_once(&loaded.view);
        if gather.save_trip {
            println!("Trip log: {}", args.log.display());
            log_sample_as(&args.log, &loaded, "check", None)?;
        } else {
            println!("not saving");
        }
        return Ok(());
    }

    let mut terminal = ratatui::init();
    let ride = if let Some(saved) = resumed(&home_dir()) {
        if let Some(log) = accept_log(&home_dir(), saved.log) {
            args.log = log;
            args.log_set = true;
        }
        apply_board(&mut args, &mut gather, &saved.board)
    } else {
        match ask(&mut terminal, gather.probe_secs)? {
            None => {
                ratatui::restore();
                return Ok(());
            }
            Some(board) => apply_board(&mut args, &mut gather, &board),
        }
    };
    ensure_log(&mut args);
    if !args.loc_set && args.loc == support_dir(&home_dir()).join("location.tsv") {
        locate::start(args.loc.clone(), gather.loc_secs);
    }
    remember_session(&args, &gather, &ride);
    if gather.save_trip {
        mark_trip(
            &args.log,
            "started",
            args.line.as_deref(),
            ride.link,
            ride.share,
        )?;
    }

    let run_result = run(&mut terminal, &args, &mut gather, &ride);
    ratatui::restore();
    if matches!(run_result, Ok(SessionEnd::Reload)) {
        return Err(relaunch());
    }
    if gather.save_trip {
        let _ = mark_trip(&args.log, "stopped", None, ride.link, ride.share);
    }
    if matches!(run_result, Ok(SessionEnd::Quit)) && gather.save_trip {
        println!("Trip log: {}", args.log.display());
        let shared = share_path(&args.log);
        if ride.share && shared.is_file() {
            println!("Anonymous copy: {}", shared.display());
        }
    }
    run_result.map(|_| ())
}

fn ask(terminal: &mut ratatui::DefaultTerminal, probe_secs: u64) -> Result<Option<Board>> {
    let mut wizard = Wizard::new();
    let storage = storage_line(probe_secs);
    loop {
        terminal.draw(|frame| {
            let view = wizard.view(frame.area().height as usize, &storage);
            ui::draw_load(frame, &view);
        })?;
        if !event::poll(Duration::from_millis(200))? {
            wizard.tick();
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let press = match key.code {
            KeyCode::Char(ch) => Press::Char(ch),
            KeyCode::Enter => Press::Enter,
            KeyCode::Backspace => Press::Backspace,
            KeyCode::Up => Press::Up,
            KeyCode::Down => Press::Down,
            KeyCode::Esc => Press::Esc,
            _ => continue,
        };
        match wizard.push(press) {
            Outcome::Stay => {}
            Outcome::Quit => return Ok(None),
            Outcome::Done(board) => return Ok(Some(board)),
        }
    }
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    args: &Args,
    gather: &mut Gather,
    ride: &Ride,
) -> Result<SessionEnd> {
    let mut last_load = Instant::now() - Duration::from_secs(2);
    let mut last_sample_at: Option<DateTime<Local>> = None;
    let mut last_probe: Option<(Instant, LinkRow)> = None;
    let mut measured: Vec<LinkRow> = Vec::new();
    let mut settings_open = false;
    let mut draft = SettingsDraft::from_saved(gather, cfg!(target_os = "macos"));
    let mut snap = load(args, gather, &mut last_probe, &mut measured)?;
    show(&mut snap, gather, ride);
    let running_mtime = exe_modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let mut pending_build: Option<(SystemTime, Instant)> = None;

    loop {
        if let Some(on_disk) = exe_modified() {
            if reload_due(running_mtime, on_disk, &mut pending_build, Instant::now()) {
                remember_session(args, gather, ride);
                return Ok(SessionEnd::Reload);
            }
        }
        if last_load.elapsed() >= Duration::from_secs(1) {
            if let Ok(mut next) = load(args, gather, &mut last_probe, &mut measured) {
                show(&mut next, gather, ride);
                snap = next;
            }
            last_load = Instant::now();
            if gather.save_trip && snap.sample_stamp != last_sample_at {
                last_sample_at = snap.sample_stamp;
                log_sample(&args.log, &snap, ride)?;
            }
        }

        terminal.draw(|frame| {
            if settings_open {
                ui::draw_settings(frame, &snap.view, &draft);
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
                    KeyCode::Char('q') if !settings_open => return Ok(SessionEnd::Quit),
                    KeyCode::Char('s') if !settings_open => {
                        draft = SettingsDraft::from_saved(gather, cfg!(target_os = "macos"));
                        settings_open = true;
                    }
                    KeyCode::Up if settings_open => draft.nudge(Nudge::Up),
                    KeyCode::Down if settings_open => draft.nudge(Nudge::Down),
                    KeyCode::Char(' ') if settings_open => draft.nudge(Nudge::Space),
                    KeyCode::Char('s') if settings_open => {
                        let was_saving = gather.save_trip;
                        draft.apply_to(gather);
                        let _ = store_gather(&home_dir(), *gather);
                        remember_session(args, gather, ride);
                        if gather.save_trip != was_saving {
                            let event = if gather.save_trip {
                                "started"
                            } else {
                                "stopped"
                            };
                            let line = gather.save_trip.then_some(args.line.as_deref()).flatten();
                            let _ = mark_trip(&args.log, event, line, ride.link, ride.share);
                        }
                        show(&mut snap, gather, ride);
                        settings_open = false;
                    }
                    KeyCode::Char('c') if settings_open => {
                        draft = SettingsDraft::from_saved(gather, cfg!(target_os = "macos"));
                        settings_open = false;
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
        .env(RESUME_ENV, "1")
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
    keep_position: bool,
}

fn load(
    args: &Args,
    gather: &Gather,
    last_probe: &mut Option<(Instant, LinkRow)>,
    measured: &mut Vec<LinkRow>,
) -> Result<Loaded> {
    let links = match &args.train {
        Some(path) => read_links(path).unwrap_or_default(),
        None => Vec::new(),
    };
    let fixes = read_fixes(&args.loc).unwrap_or_default();
    load_owned(args, gather, &links, &fixes, last_probe, measured)
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
    gather: &Gather,
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
            .map(|(at, _)| at.elapsed() >= Duration::from_secs(gather.probe_secs.max(1)))
            .unwrap_or(true);
        if due {
            let fresh = probe::measure_default(ProbePlan {
                ping: gather.ping,
                http: gather.http,
            });
            if let Some(row) = fresh.clone() {
                *last_probe = Some((Instant::now(), row.clone()));
                remember(measured, row, Local::now());
            }
            fresh
        } else {
            last_probe.as_ref().map(|(_, row)| row.clone())
        }
    };

    // The probe itself takes longer than a moment. Stamp the screen after it
    // returns, or a fresh row looks ancient and the tone says there is no sample.
    let now_local = Local::now();
    let now_utc = Utc::now();

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
    let place = place_of(latest);
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
        on_amtrak: true,
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
        keep_position: gather.position,
    })
}

fn place_of(latest: Option<&Fix>) -> String {
    match latest {
        Some(fix) => {
            let town = stored_place(&fix.place);
            if town.is_empty() {
                NO_TOWN.to_string()
            } else {
                town
            }
        }
        None => WAITING_PLACE.to_string(),
    }
}

fn show(loaded: &mut Loaded, gather: &Gather, ride: &Ride) {
    loaded.view.saving = gather.save_trip;
    loaded.view.on_amtrak = ride.on_amtrak;
    loaded.keep_position = gather.position;
}

fn log_sample(path: &Path, loaded: &Loaded, ride: &Ride) -> Result<()> {
    log_sample_as(path, loaded, "sample", ride.link.map(LinkKind::stored))?;
    if ride.share {
        append(&share_path(path), &share_record(loaded))?;
    }
    Ok(())
}

fn mark_trip(
    path: &Path,
    event: &str,
    line: Option<&str>,
    link: Option<LinkKind>,
    share: bool,
) -> Result<()> {
    append(
        path,
        &json!({
            "event": event,
            "at": Utc::now().to_rfc3339(),
            "line": line,
            "link": link.map(LinkKind::stored),
            "share": share,
        }),
    )
}

fn sample_record(loaded: &Loaded, event: &str, link: Option<&str>) -> serde_json::Value {
    let snap = &loaded.view;
    let place = if !loaded.keep_position || snap.place == WAITING_PLACE || snap.place == NO_TOWN {
        String::new()
    } else {
        stored_place(&snap.place)
    };
    let (lat, lon) = if loaded.keep_position {
        snap.position
            .map(|(lat, lon)| stored_position(lat, lon))
            .unzip()
    } else {
        (None, None)
    };
    let mut value = json!({
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
    });
    if let Some(link) = link {
        value["link"] = json!(link);
    }
    value
}

/// The anonymous copy. Train line, rounded location, and the connection
/// measurement. Not the connection type, the town, or the time.
fn share_record(loaded: &Loaded) -> serde_json::Value {
    let full = sample_record(loaded, "sample", None);
    json!({
        "line": full.get("line").cloned().unwrap_or(serde_json::Value::Null),
        "lat": full["lat"].clone(),
        "lon": full["lon"].clone(),
        "tone": full["tone"].clone(),
        "loss_pct": full["loss_pct"].clone(),
        "rtt_ms": full["rtt_ms"].clone(),
        "http_code": full["http_code"].clone(),
        "http_s": full["http_s"].clone(),
    })
}

fn share_path(trip: &Path) -> PathBuf {
    let stem = trip
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("trip");
    trip.with_file_name(format!("{stem}-share.jsonl"))
}

/// A row is one probe, and the probe is every 20 seconds.
fn storage_line(probe_secs: u64) -> String {
    let bytes = sample_record(&storage_fixture(), "sample", Some("amtrak-wifi"))
        .to_string()
        .len()
        + 1;
    about_kb(bytes, probe_secs)
}

fn storage_fixture() -> Loaded {
    Loaded {
        view: Snapshot {
            at: Local::now(),
            line: Some("Pennsylvanian".into()),
            on_amtrak: true,
            place: "Harrisburg, PA".into(),
            position: Some((40.262, -76.884)),
            motion: None,
            tone: Tone::Clear,
            pace: "54 ms, no loss".into(),
            page: "test page: 0.22 s".into(),
            path: "iPhone hotspot over Wi-Fi".into(),
            is_default: true,
            history: Vec::new(),
            saving: true,
            sample_age: Some(Duration::from_secs(2)),
            fix_age: Some(Duration::from_secs(5)),
        },
        sample_stamp: None,
        row: Some(LinkRow {
            at: Local::now(),
            path: "wifi".into(),
            dev: "en0".into(),
            ip: None,
            loss_pct: Some(0.0),
            rtt_ms: Some(54.0),
            http_code: Some(200),
            http_s: Some(0.22),
            is_default: true,
            ping_on: true,
            http_on: true,
        }),
        keep_position: true,
    }
}

fn log_sample_as(path: &Path, loaded: &Loaded, event: &str, link: Option<&str>) -> Result<()> {
    append(path, &sample_record(loaded, event, link))
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

Measure the network on an Amtrak ride, and where the train is. Every 20
seconds, or the rate you set, it checks packet loss, round-trip time, and
whether a test page loads, and pairs that with your position. One ride
writes one private file.

On start it asks if you are riding Amtrak, which line, how you are
connected, and whether to keep the trip on this computer. Settings choose
how often to measure, and whether ping, the test page, and the position
are kept. If you keep the trip, you can also keep an anonymous copy: the
train line, the location, and the connection measurement. Nothing is sent
unless you run `share` yourself.

  --line NAME    used when you continue without picking a line
  --once         print one reading and exit. Does not ask.
  --train PATH   follow this connection log instead of measuring
  --loc PATH     read location fixes from this file
  --log PATH     write the trip file here

  share [FILE] [--config PATH]
                 after a ride, send the anonymous copy to a receiver, if you
                 have a share.json for one. It shows what is in the copy and
                 asks first. Without FILE it takes the newest copy.

On macOS the trip file is under ~/Library/Application Support/amtrak-network-monitor/trips/.
On Linux it is under $XDG_STATE_HOME/amtrak-network-monitor/trips/, or .local/state/amtrak-network-monitor/trips/ in the home directory.
It keeps the town and a position to about 100 m when position is on. It does not keep a street, a Wi-Fi name, or an IP address. There is no switch for those.
s opens settings. up and down move. space changes the row. s saves. c cancels. q quits.
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
    use chrono::TimeZone;

    #[test]
    fn linux_state_dir_follows_xdg_and_macos_stays_put() {
        let home = PathBuf::from("/tmp/rider");
        assert_eq!(
            state_dir(&home, "macos", None),
            home.join("Library/Application Support/amtrak-network-monitor")
        );
        assert_eq!(
            state_dir(&home, "linux", None),
            home.join(".local/state/amtrak-network-monitor")
        );
        let custom = PathBuf::from("/tmp/state");
        assert_eq!(
            state_dir(&home, "linux", Some(custom.as_path())),
            custom.join("amtrak-network-monitor")
        );
    }

    #[test]
    fn a_fix_with_no_town_is_not_described_as_waiting() {
        let fix = Fix {
            at: Utc::now(),
            lat: 40.26,
            lon: -76.88,
            acc_m: 80.0,
            place: String::new(),
        };
        assert_eq!(place_of(Some(&fix)), NO_TOWN);
        assert_eq!(place_of(None), WAITING_PLACE);
    }

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
        assert_eq!(loc, support_dir(&home).join("location.tsv"));
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
            support_dir(&home).join("trips/2026-10-30T2205Z-lake-shore-limited.jsonl")
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
                on_amtrak: true,
                place: "New York, NY".into(),
                position: Some((40.760052, -73.966937)),
                motion: None,
                tone: Tone::Clear,
                pace: "19 ms, no loss".into(),
                page: "test page: 0.08 s".into(),
                path: "iPhone hotspot over Wi-Fi".into(),
                is_default: true,
                history: Vec::new(),
                saving: true,
                sample_age: Some(Duration::from_secs(2)),
                fix_age: Some(Duration::from_secs(5)),
            },
            sample_stamp: None,
            row: None,
            keep_position: true,
        };
        let value = sample_record(&loaded, "sample", None);
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
    fn a_shared_row_keeps_only_the_line_the_location_and_the_measurement() {
        let loaded = Loaded {
            view: Snapshot {
                at: Local.with_ymd_and_hms(2026, 10, 28, 12, 0, 0).unwrap(),
                line: Some("Pennsylvanian".into()),
                on_amtrak: true,
                place: "New York, NY".into(),
                position: Some((40.760052, -73.966937)),
                motion: None,
                tone: Tone::Clear,
                pace: "19 ms, no loss".into(),
                page: "test page: 0.08 s".into(),
                path: "iPhone hotspot over Wi-Fi".into(),
                is_default: true,
                history: Vec::new(),
                saving: true,
                sample_age: None,
                fix_age: None,
            },
            sample_stamp: None,
            row: Some(LinkRow {
                at: Local.with_ymd_and_hms(2026, 10, 28, 12, 0, 0).unwrap(),
                path: "wifi".into(),
                dev: "en0".into(),
                ip: Some("10.0.0.8".into()),
                loss_pct: Some(0.0),
                rtt_ms: Some(19.0),
                http_code: Some(200),
                http_s: Some(0.08),
                is_default: true,
                ping_on: true,
                http_on: true,
            }),
            keep_position: true,
        };
        let shared = share_record(&loaded);
        let keys: Vec<_> = shared.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            vec![
                "http_code",
                "http_s",
                "lat",
                "line",
                "lon",
                "loss_pct",
                "rtt_ms",
                "tone",
            ]
        );
        let text = shared.to_string();
        assert!(text.contains("Pennsylvanian"));
        assert!(text.contains("40.76"));
        assert!(!text.contains("New York"));
        assert!(!text.contains("hotspot"));
        assert!(!text.contains("amtrak-wifi"));
        assert!(!text.contains("personal-wifi"));
        assert!(!text.contains("10.0.0.8"));
        assert!(!text.contains("40.760052"));
        let local = sample_record(&loaded, "sample", Some("personal-wifi"));
        assert_eq!(local["link"], "personal-wifi");
        assert!(local["place"].as_str().unwrap().contains("New York"));
    }

    #[test]
    fn an_hour_of_saving_is_a_small_number_of_kilobytes() {
        let line = storage_line(20);
        assert!(line.starts_with("About "), "{line}");
        assert!(line.ends_with(" kB an hour."), "{line}");
        let n: u32 = line.split_whitespace().nth(1).unwrap().parse().unwrap();
        assert!((20..=200).contains(&n), "{line}");
    }

    #[test]
    fn a_resumed_log_has_to_stay_inside_the_support_folder() {
        let home = PathBuf::from("/tmp/rider");
        let kept = support_dir(&home).join("trips/2026-10-28T1800Z-pennsylvanian.jsonl");
        assert_eq!(accept_log(&home, Some(kept.clone())), Some(kept));
        let outside = PathBuf::from("/tmp/elsewhere/trip.jsonl");
        assert_eq!(accept_log(&home, Some(outside)), None);
        assert_eq!(
            share_path(&PathBuf::from(
                "/tmp/trips/2026-10-28T1800Z-pennsylvanian.jsonl"
            )),
            PathBuf::from("/tmp/trips/2026-10-28T1800Z-pennsylvanian-share.jsonl")
        );
    }

    #[test]
    fn the_binary_already_running_is_left_alone() {
        let running = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let mut pending = Some((running, Instant::now()));
        assert!(!reload_due(running, running, &mut pending, Instant::now()));
        assert!(pending.is_none());
    }
}
