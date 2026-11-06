//! Which app ran a session: Claude Code, Codex or DeepSeek Harness.

use std::path::Path;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum App {
    #[default]
    Claude,
    Codex,
    Dsh,
}

impl App {
    /// Anything but `codex` and `dsh` is Claude Code.
    pub fn parse(s: &str) -> App {
        match s {
            "codex" => App::Codex,
            "dsh" => App::Dsh,
            _ => App::Claude,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            App::Claude => "claude",
            App::Codex => "codex",
            App::Dsh => "dsh",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            App::Claude => "Claude Code",
            App::Codex => "Codex",
            App::Dsh => "DeepSeek",
        }
    }

    pub fn theme(self) -> &'static str {
        match self {
            App::Claude => "claude",
            App::Codex => "codex",
            App::Dsh => "deepseek",
        }
    }
}

/// The app recorded in `dir/meta.json`; Claude Code when absent.
pub fn of(dir: &Path) -> App {
    std::fs::read(dir.join("meta.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("agent")?.as_str().map(App::parse))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for a in [App::Claude, App::Codex, App::Dsh] {
            assert_eq!(App::parse(a.as_str()), a);
        }
        assert_eq!(App::parse("nope"), App::Claude);
        assert_eq!(App::Dsh.label(), "DeepSeek");
        assert_eq!(App::Dsh.theme(), "deepseek");
        assert_eq!(App::Codex.label(), "Codex");
        assert_eq!(App::Claude.label(), "Claude Code");
    }

    #[test]
    fn of_reads_meta_and_defaults_to_claude() {
        let d = std::env::temp_dir().join(format!("ari-app-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(of(&d), App::Claude);
        std::fs::write(d.join("meta.json"), r#"{"cwd":"/p"}"#).unwrap();
        assert_eq!(of(&d), App::Claude);
        std::fs::write(d.join("meta.json"), r#"{"agent":"dsh"}"#).unwrap();
        assert_eq!(of(&d), App::Dsh);
        let _ = std::fs::remove_dir_all(&d);
    }
}
