//! `ari setup` / `ari uninstall`: wire ARI into Claude Code's statusLine and back out, exactly.

use crate::config::{self, Config};
use crate::paths;
use serde_json::{Map, Value, json};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

pub const ARI_CMD: &str = "ari statusline";
const ORIGINAL: &str = "original-statusline.json";
const CHAINED_HIDE: [&str; 2] = ["cost", "tokens"];

pub fn settings_path() -> Option<PathBuf> {
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(d) if !d.is_empty() => Some(PathBuf::from(d).join("settings.json")),
        _ => Some(PathBuf::from(std::env::var_os("HOME")?).join(".claude/settings.json")),
    }
}

fn is_ari(sl: &Value) -> bool {
    sl.get("command")
        .and_then(Value::as_str)
        .is_some_and(|c| c.contains(ARI_CMD))
}

pub struct Plan {
    pub settings: Value,
    pub config: Config,
    pub original: Value,
    pub diff: String,
}

/// `None` when ARI is already the statusline.
pub fn plan(settings: &Value, cfg: &Config) -> Option<Plan> {
    let original = settings.get("statusLine").cloned().unwrap_or(Value::Null);
    if is_ari(&original) {
        return None;
    }
    let mut sl = original.as_object().cloned().unwrap_or_default();
    sl.insert("type".into(), json!("command"));
    sl.insert("command".into(), json!(ARI_CMD));
    sl.entry("refreshInterval").or_insert(json!(2));
    let mut new = settings.as_object().cloned().unwrap_or_default();
    new.insert("statusLine".into(), Value::Object(sl.clone()));

    let mut config = cfg.clone();
    let mut diff = format!(
        "settings.json  statusLine\n- {}\n+ {}\n",
        compact(&original),
        compact(&Value::Object(sl))
    );
    if let Some(cmd) = original
        .get("command")
        .and_then(Value::as_str)
        .filter(|c| !c.trim().is_empty())
        && !config.chain.commands.iter().any(|c| c == cmd)
    {
        config.chain.commands.push(cmd.to_string());
        diff += &format!("config.toml  chain.commands\n+ {cmd:?}\n");
        if config.hide.is_empty() {
            config.hide = CHAINED_HIDE.iter().map(|s| s.to_string()).collect();
            diff += &format!(
                "config.toml  hide (already shown by your statusline)\n+ {:?}\n",
                config.hide
            );
        }
    }
    Some(Plan {
        settings: Value::Object(new),
        config,
        original,
        diff,
    })
}

/// The settings with the saved original statusLine put back (or removed if there was none).
pub fn restore(settings: &Value, original: &Value) -> Value {
    let mut s = settings.as_object().cloned().unwrap_or_default();
    if original.is_null() {
        s.shift_remove("statusLine");
    } else {
        s.insert("statusLine".into(), original.clone());
    }
    Value::Object(s)
}

fn compact(v: &Value) -> String {
    if v.is_null() {
        "(none)".into()
    } else {
        v.to_string()
    }
}

fn read_settings(path: &Path) -> Result<Value, String> {
    match std::fs::read_to_string(path) {
        Ok(t) if t.trim().is_empty() => Ok(Value::Object(Map::new())),
        Ok(t) => match serde_json::from_str::<Value>(&t) {
            Ok(v) if v.is_object() => Ok(v),
            _ => Err(format!(
                "{} is not a JSON object; fix it first, ARI won't touch it",
                path.display()
            )),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Object(Map::new())),
        Err(e) => Err(format!("can't read {}: {e}", path.display())),
    }
}

/// Back up, then write pretty JSON atomically.
fn write_settings(path: &Path, v: &Value) -> std::io::Result<()> {
    if path.exists() {
        let stamp = crate::hook::now_ms() / 1000;
        std::fs::copy(
            path,
            path.with_extension(format!("json.ari-backup-{stamp}")),
        )?;
    } else if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(v).map_err(std::io::Error::other)? + "\n";
    paths::write_atomic(path, text.as_bytes())
}

fn confirm(yes: bool) -> bool {
    if yes {
        return true;
    }
    print!("Apply these changes? [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    matches!(line.trim(), "y" | "Y" | "yes")
}

fn fail(msg: &str) -> ! {
    eprintln!("ari: {msg}");
    std::process::exit(1)
}

pub fn run_setup(args: &[String]) -> ! {
    let has = |f: &str| args.iter().any(|a| a == f);
    let path = settings_path().unwrap_or_else(|| fail("HOME is not set"));
    let settings = read_settings(&path).unwrap_or_else(|e| fail(&e));
    let Some(plan) = plan(&settings, &config::load()) else {
        println!(
            "ARI is already your statusline ({}). Nothing to change.",
            path.display()
        );
        std::process::exit(0)
    };
    println!("{}", plan.diff);
    if has("--dry-run") {
        println!("Dry run: nothing written.");
        std::process::exit(0)
    }
    if !confirm(has("--yes")) {
        println!("Cancelled. Nothing written.");
        std::process::exit(0)
    }
    let cfg_dir = paths::config_root().unwrap_or_else(|| fail("HOME is not set"));
    let saved = std::fs::create_dir_all(&cfg_dir)
        .and_then(|_| std::fs::write(cfg_dir.join(ORIGINAL), plan.original.to_string()))
        .and_then(|_| config::save(&plan.config))
        .and_then(|_| write_settings(&path, &plan.settings));
    if let Err(e) = saved {
        fail(&format!(
            "setup failed, settings left as they were where possible: {e}"
        ));
    }
    println!(
        "Done. ARI's row shows on Claude Code's next statusline refresh.\n\
         Backup: {}.ari-backup-*\n\n\
         If you haven't yet, install the plugin so hooks feed ARI:\n  \
         /plugin marketplace add Izuulo/ari\n  /plugin install ari@ari\n\n\
         Undo any time with: ari uninstall",
        path.display()
    );
    std::process::exit(0)
}

pub fn run_uninstall(args: &[String]) -> ! {
    let has = |f: &str| args.iter().any(|a| a == f);
    let path = settings_path().unwrap_or_else(|| fail("HOME is not set"));
    let settings = read_settings(&path).unwrap_or_else(|e| fail(&e));
    let cfg_dir = paths::config_root().unwrap_or_else(|| fail("HOME is not set"));
    let saved = std::fs::read_to_string(cfg_dir.join(ORIGINAL))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let current = settings.get("statusLine").cloned().unwrap_or(Value::Null);
    let original = match saved {
        Some(o) => o,
        None if is_ari(&current) => Value::Null,
        None => {
            println!(
                "ARI isn't wired into {}. Nothing to restore.",
                path.display()
            );
            Value::Null
        }
    };
    let changes = is_ari(&current);
    if changes {
        println!(
            "settings.json  statusLine\n- {}\n+ {}\n",
            compact(&current),
            compact(&original)
        );
    }
    if has("--purge") {
        println!("--purge: delete ARI state and config:");
        for d in [paths::state_root(), Some(cfg_dir.clone())]
            .into_iter()
            .flatten()
        {
            println!("- {}", d.display());
        }
        println!();
    }
    if !(changes || has("--purge")) {
        std::process::exit(0)
    }
    if !confirm(has("--yes")) {
        println!("Cancelled. Nothing written.");
        std::process::exit(0)
    }
    if changes {
        if let Err(e) = write_settings(&path, &restore(&settings, &original)) {
            fail(&format!("couldn't restore {}: {e}", path.display()));
        }
        let _ = std::fs::remove_file(cfg_dir.join(ORIGINAL));
        println!("Restored your original statusLine.");
    }
    if has("--purge") {
        for d in [paths::state_root(), Some(cfg_dir)].into_iter().flatten() {
            let _ = std::fs::remove_dir_all(d);
        }
        println!("Deleted ARI state and config.");
    }
    std::process::exit(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_chains_and_keeps_keys() {
        let s: Value = serde_json::from_str(
            r#"{"model":"opus","statusLine":{"type":"command","command":"bash ~/.claude/p.sh","padding":2},"z":1}"#,
        )
        .unwrap();
        let p = plan(&s, &Config::default()).unwrap();
        assert_eq!(
            p.settings.to_string(),
            r#"{"model":"opus","statusLine":{"type":"command","command":"ari statusline","padding":2,"refreshInterval":2},"z":1}"#
        );
        assert_eq!(p.config.chain.commands, vec!["bash ~/.claude/p.sh"]);
        assert_eq!(p.config.hide, CHAINED_HIDE);
        assert!(
            config::problems(&p.config).is_empty(),
            "{:?}",
            config::problems(&p.config)
        );
        assert!(plan(&p.settings, &p.config).is_none());
        assert_eq!(restore(&p.settings, &p.original), s);
        assert_eq!(restore(&p.settings, &p.original).to_string(), s.to_string());
    }

    #[test]
    fn plan_without_statusline() {
        let s: Value = serde_json::from_str(r#"{"a":1,"b":2}"#).unwrap();
        let p = plan(&s, &Config::default()).unwrap();
        assert!(p.config.chain.commands.is_empty() && p.config.hide.is_empty());
        assert_eq!(
            restore(&p.settings, &p.original).to_string(),
            r#"{"a":1,"b":2}"#
        );
        let keep = serde_json::from_str::<Value>(
            r#"{"statusLine":{"type":"command","command":"x","refreshInterval":5}}"#,
        )
        .unwrap();
        assert_eq!(
            plan(&keep, &Config::default()).unwrap().settings["statusLine"]["refreshInterval"],
            5
        );
    }
}
