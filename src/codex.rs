//! Codex numbers from its rollout JSONL (`{"timestamp","type","payload"}` lines):
//! the latest `turn_context` (model) and `token_count` (tokens, limits) records.

use crate::tokens::{Ledger, ModelTokens};
use crate::usage::{Limit, Usage};
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const TAIL: u64 = 256 * 1024;

#[derive(Default, Clone, Debug, PartialEq)]
pub struct Tail {
    pub model: Option<String>,
    pub total: Option<Value>,
    pub last: Option<Value>,
    pub window: Option<u64>,
    pub rate_limits: Option<Value>,
}

/// Latest `turn_context` and `token_count` in the last 256 KiB; None if neither is there.
pub fn read_tail(path: &Path) -> Option<Tail> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let from = len.saturating_sub(TAIL);
    f.seek(SeekFrom::Start(from)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let mut lines = buf.split(|b| *b == b'\n');
    if from > 0 {
        lines.next(); // starts mid-line
    }
    let mut t = Tail::default();
    let mut seen = false;
    for v in lines.filter_map(|l| serde_json::from_slice::<Value>(l).ok()) {
        let p = &v["payload"];
        match (v["type"].as_str(), p["type"].as_str()) {
            (Some("turn_context"), _) => {
                t.model = p["model"].as_str().map(String::from).or(t.model);
                seen = true;
            }
            (Some("event_msg"), Some("token_count")) => {
                if p["info"].is_object() {
                    t.total = p["info"].get("total_token_usage").cloned();
                    t.last = p["info"].get("last_token_usage").cloned();
                    t.window = p["info"]["model_context_window"].as_u64();
                }
                if p["rate_limits"].is_object() {
                    t.rate_limits = p.get("rate_limits").cloned();
                }
                seen = true;
            }
            _ => {}
        }
    }
    seen.then_some(t)
}

pub fn usage(path: &Path) -> Usage {
    let Some(t) = read_tail(path) else {
        return Usage::default();
    };
    let used = t.last.as_ref().and_then(|l| l["total_tokens"].as_u64());
    let limit = |k: &str| {
        let w = t.rate_limits.as_ref()?.get(k)?;
        Some(Limit {
            pct: w["used_percent"].as_f64()?,
            resets_at: w["resets_at"].as_u64().unwrap_or(0),
        })
    };
    Usage {
        model: t.model.clone(),
        ctx_used: used,
        ctx_size: t.window,
        ctx_pct: used
            .zip(t.window.filter(|w| *w > 0))
            .map(|(u, w)| u as f64 / w as f64 * 100.0),
        five_h: limit("primary"),
        seven_d: limit("secondary"),
        ..Default::default()
    }
}

/// Totals so far; None until Codex has reported any tokens.
pub fn ledger(path: &Path) -> Option<Ledger> {
    let t = read_tail(path)?;
    let tot = t.total?;
    let n = |k: &str| tot[k].as_u64().unwrap_or(0);
    let cached = n("cached_input_tokens");
    let m = ModelTokens {
        input: n("input_tokens").saturating_sub(cached),
        cache_write: 0,
        cache_read: cached,
        output: n("output_tokens"),
    };
    let mut l = Ledger {
        input: m.input,
        cache_read: m.cache_read,
        output: m.output,
        ctx: t.last.and_then(|l| l["total_tokens"].as_u64()).unwrap_or(0),
        ..Default::default()
    };
    if let Some(model) = t.model {
        l.last_model = Some(model.clone());
        l.by_model.insert(model, m);
    }
    Some(l)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/codex/rollout.jsonl")
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("ari-codex-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn usage_from_rollout() {
        let u = usage(&fixture());
        assert!((u.ctx_pct.unwrap() - 18.382).abs() < 0.01);
        assert_eq!((u.ctx_used, u.ctx_size), (Some(50000), Some(272000)));
        assert_eq!(u.five_h.unwrap().pct, 12.5);
        assert_eq!(u.seven_d.unwrap().resets_at, 1794400000);
        assert_eq!(u.model.as_deref(), Some("gpt-5.5-codex"));
        assert_eq!(u.cost_usd, None);
        let l = ledger(&fixture()).unwrap();
        assert_eq!(
            (l.input, l.cache_read, l.output, l.ctx),
            (15000, 30000, 5000, 50000)
        );
        assert_eq!(l.by_model["gpt-5.5-codex"].cache_read, 30000);
    }

    #[test]
    fn partial_last_line_is_skipped() {
        let d = tmp("partial");
        let p = d.join("r.jsonl");
        let mut s = std::fs::read_to_string(fixture()).unwrap();
        s.push_str(r#"{"timestamp":"x","type":"event_msg","payload":{"type":"token_cou"#);
        std::fs::write(&p, s).unwrap();
        assert_eq!(usage(&p), usage(&fixture()));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn fresh_session_has_no_numbers() {
        let d = tmp("fresh");
        let p = d.join("r.jsonl");
        let s: Vec<_> = std::fs::read_to_string(fixture())
            .unwrap()
            .lines()
            .take(2)
            .map(String::from)
            .collect();
        std::fs::write(&p, s.join("\n") + "\n").unwrap();
        let u = usage(&p);
        assert_eq!(
            (u.ctx_pct, u.model.as_deref()),
            (None, Some("gpt-5.5-codex"))
        );
        assert_eq!(ledger(&p), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn usage_load_falls_back_to_rollout() {
        let d = tmp("load");
        let meta = serde_json::json!({"agent":"codex","transcript":fixture()});
        std::fs::write(d.join("meta.json"), meta.to_string()).unwrap();
        assert!(crate::usage::load(&d).ctx_pct.is_some());
        std::fs::write(
            d.join("meta.json"),
            meta.to_string().replace("codex", "claude"),
        )
        .unwrap();
        assert_eq!(crate::usage::load(&d), Usage::default());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn tokens_update_uses_rollout_for_codex() {
        let d = tmp("tokens");
        let meta = serde_json::json!({"agent":"codex","transcript":fixture()});
        std::fs::write(d.join("meta.json"), meta.to_string()).unwrap();
        let c = crate::tokens::update(&d, &Default::default(), fixture().to_str());
        assert_eq!((c.main.cache_read, c.main.output), (30000, 5000));
        let _ = std::fs::remove_dir_all(&d);
    }
}
