//! `ari hook`: called synchronously by Claude Code, Codex and dsh on every hook event.
//! Rules: always exit 0, never print, append one line, do nothing heavy.

use crate::{app::App, config, event, notify, paths};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn run(args: &[String]) -> ! {
    std::panic::set_hook(Box::new(|_| {}));
    let app = args
        .windows(2)
        .find(|w| w[0] == "--agent")
        .map_or(App::Claude, |w| App::parse(&w[1]));
    let _ = std::panic::catch_unwind(|| handle(app));
    std::process::exit(0)
}

fn handle(app: App) {
    let mut buf = Vec::new();
    if std::io::stdin().read_to_end(&mut buf).is_err() {
        return;
    }
    let Ok(raw) = serde_json::from_slice::<Value>(&buf) else {
        return;
    };
    let t = now_ms();
    if let Some(p) = std::env::var_os("ARI_RAW_LOG") {
        // Recording mode for fixtures; lines can exceed 4 KB, so record one session at a time.
        let line = serde_json::json!({ "t": t, "p": raw }).to_string() + "\n";
        let _ = append(Path::new(&p), line.as_bytes());
    }
    let Some(ev) = event::normalize(&raw, t) else {
        return;
    };
    let Some(sid) = raw.get("session_id").and_then(Value::as_str) else {
        return;
    };
    let Some(root) = paths::state_root() else {
        return;
    };
    let Some(dir) = paths::session_dir(&root, sid) else {
        return;
    };
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    // Create-only, on any event: sessions that began before ARI was installed still get a cwd.
    write_meta(&dir, &raw, t, app);
    let cfg = config::load();
    if ev.e == "SessionStart" {
        let days = cfg.retention_days.max(1);
        prune(&root, Duration::from_secs(days * 86_400), &dir);
    }
    let _ = append(&dir.join("events.jsonl"), event::to_line(&ev).as_bytes());
    if let Some(n) = notify::check(&dir, &cfg, t) {
        notify::send(&n, &cfg.notify);
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// One `write_all` on an O_APPEND file: lines under 4 KB from concurrent hooks never interleave.
fn append(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(bytes)
}

fn write_meta(dir: &Path, raw: &Value, t: u64, app: App) {
    let meta = serde_json::json!({
        "agent": app.as_str(),
        "cwd": raw.get("cwd"),
        "started": t,
        "transcript": raw.get("transcript_path"),
    });
    if let Ok(mut f) = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join("meta.json"))
    {
        let _ = f.write_all(meta.to_string().as_bytes());
    }
}

/// Remove session dirs whose last activity (events.jsonl mtime, else dir mtime) is older than `max_age`.
pub fn prune(root: &Path, max_age: Duration, keep: &Path) {
    let Ok(entries) = fs::read_dir(root.join("sessions")) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let dir = entry.path();
        if dir == keep {
            continue;
        }
        let modified = fs::metadata(dir.join("events.jsonl"))
            .or_else(|_| entry.metadata())
            .and_then(|m| m.modified());
        if let Ok(m) = modified
            && now.duration_since(m).is_ok_and(|age| age > max_age)
        {
            let _ = fs::remove_dir_all(&dir);
        }
    }
}
