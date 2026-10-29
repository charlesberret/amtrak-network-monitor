//! On Linux, a desktop session can ask the location portal. The portal shows
//! its own allow dialog. A missing session, or a missing portal, leaves the
//! town line waiting. This does not scan Wi-Fi and it does not call a geocoder.
//!
//! The macOS helper is a separate program. This module does nothing there.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Start a background reader when this process is in a desktop session.
/// `every_secs` is how often the portal is asked. It is read at this start.
pub fn start(path: PathBuf, every_secs: u64) {
    #[cfg(target_os = "linux")]
    linux::spawn(path, every_secs);
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (path, every_secs);
    }
}

pub fn fix_line(at: &str, lat: f64, lon: f64, acc_m: f64) -> String {
    format!("{at}\t{lat:.6}\t{lon:.6}\t{acc_m:.0}\t-\t")
}

fn append_fix(path: &Path, line: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if !path.exists() {
        let _ = fs::write(path, "time\tlat\tlon\tacc_m\tspeed_mps\tplace\n");
    }
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(file, "{line}");
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{append_fix, fix_line};
    use ashpd::desktop::location::{Accuracy, LocationProxy};
    use chrono::SecondsFormat;
    use futures_util::StreamExt;
    use std::path::PathBuf;

    pub fn spawn(path: PathBuf, every_secs: u64) {
        if !desktop_session() {
            return;
        }
        let _ = std::thread::Builder::new()
            .name("locate".into())
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                runtime.block_on(run(path, every_secs));
            });
    }

    fn desktop_session() -> bool {
        if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
            return false;
        }
        matches!(
            std::env::var("XDG_SESSION_TYPE").as_deref(),
            Ok("wayland" | "x11")
        )
    }

    async fn run(path: PathBuf, every_secs: u64) {
        let Ok(proxy) = LocationProxy::new().await else {
            return;
        };
        let every = u32::try_from(every_secs).unwrap_or(15);
        let Ok(session) = proxy
            .create_session(Some(40), Some(every), Some(Accuracy::Neighborhood))
            .await
        else {
            return;
        };
        let Ok(mut stream) = proxy.receive_location_updated().await else {
            return;
        };
        let mut starting = true;
        let start = proxy.start(&session, None);
        tokio::pin!(start);
        loop {
            tokio::select! {
                biased;
                update = stream.next() => {
                    let Some(update) = update else { break };
                    write_update(&path, &update);
                }
                result = &mut start, if starting => {
                    starting = false;
                    if result.is_err() {
                        break;
                    }
                }
            }
        }
    }

    fn write_update(path: &std::path::Path, update: &ashpd::desktop::location::Location) {
        let at = chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        let line = fix_line(
            &at,
            update.latitude(),
            update.longitude(),
            update.accuracy(),
        );
        append_fix(path, &line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fix_line_matches_the_reader() {
        let line = fix_line("2026-10-28T18:00:00Z", 40.2624, -76.8841, 80.0);
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(cols.len(), 6);
        assert_eq!(cols[0], "2026-10-28T18:00:00Z");
        assert_eq!(cols[1], "40.262400");
        assert_eq!(cols[3], "80");
        assert_eq!(cols[4], "-");
        assert!(cols[5].trim().is_empty());
        let dir = std::env::temp_dir().join(format!("amtrak-fix-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("location.tsv");
        append_fix(&path, &line);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("time\tlat\tlon\tacc_m\tspeed_mps\tplace\n"));
        assert!(text.contains("40.262400"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
