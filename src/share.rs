//! `amtrak-network-monitor share`: send one anonymous copy to a receiver,
//! after the rider has seen what is in it and said yes.
//!
//! The program never sends on its own. The address and the token come from
//! `share.json` in the support folder, a file the person who gave you this
//! program sends you. curl does the sending, so the token stays out of the
//! process list and no crate is added.

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

struct Config {
    url: String,
    token: String,
}

/// The fields of a row in an anonymous copy, and no others.
const FIELDS: [&str; 8] = [
    "http_code", "http_s", "lat", "line", "lon", "loss_pct", "rtt_ms", "tone",
];

/// An anonymous copy as the rider was shown it. `body` is the exact bytes
/// that were read, and the same bytes are what gets sent.
struct Copy {
    rows: usize,
    lines: BTreeSet<String>,
    body: Vec<u8>,
}

/// A loopback address is allowed over plain http so the receiver can be tried
/// on one computer. Anything else must be https.
fn check_url(url: &str) -> Result<()> {
    if !url
        .chars()
        .all(|c| c.is_ascii_graphic() && c != '"' && c != '\\' && c != '@')
    {
        bail!("the address in share.json has a character it should not have");
    }
    let loopback = ["http://127.0.0.1", "http://localhost"].iter().any(|prefix| {
        url.strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with(':') || rest.starts_with('/'))
    });
    if !(url.starts_with("https://") || loopback) {
        bail!("the address in share.json must start with https://");
    }
    Ok(())
}

fn check_token(token: &str) -> Result<()> {
    let fine = (20..=128).contains(&token.len())
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !fine {
        bail!("the token in share.json is not one this program can send");
    }
    Ok(())
}

fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split('/').next().unwrap_or(rest)
}

fn load_config(path: &Path) -> Result<Config> {
    let meta = fs::metadata(path).with_context(|| {
        format!(
            "no {}. The person who gave you this program sends you that file.",
            path.display()
        )
    })?;
    if meta.permissions().mode() & 0o077 != 0 {
        bail!(
            "{} can be read by other users. Make it private first: chmod 600 {}",
            path.display(),
            path.display()
        );
    }
    let text = fs::read_to_string(path)?;
    let value: Value = serde_json::from_str(&text).context("share.json is not valid JSON")?;
    let field = |name: &str| -> Result<String> {
        value
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_string)
            .with_context(|| format!("share.json has no \"{name}\""))
    };
    let config = Config {
        url: field("url")?,
        token: field("token")?,
    };
    check_url(&config.url)?;
    check_token(&config.token)?;
    Ok(config)
}

fn read_copy(path: &Path) -> Result<Copy> {
    let body = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let text = String::from_utf8(body.clone())
        .with_context(|| format!("{} is not an anonymous copy", path.display()))?;
    let mut rows = 0;
    let mut lines = BTreeSet::new();
    for raw in text.lines().filter(|l| !l.trim().is_empty()) {
        let value: Value = serde_json::from_str(raw)
            .with_context(|| format!("{} is not an anonymous copy", path.display()))?;
        let exact = value.as_object().is_some_and(|object| {
            object.len() == FIELDS.len() && FIELDS.iter().all(|key| object.contains_key(*key))
        });
        if !exact {
            bail!("{} is not an anonymous copy", path.display());
        }
        rows += 1;
        if let Some(name) = value.get("line").and_then(Value::as_str) {
            lines.insert(name.to_string());
        }
    }
    if rows == 0 {
        bail!("{} has no rows", path.display());
    }
    Ok(Copy { rows, lines, body })
}

fn newest_copy(trips: &Path) -> Result<PathBuf> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    let entries = fs::read_dir(trips)
        .with_context(|| format!("no trips folder at {}", trips.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_copy = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with("-share.jsonl"));
        let Ok(when) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if is_copy && best.as_ref().map_or(true, |(t, _)| when > *t) {
            best = Some((when, path));
        }
    }
    best.map(|(_, path)| path)
        .context("no anonymous copy found. Keep one by saying yes to sharing when you start a ride.")
}

struct Request {
    file: Option<PathBuf>,
    config: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<Request> {
    let mut request = Request {
        file: None,
        config: None,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--config" => {
                request.config = Some(PathBuf::from(it.next().context("--config needs a path")?))
            }
            other if other.starts_with('-') => bail!("unknown argument {other}. Try --help"),
            other if request.file.is_none() => request.file = Some(PathBuf::from(other)),
            other => bail!("one file at a time, not also {other}"),
        }
    }
    Ok(request)
}

/// Put the bytes the rider saw in a file only this user can read, hand its
/// path to `f`, and remove it afterwards. A copy of a ride still in progress
/// can grow between the summary and the send; this is what keeps the send to
/// what was shown.
fn with_private_copy<T>(dir: &Path, body: &[u8], f: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
    fs::create_dir_all(dir)?;
    let path = dir.join(format!(".sending-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)?;
    file.write_all(body)?;
    drop(file);
    let result = f(&path);
    let _ = fs::remove_file(&path);
    result
}

/// POST the file. The token and address go to curl on its standard input.
/// `-q` comes first so a `.curlrc` of the rider's cannot add a proxy or a trace.
fn send(config: &Config, file: &Path) -> Result<(u16, String)> {
    let plain = config.url.starts_with("http://");
    let mut at = OsString::from("@");
    at.push(file);
    let mut child = Command::new("curl")
        .arg("-q")
        .args(["--silent", "--show-error", "--max-time", "60"])
        .args(["--proto", if plain { "=http" } else { "=https" }])
        .args(["--request", "POST"])
        .args(["--header", "Content-Type: application/x-ndjson"])
        .arg("--data-binary")
        .arg(at)
        .args(["--write-out", "\n%{http_code}", "--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not run curl. Install curl to use share.")?;
    let settings = format!(
        "url = \"{}\"\nheader = \"Authorization: Bearer {}\"\n",
        config.url, config.token
    );
    child
        .stdin
        .take()
        .context("curl has no input")?
        .write_all(settings.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr);
        let why = why.lines().next().unwrap_or("no reason given");
        bail!("could not reach the receiver ({why}). Is this computer online?");
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let (body, code) = text
        .rsplit_once('\n')
        .context("the receiver sent nothing back")?;
    let code: u16 = code.trim().parse().context("the receiver sent nothing back")?;
    Ok((code, body.to_string()))
}

fn outcome(code: u16, body: &str, rows: usize) -> Result<String> {
    let reason = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    match code {
        201 => {
            let counted = serde_json::from_str::<Value>(body)
                .ok()
                .and_then(|v| v.get("rows").and_then(Value::as_u64))
                .map_or(rows as u64, |n| n);
            Ok(format!("Sent {counted} rows. Thank you."))
        }
        401 => bail!("the receiver did not accept your token. Ask whoever sent you share.json."),
        409 => bail!("the receiver already has this ride."),
        413 => bail!("the receiver says this file is too large."),
        422 => bail!("the receiver refused the file: {reason}."),
        429 => bail!("too many sends today. Try again tomorrow."),
        other => bail!("the receiver answered {other}: {reason}"),
    }
}

fn summary(path: &Path, copy: &Copy, config: &Config) -> String {
    let lines = if copy.lines.is_empty() {
        "no train line".to_string()
    } else {
        copy.lines.iter().cloned().collect::<Vec<_>>().join(", ")
    };
    format!(
        "Anonymous copy: {}\n  {} rows, {} bytes, {}\n  Each row is the train line, a position to about 100 meters, and the\n  connection measurement. Not the town, the time, or which connection.\n  Open the file to look first if you like.\n  It would go to {}.",
        path.display(),
        copy.rows,
        copy.body.len(),
        lines,
        host_of(&config.url)
    )
}

fn said_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

pub fn run(args: &[String], support: &Path, input: &mut dyn BufRead) -> Result<()> {
    let request = parse(args)?;
    let config_path = request.config.unwrap_or_else(|| support.join("share.json"));
    let config = load_config(&config_path)?;
    let file = match request.file {
        Some(file) => file,
        None => newest_copy(&support.join("trips"))?,
    };
    let copy = read_copy(&file)?;
    println!("{}", summary(&file, &copy, &config));
    print!("Send it? y/N ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    if !said_yes(&answer) {
        println!("Not sent.");
        return Ok(());
    }
    let (code, body) = with_private_copy(support, &copy.body, |path| send(&config, path))?;
    println!("{}", outcome(code, &body, copy.rows)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("amtrak-share-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn row(lat: f64) -> String {
        format!(
            "{{\"http_code\":200,\"http_s\":0.2,\"lat\":{lat},\"line\":\"Pennsylvanian\",\"lon\":-76.884,\"loss_pct\":0.0,\"rtt_ms\":54.0,\"tone\":\"clear\"}}\n"
        )
    }

    #[test]
    fn only_https_or_loopback_addresses_are_sent_to() {
        assert!(check_url("https://example.test/amtrak/upload").is_ok());
        assert!(check_url("http://127.0.0.1:8765/amtrak/upload").is_ok());
        assert!(check_url("http://localhost/amtrak/upload").is_ok());
        assert!(check_url("http://example.test/amtrak/upload").is_err());
        assert!(check_url("http://127.0.0.1.evil.test/x").is_err());
        assert!(check_url("http://localhost.evil.test/x").is_err());
        assert!(check_url("ftp://example.test/x").is_err());
    }

    #[test]
    fn an_address_cannot_hide_its_host_behind_a_user_name() {
        assert!(check_url("https://example.test@evil.test/amtrak/upload").is_err());
        assert!(check_url("http://127.0.0.1:x@nothing.invalid:9/amtrak/upload").is_err());
    }

    #[test]
    fn an_address_cannot_smuggle_a_second_curl_setting() {
        assert!(check_url("https://a.test/\"\nheader = \"x").is_err());
        assert!(check_url("https://a.test/ x").is_err());
        assert!(check_url("https://a.test/\\x").is_err());
    }

    #[test]
    fn a_token_cannot_smuggle_a_second_curl_setting() {
        assert!(check_token("abcdefghijklmnopqrstuvwxyz0123456789_-AB").is_ok());
        assert!(check_token("short").is_err());
        assert!(check_token("abcdefghijklmnopqrstuvwxyz\"\nurl = \"x").is_err());
        assert!(check_token(&"a".repeat(129)).is_err());
    }

    #[test]
    fn host_is_what_the_rider_is_told() {
        assert_eq!(host_of("https://example.test/amtrak/upload"), "example.test");
        assert_eq!(host_of("http://127.0.0.1:8765/amtrak/upload"), "127.0.0.1:8765");
    }

    #[test]
    fn a_config_other_users_can_read_is_refused() {
        let dir = temp("perm");
        let path = dir.join("share.json");
        let json = "{\"url\":\"https://a.test/amtrak/upload\",\"token\":\"abcdefghijklmnopqrstuvwxyz0123456789\"}";
        fs::write(&path, json).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let err = load_config(&path).err().unwrap().to_string();
        assert!(err.contains("chmod 600"), "{err}");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let config = load_config(&path).unwrap();
        assert_eq!(config.url, "https://a.test/amtrak/upload");
    }

    #[test]
    fn a_copy_is_counted_and_a_full_trip_file_is_not_one() {
        let dir = temp("copy");
        let good = dir.join("a-share.jsonl");
        fs::write(&good, format!("{}{}", row(40.1), row(40.2))).unwrap();
        let copy = read_copy(&good).unwrap();
        assert_eq!(copy.rows, 2);
        assert!(copy.lines.contains("Pennsylvanian"));
        let trip = dir.join("a.jsonl");
        fs::write(&trip, "{\"at\":\"x\",\"event\":\"started\",\"line\":null}\n").unwrap();
        assert!(read_copy(&trip).is_err());
        let town = dir.join("t-share.jsonl");
        let with_town = row(40.1).replace("}\n", ",\"place\":\"Harrisburg, PA\"}\n");
        fs::write(&town, with_town).unwrap();
        assert!(read_copy(&town).is_err());
        let missing = dir.join("m-share.jsonl");
        fs::write(&missing, row(40.1).replace("\"tone\":\"clear\"", "\"x\":\"clear\"")).unwrap();
        assert!(read_copy(&missing).is_err());
        let empty = dir.join("e-share.jsonl");
        fs::write(&empty, "\n").unwrap();
        assert!(read_copy(&empty).is_err());
    }

    #[test]
    fn the_newest_copy_is_found_and_the_full_trip_is_never_chosen() {
        let dir = temp("newest");
        fs::write(dir.join("old-share.jsonl"), row(40.0)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        fs::write(dir.join("new-share.jsonl"), row(41.0)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        fs::write(dir.join("newest-trip.jsonl"), "{}\n").unwrap();
        assert_eq!(newest_copy(&dir).unwrap(), dir.join("new-share.jsonl"));
    }

    #[test]
    fn arguments_take_one_file_and_a_config() {
        let args: Vec<String> = ["f.jsonl", "--config", "/x/share.json"].map(String::from).to_vec();
        let request = parse(&args).unwrap();
        assert_eq!(request.file, Some(PathBuf::from("f.jsonl")));
        assert_eq!(request.config, Some(PathBuf::from("/x/share.json")));
        assert!(parse(&["--token".to_string(), "x".to_string()]).is_err());
        assert!(parse(&["a".to_string(), "b".to_string()]).is_err());
    }

    #[test]
    fn the_receivers_answers_become_plain_sentences() {
        assert_eq!(outcome(201, "{\"ok\":true,\"rows\":3}", 3).unwrap(), "Sent 3 rows. Thank you.");
        assert_eq!(outcome(201, "{\"ok\":true,\"rows\":5}", 3).unwrap(), "Sent 5 rows. Thank you.");
        assert_eq!(outcome(201, "", 3).unwrap(), "Sent 3 rows. Thank you.");
        let refused = outcome(422, "{\"ok\":false,\"error\":\"row 2: tone\"}", 3).err().unwrap();
        assert!(refused.to_string().contains("row 2: tone"));
        assert!(outcome(401, "", 3).err().unwrap().to_string().contains("token"));
        assert!(outcome(409, "", 3).err().unwrap().to_string().contains("already"));
        assert!(outcome(429, "", 3).err().unwrap().to_string().contains("tomorrow"));
        assert!(outcome(500, "oops", 3).err().unwrap().to_string().contains("500"));
    }

    #[test]
    fn what_is_sent_is_what_was_read_even_if_the_file_grows_after() {
        let dir = temp("snapshot");
        let file = dir.join("a-share.jsonl");
        fs::write(&file, row(40.1)).unwrap();
        let copy = read_copy(&file).unwrap();
        fs::write(&file, format!("{}{}", row(40.1), row(40.2))).unwrap();
        let seen = with_private_copy(&dir, &copy.body, |path| {
            assert_eq!(fs::metadata(path)?.permissions().mode() & 0o777, 0o600);
            Ok(fs::read(path)?)
        })
        .unwrap();
        assert_eq!(seen, row(40.1).into_bytes());
        assert_eq!(copy.rows, 1);
        let leftovers = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".sending-"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn only_a_yes_sends() {
        for yes in ["y", "Y", " yes\n", "YES"] {
            assert!(said_yes(yes), "{yes}");
        }
        for no in ["", "n", "\n", "maybe", "yep"] {
            assert!(!said_yes(no), "{no}");
        }
    }

    #[test]
    fn the_summary_names_the_host_and_what_a_row_holds() {
        let dir = temp("summary");
        let file = dir.join("a-share.jsonl");
        fs::write(&file, row(40.1)).unwrap();
        let copy = read_copy(&file).unwrap();
        let config = Config {
            url: "https://example.test/amtrak/upload".into(),
            token: "t".repeat(30),
        };
        let text = summary(&file, &copy, &config);
        assert!(text.contains("example.test"));
        assert!(text.contains("Pennsylvanian"));
        assert!(text.contains("Not the town, the time, or which connection"));
        assert!(!text.contains(&config.token));
    }
}
