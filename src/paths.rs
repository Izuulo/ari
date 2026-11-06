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

/// Write via a temp file in the same dir + rename, so readers never see a partial file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Complete lines `path` gained past `offset`. `true` when the file got shorter than `offset`:
/// the caller starts over and the lines are read from 0. `None` when nothing changed or the file
/// can't be read; a partial last line waits for the next call.
pub fn new_lines(path: &Path, offset: u64) -> Option<(bool, Vec<u8>)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let restart = len < offset;
    let from = if restart { 0 } else { offset };
    if len == from && !restart {
        return None;
    }
    let mut buf = Vec::new();
    f.seek(SeekFrom::Start(from)).ok()?;
    f.read_to_end(&mut buf).ok()?;
    buf.truncate(buf.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1));
    (restart || !buf.is_empty()).then_some((restart, buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_lines_reads_complete_lines_and_restarts() {
        let p = std::env::temp_dir().join(format!("ari-newlines-{}", std::process::id()));
        let _ = std::fs::remove_file(&p);
        assert_eq!(new_lines(&p, 0), None, "missing file");
        std::fs::write(&p, "a\nb").unwrap();
        assert_eq!(new_lines(&p, 0), Some((false, b"a\n".to_vec())));
        assert_eq!(new_lines(&p, 2), None, "partial line waits");
        std::fs::write(&p, "a\nb\n").unwrap();
        assert_eq!(new_lines(&p, 2), Some((false, b"b\n".to_vec())));
        assert_eq!(new_lines(&p, 4), None, "unchanged");
        std::fs::write(&p, "x\n").unwrap();
        assert_eq!(
            new_lines(&p, 4),
            Some((true, b"x\n".to_vec())),
            "shorter: start over"
        );
        std::fs::write(&p, "").unwrap();
        assert_eq!(
            new_lines(&p, 2),
            Some((true, vec![])),
            "emptied: still a restart"
        );
        let _ = std::fs::remove_file(&p);
    }

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
