//! `~/.config/ari/config.toml`. Every key optional; any problem falls back to defaults.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    pub position: Position,
    pub hide: Vec<String>,
    pub face: bool,
    pub bell: bool,
    pub ascii: bool,
    pub max_width: usize,
    pub retention_days: u64,
    pub chain: Chain,
    pub thresholds: Thresholds,
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
            hide: Vec::new(),
            face: true,
            bell: false,
            ascii: false,
            max_width: 0,
            retention_days: 7,
            chain: Chain::default(),
            thresholds: Thresholds::default(),
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
        }
    }
}

pub fn path() -> Option<PathBuf> {
    crate::paths::config_root().map(|d| d.join("config.toml"))
}

pub fn parse(text: &str) -> Config {
    toml::from_str(text).unwrap_or_default()
}

pub fn load() -> Config {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| parse(&t))
        .unwrap_or_default()
}

pub fn save(cfg: &Config) -> std::io::Result<()> {
    let p = path().ok_or_else(|| std::io::Error::other("no HOME"))?;
    std::fs::create_dir_all(p.parent().unwrap())?;
    let text = toml::to_string_pretty(cfg).map_err(std::io::Error::other)?;
    std::fs::write(p, text)
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
        assert!(c.face && !c.bell);
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
        assert_eq!(parse(&toml::to_string_pretty(&c).unwrap()), c);
    }
}
