//! `~/.config/ari/config.toml`. Every key optional; any problem falls back to defaults.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const THEMES: [&str; 9] = [
    "default",
    "soft",
    "vivid",
    "colorblind",
    "mono",
    "auto",
    "claude",
    "codex",
    "deepseek",
];
/// Colour roles a theme sets and `[colors]` can override.
pub const ROLES: [&str; 8] = [
    "good",
    "warn",
    "bad",
    "accent",
    "highlight",
    "info",
    "muted",
    "bar",
];
/// Statusline segments `hide` understands.
pub const SEGMENTS: [&str; 11] = [
    "conflicts",
    "now",
    "tasks",
    "agents",
    "errors",
    "tests",
    "ctx",
    "5h",
    "7d",
    "cost",
    "tokens",
];
/// Panel sections `hide_panel` understands (`tasks` is the checklist bar in the header).
pub const SECTIONS: [&str; 6] = ["tasks", "agents", "errors", "usage", "session", "turns"];
/// The colours the settings screen steps through for a role, after `theme`.
pub const COLOR_CYCLE: [&str; 15] = [
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "gray",
    "darkgray",
    "lightred",
    "lightgreen",
    "lightyellow",
    "lightblue",
    "lightmagenta",
    "lightcyan",
];

const HEADER: &str = "# ARI settings. Edit here or press s in ari watch; changes apply live.\n# Every key: https://github.com/Izuulo/ari#config\n\n";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    pub position: Position,
    pub theme: String,
    /// Statusline segments to leave out.
    pub hide: Vec<String>,
    /// Panel sections to leave out.
    pub hide_panel: Vec<String>,
    pub face: bool,
    pub bell: bool,
    pub ascii: bool,
    pub max_width: usize,
    pub retention_days: u64,
    pub chain: Chain,
    pub thresholds: Thresholds,
    /// Role → colour (`"red"`, `"#rrggbb"`, `"114"`), over the theme's.
    pub colors: BTreeMap<String, String>,
    pub prices: BTreeMap<String, Price>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Position {
    #[default]
    Top,
    Bottom,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Chain {
    pub commands: Vec<String>,
    pub timeout_ms: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Thresholds {
    pub ctx_warn: u8,
    pub ctx_crit: u8,
    pub stuck_repeats: u8,
    /// Context % where Claude Code auto-compacts, for the forecast.
    pub compact_pct: u8,
}

/// Dollars per million tokens.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            position: Position::Top,
            theme: "default".into(),
            hide: Vec::new(),
            hide_panel: Vec::new(),
            face: false,
            bell: false,
            ascii: false,
            max_width: 0,
            retention_days: 7,
            chain: Chain::default(),
            thresholds: Thresholds::default(),
            colors: BTreeMap::new(),
            prices: BTreeMap::new(),
        }
    }
}

impl Default for Chain {
    fn default() -> Self {
        Chain {
            commands: Vec::new(),
            timeout_ms: 300,
        }
    }
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            ctx_warn: 60,
            ctx_crit: 80,
            stuck_repeats: 3,
            compact_pct: 90,
        }
    }
}

pub fn path() -> Option<PathBuf> {
    crate::paths::config_root().map(|d| d.join("config.toml"))
}

pub fn parse(text: &str) -> Config {
    toml::from_str(text).unwrap_or_default()
}

/// A missing file is the defaults; an unreadable or broken one is an error naming the line.
pub fn load_from(p: &Path) -> Result<Config, String> {
    match std::fs::read_to_string(p) {
        Ok(t) => toml::from_str(&t).map_err(|e| e.to_string().trim_end().to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn load_checked() -> Result<Config, String> {
    path().map_or(Ok(Config::default()), |p| load_from(&p))
}

pub fn load() -> Config {
    load_checked().unwrap_or_default()
}

fn to_file_text(cfg: &Config) -> Result<String, toml::ser::Error> {
    Ok(HEADER.to_string() + &toml::to_string_pretty(cfg)?)
}

pub fn save_to(p: &Path, cfg: &Config) -> std::io::Result<()> {
    let text = to_file_text(cfg).map_err(std::io::Error::other)?;
    // A dotfile manager's symlink stays a symlink: write where it points.
    let p = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::paths::write_atomic(&p, text.as_bytes())
}

pub fn save(cfg: &Config) -> std::io::Result<()> {
    save_to(
        &path().ok_or_else(|| std::io::Error::other("no HOME"))?,
        cfg,
    )
}

/// Values ARI can't use and quietly replaces with defaults, one line each.
pub fn problems(cfg: &Config) -> Vec<String> {
    use std::str::FromStr;
    let mut out = Vec::new();
    if !THEMES.contains(&cfg.theme.as_str()) {
        out.push(format!("unknown theme \"{}\"", cfg.theme));
    }
    for (role, value) in &cfg.colors {
        if !ROLES.contains(&role.as_str()) {
            out.push(format!("unknown colour role \"{role}\""));
        } else if ratatui::style::Color::from_str(value).is_err() {
            out.push(format!("{role}: can't read colour \"{value}\""));
        }
    }
    for id in &cfg.hide {
        if !SEGMENTS.contains(&id.as_str()) {
            out.push(format!("hide: unknown segment \"{id}\""));
        }
    }
    for id in &cfg.hide_panel {
        if !SECTIONS.contains(&id.as_str()) {
            out.push(format!("hide_panel: unknown section \"{id}\""));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let c = parse("");
        assert_eq!(c, Config::default());
        assert_eq!(c.position, Position::Top);
        assert_eq!((c.retention_days, c.chain.timeout_ms), (7, 300));
        assert_eq!((c.thresholds.ctx_warn, c.thresholds.ctx_crit), (60, 80));
        assert!(!c.face && !c.bell);
    }

    #[test]
    fn partial_override_and_garbage() {
        let c = parse(
            "position = \"bottom\"\nhide = [\"cost\"]\n[chain]\ncommands = [\"x\"]\n[thresholds]\nctx_warn = 50\n",
        );
        assert_eq!(c.position, Position::Bottom);
        assert_eq!(c.hide, vec!["cost"]);
        assert_eq!(c.chain.commands, vec!["x"]);
        assert_eq!(
            (
                c.chain.timeout_ms,
                c.thresholds.ctx_warn,
                c.thresholds.ctx_crit
            ),
            (300, 50, 80)
        );
        assert_eq!(parse("position = 12\n[[[nope"), Config::default());
        let p = parse("[prices]\n\"claude-opus-5-5\" = { input = 5.0, cache_read = 0.5 }\n");
        assert_eq!(p.prices["claude-opus-5-5"].cache_read, 0.5);
    }

    #[test]
    fn round_trip() {
        let mut c = Config::default();
        c.chain.commands = vec!["bash ~/.claude/x.sh".into()];
        c.hide = vec!["cost".into(), "tokens".into()];
        c.theme = "mono".into();
        c.colors.insert("bad".into(), "lightred".into());
        assert_eq!(parse(&toml::to_string_pretty(&c).unwrap()), c);
    }

    #[test]
    fn hide_panel_parses_apart_from_hide() {
        let c = parse("hide = [\"agents\"]\nhide_panel = [\"turns\"]\n");
        assert_eq!(c.hide, ["agents"]);
        assert_eq!(c.hide_panel, ["turns"]);
        assert!(parse("").hide_panel.is_empty());
    }

    #[test]
    fn theme_and_colors_parse() {
        let c = parse("theme = \"soft\"\n[colors]\ngood = \"#5fd787\"\n");
        assert_eq!(c.theme, "soft");
        assert_eq!(c.colors["good"], "#5fd787");
        let d = parse("");
        assert_eq!(d.theme, "default");
        assert!(d.colors.is_empty());
    }

    #[test]
    fn saved_file_has_header_and_parses() {
        let c = Config {
            theme: "vivid".into(),
            hide: vec!["tasks".into()],
            ..Config::default()
        };
        let text = to_file_text(&c).unwrap();
        assert!(text.starts_with("# ARI settings."), "{text}");
        assert_eq!(parse(&text), c);
    }

    #[test]
    fn load_from_and_save_to() {
        let dir = std::env::temp_dir().join(format!("ari-cfg-{}", std::process::id()));
        let p = dir.join("ari").join("config.toml");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load_from(&p), Ok(Config::default()));
        let c = Config {
            bell: true,
            ..Config::default()
        };
        save_to(&p, &c).unwrap();
        assert_eq!(load_from(&p), Ok(c));
        std::fs::write(&p, "face = [[[").unwrap();
        assert!(load_from(&p).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn save_follows_a_symlink() {
        let dir = std::env::temp_dir().join(format!("ari-cfg-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let real = dir.join("dotfiles.toml");
        let link = dir.join("config.toml");
        std::fs::write(&real, "bell = false\n").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let c = Config {
            bell: true,
            ..Config::default()
        };
        save_to(&link, &c).unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(load_from(&real), Ok(c));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn problems_name_each_bad_value() {
        let mut c = Config {
            theme: "neon".into(),
            ..Config::default()
        };
        c.colors.insert("shiny".into(), "red".into());
        c.colors.insert("good".into(), "#zz".into());
        c.hide = vec!["nope".into(), "cost".into()];
        // `usage` is a panel section, not a statusline segment.
        c.hide_panel = vec!["usage".into(), "cost".into()];
        let p = problems(&c);
        assert_eq!(p.len(), 5, "{p:?}");
        for word in [
            "neon",
            "shiny",
            "#zz",
            "nope",
            "hide_panel: unknown section \"cost\"",
        ] {
            assert!(p.iter().any(|l| l.contains(word)), "{word} in {p:?}");
        }
        assert!(problems(&Config::default()).is_empty());
    }

    #[test]
    fn color_names_parse() {
        use std::str::FromStr;
        for name in COLOR_CYCLE.iter().chain(&["#5fd787", "114"]) {
            assert!(
                ratatui::style::Color::from_str(name).is_ok(),
                "{name} should parse"
            );
        }
    }
}
