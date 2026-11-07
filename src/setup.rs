//! `ari setup` / `ari uninstall`: wire ARI into Claude Code's statusLine and back out, exactly.

use crate::app::App;
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

/// The config without what `plan` added: the original statusline in `chain.commands`, and a
/// `hide` it wrote (also the older list with `time` and `cache`). `None` when nothing changes.
pub fn unchain(cfg: &Config, original: &Value) -> Option<Config> {
    let mut c = cfg.clone();
    if let Some(cmd) = original.get("command").and_then(Value::as_str) {
        c.chain.commands.retain(|x| x != cmd);
    }
    // `plan` writes `hide` only while chaining, so only a removed chain takes it along.
    if c.chain.commands.len() < cfg.chain.commands.len()
        && (c.hide == CHAINED_HIDE || c.hide == ["cost", "tokens", "time", "cache"])
    {
        c.hide.clear();
    }
    (c != *cfg).then_some(c)
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
    // A config.toml that doesn't parse is left as it is.
    let unchained = config::load_checked()
        .ok()
        .filter(|_| changes && !has("--purge"))
        .and_then(|c| Some((unchain(&c, &original)?, c)));
    if changes {
        println!(
            "settings.json  statusLine\n- {}\n+ {}\n",
            compact(&current),
            compact(&original)
        );
    }
    if let Some((new, old)) = &unchained {
        println!(
            "config.toml  chain.commands, hide\n- {:?}, {:?}\n+ {:?}, {:?}\n",
            old.chain.commands, old.hide, new.chain.commands, new.hide
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
        if let Some((new, _)) = &unchained
            && let Err(e) = config::save(new)
        {
            eprintln!("ari: couldn't update config.toml: {e}");
        }
    }
    if has("--purge") {
        for d in [paths::state_root(), Some(cfg_dir)].into_iter().flatten() {
            let _ = std::fs::remove_dir_all(d);
        }
        println!("Deleted ARI state and config.");
    }
    std::process::exit(0)
}

// ── Codex and DeepSeek Harness hook files ───────────────────────────────────

/// Codex `hooks.json` events (codex-rs/protocol/src/hooks.rs `HookEventsToml`).
const CODEX_EVENTS: [&str; 11] = [
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "PreCompact",
    "PostCompact",
    "Stop",
    "SubagentStart",
    "SubagentStop",
];
/// The events dsh's Claude Code hooks bridge fires.
const DSH_EVENTS: [&str; 7] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Stop",
    "SubagentStart",
    "SubagentStop",
];

fn hook_cmd(app: App) -> String {
    format!(
        "command -v ari >/dev/null 2>&1 && ari hook --agent {} || true",
        app.as_str()
    )
}

/// The event map: `hooks` of a settings-style file, or the root of a bare event map.
fn event_map(root: &mut Map<String, Value>) -> Result<&mut Map<String, Value>, String> {
    let bare = !root.contains_key("hooks")
        && root
            .keys()
            .any(|k| CODEX_EVENTS.contains(&k.as_str()) || k == "Notification");
    if bare {
        return Ok(root);
    }
    root.entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("`hooks` is not a JSON object".into())
}

fn commands(groups: &Value) -> impl Iterator<Item = &str> {
    groups
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| g.get("hooks")?.as_array())
        .flatten()
        .filter_map(|h| h.get("command")?.as_str())
}

/// Every hook command in a hooks file (settings-style or bare).
pub fn all_commands(text: &str) -> Vec<String> {
    let Ok(Value::Object(mut root)) = serde_json::from_str::<Value>(text) else {
        return vec![];
    };
    let Ok(map) = event_map(&mut root) else {
        return vec![];
    };
    map.values().flat_map(commands).map(String::from).collect()
}

/// `existing` with ARI's entry for `app` under each of its events (missing or empty: a new
/// file). Events that already run ARI are left alone. A file that isn't a hooks object is `Err`.
pub fn add_hooks(existing: Option<&str>, app: App) -> Result<String, String> {
    let text = existing.filter(|t| !t.trim().is_empty()).unwrap_or("{}");
    let mut root = match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(m)) => m,
        _ => return Err("not a JSON object; fix it first, ARI won't touch it".into()),
    };
    let cmd = hook_cmd(app);
    let map = event_map(&mut root)?;
    let events: &[&str] = if app == App::Codex {
        &CODEX_EVENTS
    } else {
        &DSH_EVENTS
    };
    for e in events {
        let groups = map.entry(*e).or_insert_with(|| json!([]));
        if commands(groups).any(|c| c == cmd) {
            continue;
        }
        groups
            .as_array_mut()
            .ok_or(format!(
                "`{e}` is not a list; fix it first, ARI won't touch it"
            ))?
            .push(json!({"hooks": [{"type": "command", "command": cmd, "timeout": 1}]}));
    }
    Ok(serde_json::to_string_pretty(&Value::Object(root)).map_err(|e| e.to_string())? + "\n")
}

/// `existing` without commands that run `ari hook --agent <app>`; groups and events left empty
/// by that go too. `None` when there was nothing to remove (or the file doesn't parse).
pub fn remove_hooks(existing: &str, app: App) -> Option<String> {
    let mark = format!("ari hook --agent {}", app.as_str());
    let Ok(Value::Object(mut root)) = serde_json::from_str::<Value>(existing) else {
        return None;
    };
    let map = event_map(&mut root).ok()?;
    let mut removed = false;
    map.retain(|_, groups| {
        let Some(list) = groups.as_array_mut() else {
            return true;
        };
        let before = list.len();
        list.retain_mut(|g| {
            let Some(hs) = g.get_mut("hooks").and_then(Value::as_array_mut) else {
                return true;
            };
            let n = hs.len();
            hs.retain(|h| !h["command"].as_str().is_some_and(|c| c.contains(&mark)));
            removed |= hs.len() < n;
            !(hs.is_empty() && n > 0)
        });
        !(list.is_empty() && before > 0)
    });
    removed.then(|| serde_json::to_string_pretty(&Value::Object(root)).unwrap_or_default() + "\n")
}

/// ARI's entries merged into a Codex `hooks.json`.
pub fn codex_add(existing: Option<&str>) -> Result<String, String> {
    add_hooks(existing, App::Codex)
}

/// A Codex `hooks.json` without ARI's commands; `None` when none were there.
pub fn codex_remove(existing: &str) -> Option<String> {
    remove_hooks(existing, App::Codex)
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
    fn unchain_drops_only_what_setup_added() {
        let s: Value =
            serde_json::from_str(r#"{"statusLine":{"type":"command","command":"bash p.sh"}}"#)
                .unwrap();
        let p = plan(&s, &Config::default()).unwrap();
        let c = unchain(&p.config, &p.original).unwrap();
        assert!(c.chain.commands.is_empty() && c.hide.is_empty());
        let mut mine = p.config.clone();
        mine.hide = vec!["cost".into()];
        assert_eq!(unchain(&mine, &p.original).unwrap().hide, ["cost"]);
        let mut legacy = p.config.clone();
        legacy.hide = ["cost", "tokens", "time", "cache"]
            .map(String::from)
            .to_vec();
        assert!(unchain(&legacy, &p.original).unwrap().hide.is_empty());
        assert_eq!(unchain(&Config::default(), &Value::Null), None);
        // Setup only writes `hide` while chaining; a hide the user chose without a chain stays.
        let own = Config {
            hide: CHAINED_HIDE.map(String::from).to_vec(),
            ..Config::default()
        };
        assert_eq!(unchain(&own, &Value::Null), None);
        assert_eq!(unchain(&own, &p.original), None);
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
