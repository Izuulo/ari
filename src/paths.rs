use std::path::{Path, PathBuf};

/// `$XDG_STATE_HOME/ari`, else `~/.local/state/ari` (macOS too, see docs/DECISIONS.md).
pub fn state_root() -> Option<PathBuf> {
    xdg_dir("XDG_STATE_HOME", ".local/state")
}

/// `$XDG_CONFIG_HOME/ari`, else `~/.config/ari`.
pub fn config_root() -> Option<PathBuf> {
    xdg_dir("XDG_CONFIG_HOME", ".config")
}

fn xdg_dir(var: &str, fallback: &str) -> Option<PathBuf> {
    let base = match std::env::var_os(var) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(fallback),
    };
    Some(base.join("ari"))
}

/// Session ids come from untrusted stdin and become a path component.
pub fn valid_session_id(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn session_dir(root: &Path, id: &str) -> Option<PathBuf> {
    valid_session_id(id).then(|| root.join("sessions").join(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids() {
        assert!(valid_session_id("dc28075e-e1f6-4439-b938-a813f2efb8fd"));
        assert!(valid_session_id("abc_123"));
        for bad in ["", "../x", "a/b", "a b", ".", "..", "a\0b"] {
            assert!(!valid_session_id(bad), "{bad:?}");
        }
        assert!(!valid_session_id(&"a".repeat(129)));
        assert!(session_dir(Path::new("/r"), "../etc").is_none());
        assert_eq!(
            session_dir(Path::new("/r"), "s1").unwrap(),
            Path::new("/r/sessions/s1")
        );
    }
}
