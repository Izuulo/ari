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

/// `$var` when set, else `~/dir`.
fn home_or(var: &str, dir: &str) -> Option<PathBuf> {
    match std::env::var_os(var) {
        Some(d) if !d.to_string_lossy().trim().is_empty() => Some(PathBuf::from(d)),
        _ => Some(PathBuf::from(std::env::var_os("HOME")?).join(dir)),
    }
}

pub fn settings_path() -> Option<PathBuf> {
    home_or("CLAUDE_CONFIG_DIR", ".claude").map(|d| d.join("settings.json"))
}

pub fn codex_home() -> Option<PathBuf> {
    home_or("CODEX_HOME", ".codex")
}

pub fn dsh_home() -> Option<PathBuf> {
    home_or("DSH_HOME", ".dsh")
}

/// ARI's own hooks file for dsh's bridge.
pub fn dsh_ari_hooks() -> Option<PathBuf> {
    Some(paths::config_root()?.join("dsh-hooks.json"))
}

pub fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// The apps on this machine: their command is on PATH or their home dir exists.
pub fn found() -> Vec<App> {
    let claude = settings_path().and_then(|p| p.parent().map(Path::to_path_buf));
    [
        (App::Claude, claude),
        (App::Codex, codex_home()),
        (App::Dsh, dsh_home()),
    ]
    .into_iter()
    .filter(|(a, d)| which(a.as_str()).is_some() || d.as_ref().is_some_and(|d| d.is_dir()))
    .map(|(a, _)| a)
    .collect()
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
    write_text(
        path,
        &(serde_json::to_string_pretty(v).map_err(std::io::Error::other)? + "\n"),
    )
}

/// Back up to `<path>.ari-backup-<secs>`, then write atomically.
fn write_text(path: &Path, text: &str) -> std::io::Result<()> {
    if path.exists() {
        let mut backup = path.as_os_str().to_owned();
        backup.push(format!(".ari-backup-{}", crate::hook::now_ms() / 1000));
        std::fs::copy(path, backup)?;
    } else if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    paths::write_atomic(path, text.as_bytes())
}

/// The file's text; `None` when it doesn't exist.
fn read_opt(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("can't read {}: {e}", path.display())),
    }
}

/// `~/x` for paths under HOME, for messages.
fn tilde(p: &Path) -> String {
    std::env::var_os("HOME")
        .and_then(|h| p.strip_prefix(h).ok())
        .map_or_else(|| p.display().to_string(), |r| format!("~/{}", r.display()))
}

fn same_json(a: &str, b: &str) -> bool {
    serde_json::from_str::<Value>(a).ok() == serde_json::from_str::<Value>(b).ok()
}

/// Write only when not a dry run and the user agrees.
fn go(dry: bool, yes: bool, app: &str) -> bool {
    if dry {
        return false;
    }
    let ok = confirm(yes);
    if !ok {
        println!("{app}: cancelled, nothing written.");
    }
    ok
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

pub fn run_setup(args: &[String]) -> ! {
    let has = |f: &str| args.iter().any(|a| a == f);
    let (dry, yes) = (has("--dry-run"), has("--yes"));
    let apps = found();
    if apps.is_empty() {
        println!("Found no Claude Code, Codex or DeepSeek Harness (dsh). Nothing to set up.");
        std::process::exit(0)
    }
    let mut ok = true;
    for app in apps {
        let done = match app {
            App::Claude => setup_claude(dry, yes),
            App::Codex => setup_codex(dry, yes),
            App::Dsh => setup_dsh(dry, yes),
        };
        // A broken file is explained and skipped; the other apps still get set up.
        if let Err(e) = done {
            eprintln!("ari: {e}");
            ok = false;
        }
    }
    if dry {
        println!("Dry run: nothing written.");
    }
    std::process::exit(if ok { 0 } else { 1 })
}

fn setup_claude(dry: bool, yes: bool) -> Result<(), String> {
    let path = settings_path().ok_or("HOME is not set")?;
    let settings = read_settings(&path)?;
    let Some(plan) = plan(&settings, &config::load()) else {
        println!(
            "ARI is already your statusline ({}). Nothing to change.",
            path.display()
        );
        return Ok(());
    };
    println!("{}", plan.diff);
    if !go(dry, yes, "Claude Code") {
        return Ok(());
    }
    let cfg_dir = paths::config_root().ok_or("HOME is not set")?;
    std::fs::create_dir_all(&cfg_dir)
        .and_then(|_| std::fs::write(cfg_dir.join(ORIGINAL), plan.original.to_string()))
        .and_then(|_| config::save(&plan.config))
        .and_then(|_| write_settings(&path, &plan.settings))
        .map_err(|e| format!("setup failed, settings left as they were where possible: {e}"))?;
    println!(
        "Done. ARI's row shows on Claude Code's next statusline refresh.\n\
         Backup: {}.ari-backup-*\n\n\
         If you haven't yet, install the plugin so hooks feed ARI:\n  \
         /plugin marketplace add Izuulo/ari\n  /plugin install ari@ari\n\n\
         Undo any time with: ari uninstall",
        path.display()
    );
    Ok(())
}

fn setup_codex(dry: bool, yes: bool) -> Result<(), String> {
    let path = codex_home().ok_or("HOME is not set")?.join("hooks.json");
    let old = read_opt(&path)?;
    let new = codex_add(old.as_deref()).map_err(|e| format!("{}: {e}", tilde(&path)))?;
    if old.as_deref().is_some_and(|o| same_json(o, &new)) {
        println!(
            "Codex: ARI's hooks are already in {}. Nothing to change.",
            tilde(&path)
        );
        return Ok(());
    }
    println!(
        "{}  hooks\n+ `{}` on {}\n",
        tilde(&path),
        hook_cmd(App::Codex),
        CODEX_EVENTS.join(", ")
    );
    if !go(dry, yes, "Codex") {
        return Ok(());
    }
    write_text(&path, &new).map_err(|e| format!("couldn't write {}: {e}", path.display()))?;
    println!(
        "Codex: hooks added to {}. Open Codex and run /hooks once to approve them.",
        tilde(&path)
    );
    Ok(())
}

/// Claude Code's own settings files: dsh hooks put there would fire in Claude Code too.
pub fn claude_settings(p: &Path) -> bool {
    p.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("settings") && n.ends_with(".json"))
}

/// Some hook in `text` runs plain `ari hook` (sessions recorded as Claude Code).
pub fn plain_ari_hook(text: &str) -> bool {
    all_commands(text)
        .iter()
        .any(|c| c.contains("ari hook") && !c.contains("--agent"))
}

fn setup_dsh(dry: bool, yes: bool) -> Result<(), String> {
    let patch_path = dsh_home()
        .ok_or("HOME is not set")?
        .join("cordis.patch.yml");
    let ours = dsh_ari_hooks().ok_or("HOME is not set")?;
    let ours_text = ours.to_str().ok_or("ARI's config path is not UTF-8")?;
    let in_patch = |e: String| format!("{}: {e}", tilde(&patch_path));
    let patch = read_opt(&patch_path)?;
    let new_patch = dsh_add(patch.as_deref(), ours_text).map_err(in_patch)?;
    let target = dsh_bridge(&new_patch)
        .map_err(in_patch)?
        .map_or_else(|| ours.clone(), PathBuf::from);
    let old = read_opt(&target)?;
    if target != ours && claude_settings(&target) {
        println!(
            "dsh: its hooks bridge reads {}, a Claude Code settings file; ARI won't add dsh hooks \
             there (Claude Code would run them too). Left alone.",
            tilde(&target)
        );
        return Ok(());
    }
    if target != ours && old.as_deref().is_some_and(plain_ari_hook) {
        println!(
            "dsh: {} already runs `ari hook` (its sessions show as Claude Code). Left alone.",
            tilde(&target)
        );
        return Ok(());
    }
    let new =
        add_hooks(old.as_deref(), App::Dsh).map_err(|e| format!("{}: {e}", tilde(&target)))?;
    let hooks_change = !old.as_deref().is_some_and(|o| same_json(o, &new));
    let patch_change = patch.as_deref() != Some(new_patch.as_str());
    if !(hooks_change || patch_change) {
        println!(
            "dsh: ARI's hooks are already in {}. Nothing to change.",
            tilde(&target)
        );
        return Ok(());
    }
    if patch_change {
        println!("{}  (dsh home patch)", tilde(&patch_path));
        for l in new_patch.lines().skip_while(|l| *l != BEGIN) {
            println!("+ {l}");
        }
        println!();
    }
    if hooks_change {
        println!(
            "{}  hooks\n+ `{}` on {}\n",
            tilde(&target),
            hook_cmd(App::Dsh),
            DSH_EVENTS.join(", ")
        );
    }
    if !go(dry, yes, "dsh") {
        return Ok(());
    }
    let write = |p: &Path, t: &str| {
        write_text(p, t).map_err(|e| format!("couldn't write {}: {e}", p.display()))
    };
    // Hooks file first, so the bridge never points at a missing file.
    if hooks_change {
        write(&target, &new)?;
    }
    if patch_change {
        write(&patch_path, &new_patch)?;
    }
    println!(
        "dsh: hooks added to {}. They run from dsh's next start.",
        tilde(&target)
    );
    Ok(())
}

pub fn run_uninstall(args: &[String]) -> ! {
    let has = |f: &str| args.iter().any(|a| a == f);
    let (yes, purge) = (has("--yes"), has("--purge"));
    let apps = found();
    let mut ok = true;
    for done in [
        apps.contains(&App::Codex).then(|| uninstall_codex(yes)),
        apps.contains(&App::Dsh).then(|| uninstall_dsh(yes)),
        (apps.contains(&App::Claude) || purge).then(|| uninstall_claude(yes, purge)),
    ]
    .into_iter()
    .flatten()
    {
        if let Err(e) = done {
            eprintln!("ari: {e}");
            ok = false;
        }
    }
    std::process::exit(if ok { 0 } else { 1 })
}

/// A hooks file ARI can take its entries out of: empty, or a JSON object.
fn editable(path: &Path, text: &str) -> Result<(), String> {
    match serde_json::from_str::<Value>(text) {
        _ if text.trim().is_empty() => Ok(()),
        Ok(Value::Object(_)) => Ok(()),
        _ => Err(format!(
            "{}: not a JSON object, so ARI can't take its hooks out; fix it, then run ari uninstall again",
            tilde(path)
        )),
    }
}

fn uninstall_codex(yes: bool) -> Result<(), String> {
    let path = codex_home().ok_or("HOME is not set")?.join("hooks.json");
    let Some(old) = read_opt(&path)? else {
        return Ok(());
    };
    editable(&path, &old)?;
    let Some(new) = codex_remove(&old) else {
        return Ok(());
    };
    println!("{}  hooks\n- `{}`\n", tilde(&path), hook_cmd(App::Codex));
    if !go(false, yes, "Codex") {
        return Ok(());
    }
    // A file left with no hooks at all was ARI's alone.
    let empty = [json!({}), json!({"hooks": {}})]
        .contains(&serde_json::from_str(&new).unwrap_or(Value::Null));
    if empty {
        std::fs::remove_file(&path)
    } else {
        write_text(&path, &new)
    }
    .map_err(|e| format!("couldn't update {}: {e}", path.display()))?;
    println!("Codex: ARI's hooks removed from {}.", tilde(&path));
    Ok(())
}

fn uninstall_dsh(yes: bool) -> Result<(), String> {
    let patch_path = dsh_home()
        .ok_or("HOME is not set")?
        .join("cordis.patch.yml");
    let ours = dsh_ari_hooks().ok_or("HOME is not set")?;
    let ours_text = ours.to_str().ok_or("ARI's config path is not UTF-8")?;
    let patch = read_opt(&patch_path)?;
    let new_patch = patch.as_deref().and_then(dsh_remove);
    // What dsh would read once ARI's block is gone.
    let left = new_patch.as_deref().or(patch.as_deref()).unwrap_or("");
    let bridge = dsh_bridge(left).ok().flatten();
    if bridge.as_deref() == Some(ours_text) || left.contains(ours_text) {
        return Err(format!(
            "{} still mounts dsh's hooks bridge on {} outside ARI's markers; left both alone. \
             Remove that row, then run ari uninstall again.",
            tilde(&patch_path),
            tilde(&ours)
        ));
    }
    // The user's own bridge file: take ARI's entries out of it.
    let theirs = match bridge.map(PathBuf::from) {
        Some(p) => match read_opt(&p)? {
            Some(t) => {
                editable(&p, &t)?;
                remove_hooks(&t, App::Dsh).map(|new| (p, new))
            }
            None => None,
        },
        None => None,
    };
    let ours_exists = ours.exists();
    if new_patch.is_none() && theirs.is_none() && !ours_exists {
        return Ok(());
    }
    if new_patch.is_some() {
        println!(
            "{}  (dsh home patch)\n- ARI's hooks bridge row\n",
            tilde(&patch_path)
        );
    }
    if ours_exists {
        println!("{}\n- ARI's hooks file for dsh\n", tilde(&ours));
    }
    if let Some((p, _)) = &theirs {
        println!("{}  hooks\n- `{}`\n", tilde(p), hook_cmd(App::Dsh));
    }
    if !go(false, yes, "dsh") {
        return Ok(());
    }
    let fail = |p: &Path, e: std::io::Error| format!("couldn't update {}: {e}", p.display());
    // Patch first, so the bridge never points at a removed file.
    match new_patch.as_deref() {
        Some("") => std::fs::remove_file(&patch_path).map_err(|e| fail(&patch_path, e))?,
        Some(t) => write_text(&patch_path, t).map_err(|e| fail(&patch_path, e))?,
        None => {}
    }
    if ours_exists {
        std::fs::remove_file(&ours).map_err(|e| fail(&ours, e))?;
    }
    if let Some((p, new)) = theirs {
        write_text(&p, &new).map_err(|e| fail(&p, e))?;
    }
    println!("dsh: ARI's hooks removed.");
    Ok(())
}

fn uninstall_claude(yes: bool, purge: bool) -> Result<(), String> {
    let path = settings_path().ok_or("HOME is not set")?;
    let settings = read_settings(&path)?;
    let cfg_dir = paths::config_root().ok_or("HOME is not set")?;
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
        .filter(|_| changes && !purge)
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
    if purge {
        println!("--purge: delete ARI state and config:");
        for d in [paths::state_root(), Some(cfg_dir.clone())]
            .into_iter()
            .flatten()
        {
            println!("- {}", d.display());
        }
        println!();
    }
    if !(changes || purge) {
        return Ok(());
    }
    if !confirm(yes) {
        println!("Cancelled. Nothing written.");
        return Ok(());
    }
    if changes {
        write_settings(&path, &restore(&settings, &original))
            .map_err(|e| format!("couldn't restore {}: {e}", path.display()))?;
        let _ = std::fs::remove_file(cfg_dir.join(ORIGINAL));
        println!("Restored your original statusLine.");
        if let Some((new, _)) = &unchained
            && let Err(e) = config::save(new)
        {
            eprintln!("ari: couldn't update config.toml: {e}");
        }
    }
    if purge {
        for d in [paths::state_root(), Some(cfg_dir)].into_iter().flatten() {
            let _ = std::fs::remove_dir_all(d);
        }
        println!("Deleted ARI state and config.");
    }
    Ok(())
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

// dsh reads the home-level patch `$DSH_HOME/cordis.patch.yml` (a YAML list of rows) after every
// profile layer (deepseek-harness apps/cli/README.md "Profiles"); ARI adds one delimited `insert`.
const BRIDGE: &str = "@deepseek-ai/dsh-hooks-claude-code";
const BEGIN: &str = "# >>> ari: added by `ari setup`, removed by `ari uninstall`";
const END: &str = "# <<< ari";
const WAS_EMPTY: &str = "# (this file was `[]`)";

fn is_code(l: &str) -> bool {
    let t = l.trim();
    !t.is_empty() && !t.starts_with('#')
}

/// ARI's Claude-shaped hooks file that dsh's bridge runs.
pub fn dsh_hooks() -> String {
    add_hooks(None, App::Dsh).unwrap_or_default()
}

/// The `configPath` of the row in a dsh patch that mounts the Claude Code hooks bridge (ARI's own
/// or the user's). `Ok(None)`: no such row. `Err`: a path ARI can't follow (relative paths resolve
/// from wherever dsh was launched).
// shortcut: reads the bridge's row as text, not YAML (the first `configPath:` between the row's
// `-` and the next line at that indent or less); upgrade if people write the row some other way.
pub fn dsh_bridge(patch: &str) -> Result<Option<String>, String> {
    let indent = |l: &str| l.len() - l.trim_start_matches(' ').len();
    let dash = |l: &str| l.trim_start() == "-" || l.trim_start().starts_with("- ");
    let code: Vec<&str> = patch.lines().filter(|l| is_code(l)).collect();
    let Some(n) = code.iter().position(|l| l.contains(BRIDGE)) else {
        return Ok(None);
    };
    // The row's keys sit at `key`; the row starts at the nearest `-` left of them.
    let key = indent(code[n]) + if dash(code[n]) { 2 } else { 0 };
    let start = (0..=n)
        .rev()
        .find(|&i| dash(code[i]) && indent(code[i]) < key)
        .unwrap_or(n);
    let end = (start + 1..code.len())
        .find(|&i| indent(code[i]) <= indent(code[start]))
        .unwrap_or(code.len());
    let raw = code[start..end]
        .iter()
        .find_map(|l| l.split_once("configPath:").map(|(_, v)| v.trim()))
        .ok_or("the dsh hooks bridge has no configPath ARI can read")?;
    let path = if let Some(q) = raw.strip_prefix('\'') {
        let mut out = String::new();
        let mut it = q.chars().peekable();
        while let Some(c) = it.next() {
            match c {
                '\'' if it.peek() == Some(&'\'') => {
                    it.next();
                    out.push('\'');
                }
                '\'' => break,
                c => out.push(c),
            }
        }
        out
    } else if let Some(q) = raw.strip_prefix('"') {
        q.split('"').next().unwrap_or_default().to_string()
    } else {
        let plain = raw.split(" #").next().unwrap_or_default();
        plain
            .split([',', '}'])
            .next()
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    if path.starts_with('/') {
        Ok(Some(path))
    } else {
        Err(format!(
            "the dsh hooks bridge reads configPath {raw}; ARI only follows an absolute path"
        ))
    }
}

/// The patch with a row mounting the bridge on `ari_hooks_path`, in ARI's delimited block.
/// A patch that already mounts the bridge comes back unchanged: ARI's entries go into that
/// row's file instead. A patch that isn't a plain YAML list is `Err`.
pub fn dsh_add(patch: Option<&str>, ari_hooks_path: &str) -> Result<String, String> {
    let text = patch.unwrap_or("");
    if dsh_bridge(text)?.is_some() {
        return Ok(text.to_string());
    }
    let code: Vec<&str> = text.lines().filter(|l| is_code(l)).collect();
    let was_empty = code.len() == 1 && code[0].trim() == "[]";
    if !was_empty && code.first().is_some_and(|l| !l.starts_with('-')) {
        return Err(
            "its list doesn't start at column 0, so ARI's row would break it; ARI won't touch it"
                .into(),
        );
    }
    if !was_empty && code.iter().any(|l| !l.starts_with(['-', ' '])) {
        return Err(
            "not a YAML list of rows ARI can add to; fix it first, ARI won't touch it".into(),
        );
    }
    let mut out = String::new();
    for l in text.lines().filter(|l| !(was_empty && l.trim() == "[]")) {
        out += l;
        out.push('\n');
    }
    out += BEGIN;
    out.push('\n');
    if was_empty {
        out += WAS_EMPTY;
        out.push('\n');
    }
    out += &format!(
        "- insert:\n    - id: ari-hooks\n      name: '{BRIDGE}'\n      config:\n        configPath: '{}'\n{END}\n",
        ari_hooks_path.replace('\'', "''")
    );
    Ok(out)
}

/// The patch without ARI's block; `""` when nothing else was in it. `None` when it had none.
pub fn dsh_remove(patch: &str) -> Option<String> {
    let lines: Vec<&str> = patch.lines().collect();
    let b = lines.iter().position(|l| *l == BEGIN)?;
    let e = b + lines[b..].iter().position(|l| *l == END)?;
    let mut kept = [&lines[..b], &lines[e + 1..]].concat();
    if lines[b..e].contains(&WAS_EMPTY) {
        kept.insert(b, "[]");
    }
    let s = kept.join("\n");
    Some(if s.trim().is_empty() {
        String::new()
    } else {
        s + "\n"
    })
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
