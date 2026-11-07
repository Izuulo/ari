//! `ari doctor`: is everything wired? Read-only; every problem comes with a fix.

use crate::app::App;
use crate::chain::{self, Outcome};
use crate::{config, paths, sessions, setup};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    /// Neutral: nothing to check (an app that isn't installed).
    pub info: bool,
    pub detail: String,
    pub fix: Option<String>,
}

fn pass(name: &'static str, detail: impl Into<String>) -> Check {
    Check {
        name,
        ok: true,
        info: false,
        detail: detail.into(),
        fix: None,
    }
}

fn info(name: &'static str, detail: impl Into<String>) -> Check {
    Check {
        info: true,
        ..pass(name, detail)
    }
}

fn problem(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Check {
    Check {
        name,
        ok: false,
        info: false,
        detail: detail.into(),
        fix: Some(fix.into()),
    }
}

fn claude_dir() -> Option<PathBuf> {
    setup::settings_path().and_then(|p| p.parent().map(Path::to_path_buf))
}

fn json(p: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(p).ok()?).ok()
}

fn age(ms: u64) -> String {
    match ms / 1000 {
        s @ 0..60 => format!("{s}s ago"),
        s @ 60..3600 => format!("{}m ago", s / 60),
        s @ 3600..86_400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86_400),
    }
}

fn binary() -> Check {
    match setup::which("ari") {
        Some(p) => {
            let version = std::process::Command::new(&p)
                .arg("--version")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();
            pass("binary", format!("{version} · {}", p.display()))
        }
        None => {
            let here = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.display().to_string()));
            problem(
                "binary",
                "`ari` is not on PATH, so hooks stay silent and record nothing",
                format!(
                    "add {} to PATH",
                    here.unwrap_or_else(|| "the folder holding ari".into())
                ),
            )
        }
    }
}

fn plugin(claude: &Path, settings: &Value) -> Check {
    let installed = json(&claude.join("plugins/installed_plugins.json"));
    let key = installed
        .as_ref()
        .and_then(|d| d.get("plugins").or(Some(d)))
        .and_then(Value::as_object)
        .and_then(|m| m.keys().find(|k| k.starts_with("ari@")).cloned());
    let enabled = |k: &str| {
        settings
            .pointer("/enabledPlugins")
            .and_then(|e| e.get(k))
            .and_then(Value::as_bool)
            != Some(false)
    };
    match key {
        Some(k) if enabled(&k) => pass("plugin", format!("{k} installed and enabled")),
        Some(k) => problem(
            "plugin",
            format!("{k} is installed but disabled"),
            format!("/plugin enable {k}"),
        ),
        None if settings
            .get("hooks")
            .is_some_and(|h| h.to_string().contains("ari hook")) =>
        {
            pass(
                "plugin",
                "not installed, but settings.json wires `ari hook` directly",
            )
        }
        None => problem(
            "plugin",
            "not installed, so no hooks feed ARI (tasks, agents and errors stay empty)",
            "/plugin marketplace add Izuulo/ari, then /plugin install ari@ari",
        ),
    }
}

fn statusline(settings: &Value) -> Check {
    match settings
        .pointer("/statusLine/command")
        .and_then(Value::as_str)
    {
        Some(c) if c.contains(setup::ARI_CMD) => pass("statusline", "runs `ari statusline`"),
        Some(_) => problem(
            "statusline",
            "set to another command; ARI's row won't show",
            "ari setup (keeps your current statusline)",
        ),
        None => problem("statusline", "not set", "ari setup"),
    }
}

fn codex() -> Check {
    let path = setup::codex_home().unwrap_or_default().join("hooks.json");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    if setup::all_commands(&text)
        .iter()
        .any(|c| c.contains("ari hook --agent codex"))
    {
        pass(
            "codex",
            "wired (approve with /hooks if events don't arrive)",
        )
    } else {
        problem(
            "codex",
            format!("ARI's hooks are not in {}", path.display()),
            "ari setup",
        )
    }
}

fn dsh() -> Check {
    let patch = setup::dsh_home()
        .unwrap_or_default()
        .join("cordis.patch.yml");
    let target = match setup::dsh_bridge(&std::fs::read_to_string(&patch).unwrap_or_default()) {
        Ok(Some(p)) => PathBuf::from(p),
        Ok(None) => {
            return problem(
                "dsh",
                "its Claude Code hooks bridge isn't mounted, so dsh sends ARI nothing",
                "ari setup",
            );
        }
        Err(e) => {
            return problem(
                "dsh",
                e,
                "make the bridge's configPath an absolute path, then run ari setup",
            );
        }
    };
    let text = std::fs::read_to_string(&target).unwrap_or_default();
    let shown = target.display();
    if setup::all_commands(&text)
        .iter()
        .any(|c| c.contains("ari hook --agent dsh"))
    {
        pass("dsh", format!("wired via {shown}"))
    } else if setup::plain_ari_hook(&text) {
        problem(
            "dsh",
            format!("{shown} runs plain `ari hook`, so dsh sessions show as Claude Code"),
            "change it to `ari hook --agent dsh` there (ari setup leaves that file alone)",
        )
    } else if setup::claude_settings(&target) {
        problem(
            "dsh",
            format!("the bridge reads {shown}, a Claude Code settings file ARI won't add to"),
            "point the bridge's configPath at a hooks.json of its own, then run ari setup",
        )
    } else {
        problem("dsh", format!("{shown} doesn't run ARI"), "ari setup")
    }
}

fn chain_checks(cfg: &config::Config) -> Vec<Check> {
    if cfg.chain.commands.is_empty() {
        return vec![pass("chain", "none (ARI is your only statusline)")];
    }
    let sample = br#"{"session_id":"ari-doctor","model":{"display_name":"Opus"},"cost":{"total_cost_usd":0},"context_window":{"used_percentage":0}}"#;
    let timeout = Duration::from_millis(cfg.chain.timeout_ms);
    cfg.chain
        .commands
        .iter()
        .map(|cmd| {
            let short: String = cmd.chars().take(48).collect::<String>()
                + if cmd.chars().count() > 48 { "…" } else { "" };
            let t = std::time::Instant::now();
            let ms = || t.elapsed().as_millis();
            match chain::probe(cmd, sample, timeout) {
                Outcome::Output(_) => pass("chain", format!("{short} · ok in {}ms", ms())),
                Outcome::Empty => pass(
                    "chain",
                    format!("{short} · ok in {}ms, prints nothing", ms()),
                ),
                Outcome::Failed => problem(
                    "chain",
                    format!("{short} · exits with an error (skipped)"),
                    "run it by hand to see why, or remove it from [chain] in config.toml",
                ),
                Outcome::TimedOut => problem(
                    "chain",
                    format!("{short} · slower than {}ms (skipped)", cfg.chain.timeout_ms),
                    "raise [chain] timeout_ms in config.toml, or speed the command up",
                ),
                Outcome::CantStart => problem(
                    "chain",
                    format!("{short} · can't start"),
                    "check that `sh` is available",
                ),
            }
        })
        .collect()
}

fn hooks(now_ms: u64, apps: &[App]) -> Check {
    let newest = paths::state_root()
        .map(|r| sessions::list(&r))
        .and_then(|l| {
            l.into_iter()
                .filter(|s| s.dir.join("events.jsonl").exists())
                .map(|s| s.last_ms)
                .max()
        });
    match newest {
        Some(t) => pass(
            "hooks",
            format!("last event {}", age(now_ms.saturating_sub(t))),
        ),
        None => {
            let names: Vec<&str> = apps.iter().map(|a| a.label()).collect();
            let fix = match names.as_slice() {
                [] => "install Claude Code, Codex or DeepSeek Harness, then run ari setup".into(),
                [one] => format!("start a new {one} session"),
                [rest @ .., last] => format!("start a new {} or {last} session", rest.join(", ")),
            };
            let reload = if apps.contains(&App::Claude) {
                " (or run /reload-plugins in Claude Code)"
            } else {
                ""
            };
            problem("hooks", "no hook events recorded yet", fix + reload)
        }
    }
}

/// `YYYY-MM-DDTHH:MM:SS(.fff)Z` to epoch ms (days-from-civil; no date crate).
fn iso_ms(s: &str) -> Option<u64> {
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (h, mi, sec) = (n(11..13)?, n(14..16)?, n(17..19)?);
    let ms = match s.get(19..)? {
        "Z" => 0,
        f => format!("{:0<3}", f.strip_prefix('.')?.strip_suffix('Z')?)
            .get(..3)?
            .parse::<i64>()
            .ok()?,
    };
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    u64::try_from(((days * 24 + h) * 60 + mi) * 60_000 + sec * 1000 + ms).ok()
}

/// `dyld[72458]` -> `dyld[…]`, so runs that differ only by PID group together.
fn squash_brackets(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('[') {
        let (head, tail) = rest.split_at(i + 1);
        out.push_str(head);
        let n = tail.bytes().take_while(u8::is_ascii_digit).count();
        if n > 0 && tail[n..].starts_with(']') {
            out.push('…');
            rest = &tail[n..];
        } else {
            rest = tail;
        }
    }
    out + rest
}

/// Failed hook runs since `since_ms`, grouped by (hook, first stderr line); reads each
/// transcript's last 2 MB only.
fn hook_errors(transcripts: &[PathBuf], since_ms: u64) -> Vec<(String, String, u32)> {
    use std::io::{Read, Seek, SeekFrom};
    const TAIL: u64 = 2 << 20;
    let mut groups: Vec<(String, String, u32)> = Vec::new();
    for p in transcripts {
        let Ok(mut f) = std::fs::File::open(p) else {
            continue;
        };
        let len = f.metadata().map_or(0, |m| m.len());
        let cut = len > TAIL;
        if f.seek(SeekFrom::Start(len.saturating_sub(TAIL))).is_err() {
            continue;
        }
        let mut buf = Vec::new();
        let _ = f.read_to_end(&mut buf);
        let text = String::from_utf8_lossy(&buf);
        for line in text.lines().skip(usize::from(cut)) {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let a = &v["attachment"];
            if a["type"] != "hook_non_blocking_error"
                || v["timestamp"]
                    .as_str()
                    .and_then(iso_ms)
                    .is_none_or(|t| t < since_ms)
            {
                continue;
            }
            let hook = a["hookName"].as_str().unwrap_or("?").to_string();
            let err = a["stderr"]
                .as_str()
                .and_then(|s| s.lines().find(|l| !l.trim().is_empty()))
                .map(|l| {
                    l.trim()
                        .trim_start_matches("Failed with non-blocking status code: ")
                })
                .unwrap_or("")
                .to_string();
            let err = squash_brackets(&err);
            match groups.iter_mut().find(|g| g.0 == hook && g.1 == err) {
                Some(g) => g.2 += 1,
                None => groups.push((hook, err, 1)),
            }
        }
    }
    groups
}

fn hook_failures(now_ms: u64) -> Check {
    let mut dirs = paths::state_root()
        .map(|r| sessions::list(&r))
        .unwrap_or_default();
    dirs.truncate(10);
    let files: Vec<PathBuf> = dirs
        .iter()
        .filter_map(|s| {
            let u = crate::usage::load(&s.dir);
            crate::tokens::transcript_for(&s.dir, &u).map(PathBuf::from)
        })
        .collect();
    hook_check(&files, now_ms)
}

fn hook_check(files: &[PathBuf], now_ms: u64) -> Check {
    // Resumed sessions share a transcript; count it once.
    let mut files = files.to_vec();
    files.sort();
    files.dedup();
    let g = hook_errors(&files, now_ms.saturating_sub(24 * 3_600_000));
    if g.is_empty() {
        return pass("hook errors", "none in the last 24h");
    }
    let total: u32 = g.iter().map(|g| g.2).sum();
    let list: Vec<String> = g
        .iter()
        .take(3)
        .map(|(h, e, _)| format!("{h} — {e}"))
        .collect();
    problem(
        "hook errors",
        format!("{total} failed hook runs in 24h: {}", list.join("; ")),
        "a hook (not ARI's) is failing — run its command by hand to see the error",
    )
}

/// config.toml: missing (defaults), unreadable, or values ARI can't use.
fn config_status(exists: bool, loaded: Result<config::Config, String>) -> Check {
    match loaded {
        _ if !exists => pass("config", "none, using defaults"),
        Err(e) => problem(
            "config",
            e,
            "fix that line, or delete the file to use defaults",
        ),
        Ok(c) => match config::problems(&c) {
            p if p.is_empty() => pass("config", "ok"),
            p => problem(
                "config",
                p.join("; "),
                "fix these in config.toml, or press s in ari watch",
            ),
        },
    }
}

pub fn checks(now_ms: u64) -> Vec<Check> {
    let apps = setup::found();
    let mut out = vec![binary()];
    // One line per app; one that isn't installed is neutral, never a problem.
    for app in [App::Claude, App::Codex, App::Dsh] {
        if !apps.contains(&app) {
            out.push(info(app.as_str(), format!("{} not found", app.label())));
            continue;
        }
        match app {
            App::Claude => {
                let claude = claude_dir().unwrap_or_default();
                let settings = json(&claude.join("settings.json")).unwrap_or(Value::Null);
                out.extend([plugin(&claude, &settings), statusline(&settings)]);
            }
            App::Codex => out.push(codex()),
            App::Dsh => out.push(dsh()),
        }
    }
    let exists = config::path().is_some_and(|p| p.exists());
    let loaded = config::load_checked();
    out.push(config_status(exists, loaded.clone()));
    out.extend(chain_checks(&loaded.unwrap_or_default()));
    out.push(hooks(now_ms, &apps));
    out.push(hook_failures(now_ms));
    out
}

pub fn run(_args: &[String]) -> ! {
    let checks = checks(crate::hook::now_ms());
    println!("ARI doctor");
    for c in &checks {
        println!(
            "  {} {:<10} {}",
            match (c.info, c.ok) {
                (true, _) => '·',
                (_, true) => '✓',
                _ => '✗',
            },
            c.name,
            c.detail
        );
        if let Some(fix) = &c.fix {
            println!("  {:<12} → {fix}", "");
        }
    }
    let failed = checks.iter().filter(|c| !c.ok).count();
    if failed == 0 {
        println!("\nAll good.");
        std::process::exit(0)
    }
    println!(
        "\n{failed} problem{} found.",
        if failed == 1 { "" } else { "s" }
    );
    std::process::exit(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_check_reports() {
        let c = config_status(false, Ok(config::Config::default()));
        assert!(c.ok);
        assert_eq!(c.detail, "none, using defaults");
        let c = config_status(true, Ok(config::Config::default()));
        assert!(c.ok);
        assert_eq!(c.detail, "ok");
        let c = config_status(true, Err("expected `=` at line 3".into()));
        assert!(!c.ok);
        assert_eq!(c.detail, "expected `=` at line 3");
        assert_eq!(
            c.fix.as_deref(),
            Some("fix that line, or delete the file to use defaults")
        );
        let bad = config::Config {
            theme: "neon".into(),
            hide: vec!["nope".into()],
            ..config::Config::default()
        };
        let c = config_status(true, Ok(bad));
        assert!(!c.ok);
        assert_eq!(
            c.detail,
            "unknown theme \"neon\"; hide: unknown segment \"nope\""
        );
    }

    const DYLD: &str =
        "dyld[…]: Library not loaded: /opt/homebrew/opt/llhttp/lib/libllhttp.9.3.dylib";

    fn rec(ts: &str) -> String {
        rec_pid(ts, 72458)
    }

    fn rec_pid(ts: &str, pid: u32) -> String {
        let dyld = DYLD.replace('…', &pid.to_string());
        format!(
            r#"{{"type":"attachment","timestamp":"{ts}","attachment":{{"type":"hook_non_blocking_error","hookName":"SessionStart:startup","stderr":"Failed with non-blocking status code: {dyld}\n"}}}}"#
        )
    }

    fn file(name: &str, lines: &[String]) -> PathBuf {
        let p = std::env::temp_dir().join(format!("ari-hookerr-{}-{name}", std::process::id()));
        std::fs::write(&p, lines.join("\n")).unwrap();
        p
    }

    #[test]
    fn iso_parse() {
        assert_eq!(iso_ms("2026-11-05T17:15:36.328Z"), Some(1_793_898_936_328));
        assert_eq!(iso_ms("1970-01-02T00:00:00Z"), Some(86_400_000));
        assert_eq!(iso_ms("nope"), None);
    }

    #[test]
    fn counts_recent_errors() {
        let p = file(
            "a",
            &[
                rec("2026-11-05T17:15:36.328Z"),
                "{}".into(),
                rec("2026-11-05T17:16:00Z"),
            ],
        );
        let since = iso_ms("2026-11-05T00:00:00Z").unwrap();
        let got = hook_errors(&[p], since);
        assert_eq!(got, vec![("SessionStart:startup".into(), DYLD.into(), 2)]);
    }

    #[test]
    fn old_or_missing_is_empty() {
        let p = file("b", &[rec("2026-11-01T00:00:00Z")]);
        let since = iso_ms("2026-11-05T00:00:00Z").unwrap();
        assert!(hook_errors(&[p], since).is_empty());
        assert!(hook_errors(&[PathBuf::from("/nonexistent/x")], since).is_empty());
    }

    #[test]
    fn pids_group_together() {
        let p = file(
            "c",
            &[
                rec_pid("2026-11-05T17:15:36Z", 1),
                rec_pid("2026-11-05T17:16:00Z", 22),
            ],
        );
        let got = hook_errors(&[p], 0);
        assert_eq!(got, vec![("SessionStart:startup".into(), DYLD.into(), 2)]);
    }

    #[test]
    fn check_reports_problem_or_pass() {
        let now = iso_ms("2026-11-05T18:00:00Z").unwrap();
        let p = file(
            "d",
            &[rec("2026-11-05T17:15:36Z"), rec("2026-11-05T17:16:00Z")],
        );
        let c = hook_check(&[p.clone(), p], now);
        assert!(!c.ok);
        assert!(c.detail.starts_with(
            "2 failed hook runs in 24h: SessionStart:startup — dyld[…]: Library not loaded"
        ));
        assert!(c.fix.unwrap().contains("run its command by hand"));
        assert!(hook_check(&[], now).ok);
    }
}
