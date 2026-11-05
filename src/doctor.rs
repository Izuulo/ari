//! `ari doctor`: is everything wired? Read-only; every problem comes with a fix.

use crate::chain::{self, Outcome};
use crate::{config, paths, sessions, setup};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
    pub fix: Option<String>,
}

fn pass(name: &'static str, detail: impl Into<String>) -> Check {
    Check {
        name,
        ok: true,
        detail: detail.into(),
        fix: None,
    }
}

fn problem(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Check {
    Check {
        name,
        ok: false,
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
    let found = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|d| d.join("ari"))
            .find(|p| p.is_file())
    });
    match found {
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

fn hooks(now_ms: u64) -> Check {
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
        None => problem(
            "hooks",
            "no hook events recorded yet",
            "start a new Claude Code session (or run /reload-plugins in one)",
        ),
    }
}

pub fn checks(now_ms: u64) -> Vec<Check> {
    let claude = claude_dir().unwrap_or_default();
    let settings = json(&claude.join("settings.json")).unwrap_or(Value::Null);
    let mut out = vec![binary(), plugin(&claude, &settings), statusline(&settings)];
    out.extend(chain_checks(&config::load()));
    out.push(hooks(now_ms));
    out
}

pub fn run(_args: &[String]) -> ! {
    let checks = checks(crate::hook::now_ms());
    println!("ARI doctor");
    for c in &checks {
        println!(
            "  {} {:<10} {}",
            if c.ok { '✓' } else { '✗' },
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
