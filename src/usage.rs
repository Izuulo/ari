//! The statusline payload, reduced to the fields ARI shows. Everything optional.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Usage {
    pub session_id: Option<String>,
    pub transcript_path: Option<String>,
    pub model: Option<String>,
    pub ctx_pct: Option<f64>,
    pub ctx_size: Option<u64>,
    pub ctx_used: Option<u64>,
    pub five_h: Option<Limit>,
    pub seven_d: Option<Limit>,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<u64>,
    pub api_duration_ms: Option<u64>,
    pub lines_added: Option<u64>,
    pub lines_removed: Option<u64>,
}

#[derive(Serialize, Deserialize, Default, Clone, Copy, Debug, PartialEq)]
pub struct Limit {
    pub pct: f64,
    /// Unix epoch seconds.
    pub resets_at: u64,
}

pub fn from_value(v: &Value) -> Usage {
    let s = |p: &str| v.pointer(p).and_then(Value::as_str).map(String::from);
    let f = |p: &str| v.pointer(p).and_then(Value::as_f64);
    let u = |p: &str| v.pointer(p).and_then(Value::as_u64);
    let limit = |name: &str| {
        Some(Limit {
            pct: f(&format!("/rate_limits/{name}/used_percentage"))?,
            resets_at: u(&format!("/rate_limits/{name}/resets_at")).unwrap_or(0),
        })
    };
    Usage {
        session_id: s("/session_id"),
        transcript_path: s("/transcript_path"),
        model: s("/model/display_name").or_else(|| s("/model/id")),
        ctx_pct: f("/context_window/used_percentage"),
        ctx_size: u("/context_window/context_window_size"),
        ctx_used: u("/context_window/total_input_tokens"),
        five_h: limit("five_hour"),
        seven_d: limit("seven_day"),
        cost_usd: f("/cost/total_cost_usd"),
        duration_ms: u("/cost/total_duration_ms"),
        api_duration_ms: u("/cost/total_api_duration_ms"),
        lines_added: u("/cost/total_lines_added"),
        lines_removed: u("/cost/total_lines_removed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn full_payload() {
        let u = from_value(&json!({
            "session_id": "abc", "transcript_path": "/t.jsonl",
            "model": {"id": "claude-opus-5-5", "display_name": "Opus"},
            "cost": {"total_cost_usd": 2.69, "total_duration_ms": 840000, "total_api_duration_ms": 594000,
                     "total_lines_added": 156, "total_lines_removed": 23},
            "context_window": {"total_input_tokens": 142000, "context_window_size": 200000, "used_percentage": 71,
                               "current_usage": null},
            "rate_limits": {"five_hour": {"used_percentage": 62.0, "resets_at": 1738425600},
                            "seven_day": {"used_percentage": 34.5, "resets_at": 1738857600}}
        }));
        assert_eq!(u.model.as_deref(), Some("Opus"));
        assert_eq!(
            (u.ctx_pct, u.ctx_used, u.ctx_size),
            (Some(71.0), Some(142000), Some(200000))
        );
        assert_eq!(
            u.five_h,
            Some(Limit {
                pct: 62.0,
                resets_at: 1738425600
            })
        );
        assert_eq!(u.seven_d.unwrap().pct, 34.5);
        assert_eq!((u.cost_usd, u.api_duration_ms), (Some(2.69), Some(594000)));
    }

    #[test]
    fn missing_rate_limits_stay_none() {
        let u =
            from_value(&json!({"session_id": "abc", "context_window": {"current_usage": null}}));
        assert_eq!((u.five_h, u.seven_d, u.ctx_pct), (None, None, None));
        assert_eq!(from_value(&json!("garbage")), Usage::default());
    }
}
