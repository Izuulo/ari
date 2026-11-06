//! Token totals from transcripts: main session and each subagent, read incrementally.

use crate::config::Price;
use crate::state::State;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const CACHE_VERSION: u32 = 4;
/// Requests with fewer input-side tokens than this don't move the hit-rate needle.
const MIN_HIT_TOKENS: u64 = 1000;
const RECENT_IDS: usize = 16;
const TOOL_IDS: usize = 32;

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ModelTokens {
    pub input: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Ledger {
    pub offset: u64,
    pub input: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
    pub requests: u32,
    /// One API response spans several transcript entries with the same id (DECISIONS D14).
    pub recent_ids: VecDeque<String>,
    pub last_hit: Option<f64>,
    pub prev_hit: Option<f64>,
    pub by_model: BTreeMap<String, ModelTokens>,
    /// Model of the latest counted response.
    pub last_model: Option<String>,
    /// Latest `away_summary` system record: first line, capped.
    pub away: Option<String>,
    /// Input side (input + cache write + cache read) of the latest request: context in use.
    pub ctx: u64,
    /// Estimate of what filled `ctx`: `base`, `messages`, or a tool name → tokens.
    pub fill: BTreeMap<String, u64>,
    /// Recent tool_use id → tool name, to name the results that follow.
    pub tool_ids: VecDeque<(String, String)>,
    /// Tools whose results arrived since the latest request.
    pub pending: Vec<String>,
}

impl Ledger {
    pub fn total(&self) -> u64 {
        self.input + self.cache_write + self.cache_read + self.output
    }

    pub fn hit(&self) -> Option<f64> {
        let denom = self.input + self.cache_write + self.cache_read;
        (denom > 0).then(|| self.cache_read as f64 / denom as f64)
    }

    fn add(&mut self, other: &Ledger) {
        self.input += other.input;
        self.cache_write += other.cache_write;
        self.cache_read += other.cache_read;
        self.output += other.output;
        self.requests += other.requests;
        for (m, t) in &other.by_model {
            let e = self.by_model.entry(m.clone()).or_default();
            e.input += t.input;
            e.cache_write += t.cache_write;
            e.cache_read += t.cache_read;
            e.output += t.output;
        }
    }

    /// Credit the context's growth to the tool results since the last request (an estimate).
    fn grow(&mut self, side: u64) {
        let pending = std::mem::take(&mut self.pending);
        if self.ctx == 0 || side < self.ctx {
            self.fill = BTreeMap::from([("base".to_string(), side)]);
        } else if pending.is_empty() {
            *self.fill.entry("messages".into()).or_default() += side - self.ctx;
        } else {
            let (d, n) = (side - self.ctx, pending.len() as u64);
            for (i, tool) in pending.into_iter().enumerate() {
                *self.fill.entry(tool).or_default() += d / n + if i == 0 { d % n } else { 0 };
            }
        }
        self.ctx = side;
    }

    fn apply(&mut self, entry: &Value) {
        if entry.get("subtype").and_then(Value::as_str) == Some("away_summary")
            && let Some(l) = entry
                .get("content")
                .and_then(Value::as_str)
                .and_then(|c| c.lines().map(str::trim).find(|l| !l.is_empty()))
        {
            self.away = Some(crate::event::truncate(l, 200));
        }
        let blocks = |m: &Value| {
            m.get("content")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        if entry.get("type").and_then(Value::as_str) == Some("user")
            && let Some(m) = entry.get("message")
        {
            for b in blocks(m) {
                if let Some(id) = b.get("tool_use_id").and_then(Value::as_str)
                    && let Some((_, name)) = self.tool_ids.iter().find(|(i, _)| i == id)
                {
                    self.pending.push(name.clone());
                }
            }
        }
        if entry.get("type").and_then(Value::as_str) != Some("assistant") {
            return;
        }
        let Some(m) = entry.get("message") else {
            return;
        };
        // Before the dedupe: one response's tool_use blocks arrive in later entries (D14).
        for b in blocks(m) {
            if b.get("type").and_then(Value::as_str) == Some("tool_use")
                && let (Some(id), Some(name)) = (
                    b.get("id").and_then(Value::as_str),
                    b.get("name").and_then(Value::as_str),
                )
            {
                self.tool_ids.push_back((id.into(), name.into()));
                if self.tool_ids.len() > TOOL_IDS {
                    self.tool_ids.pop_front();
                }
            }
        }
        let id = m
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if !id.is_empty() {
            if self.recent_ids.contains(&id) {
                return;
            }
            self.recent_ids.push_back(id);
            if self.recent_ids.len() > RECENT_IDS {
                self.recent_ids.pop_front();
            }
        }
        let Some(u) = m.get("usage") else { return };
        let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
        let t = ModelTokens {
            input: n("input_tokens"),
            cache_write: n("cache_creation_input_tokens"),
            cache_read: n("cache_read_input_tokens"),
            output: n("output_tokens"),
        };
        self.input += t.input;
        self.cache_write += t.cache_write;
        self.cache_read += t.cache_read;
        self.output += t.output;
        self.requests += 1;
        let side = t.input + t.cache_write + t.cache_read;
        self.grow(side);
        if side >= MIN_HIT_TOKENS {
            self.prev_hit = self.last_hit;
            self.last_hit = Some(t.cache_read as f64 / side as f64);
        }
        let model = m.get("model").and_then(Value::as_str).unwrap_or("unknown");
        // `<synthetic>` entries follow interrupts and API errors; they're not a model.
        if !model.starts_with('<') {
            self.last_model = Some(model.to_string());
        }
        let e = self.by_model.entry(model.to_string()).or_default();
        e.input += t.input;
        e.cache_write += t.cache_write;
        e.cache_read += t.cache_read;
        e.output += t.output;
    }
}

/// `claude-opus-5-5-20261001` → `opus-5-5`; other providers' names pass through.
pub fn short_model(id: &str) -> String {
    let id = id.strip_prefix("claude-").unwrap_or(id);
    match id.rsplit_once('-') {
        Some((head, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => {
            head.into()
        }
        _ => id.into(),
    }
}

/// A ledger from transcript text already in memory (the bundled demo).
pub fn ledger_from(text: &str) -> Ledger {
    let mut l = Ledger::default();
    for v in text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
    {
        l.apply(&v);
    }
    l
}

/// Read whatever `path` gained since `l.offset`; only complete lines. A shorter file starts over.
pub fn read_ledger(path: &Path, l: &mut Ledger) {
    let Ok(mut f) = std::fs::File::open(path) else {
        return;
    };
    let len = f.metadata().map_or(0, |m| m.len());
    if len < l.offset {
        *l = Ledger::default();
    }
    if len == l.offset || f.seek(SeekFrom::Start(l.offset)).is_err() {
        return;
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return;
    }
    let Some(end) = buf.iter().rposition(|b| *b == b'\n').map(|i| i + 1) else {
        return;
    };
    for line in buf[..end].split(|b| *b == b'\n') {
        if let Ok(v) = serde_json::from_slice::<Value>(line) {
            l.apply(&v);
        }
    }
    l.offset += end as u64;
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct TokenCache {
    pub v: u32,
    pub main: Ledger,
    pub agents: BTreeMap<String, Ledger>,
}

impl TokenCache {
    pub fn total(&self) -> Ledger {
        let mut t = self.main.clone();
        for a in self.agents.values() {
            t.add(a);
        }
        t
    }

    /// Main conversation's hit rate fell from ≥70% to <30% on the latest request.
    pub fn drop_alert(&self) -> Option<(f64, f64)> {
        match (self.main.prev_hit, self.main.last_hit) {
            (Some(p), Some(l)) if p >= 0.7 && l < 0.3 => Some((p, l)),
            _ => None,
        }
    }
}

/// Observed layout: `<transcript without .jsonl>/subagents/agent-<id>.jsonl`.
pub fn subagent_path(main: &str, agent_id: &str) -> String {
    format!(
        "{}/subagents/agent-{agent_id}.jsonl",
        main.strip_suffix(".jsonl").unwrap_or(main)
    )
}

pub fn cached(dir: &Path) -> TokenCache {
    std::fs::read(dir.join("tokens.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<TokenCache>(&b).ok())
        .filter(|c| c.v == CACHE_VERSION)
        .unwrap_or(TokenCache {
            v: CACHE_VERSION,
            ..Default::default()
        })
}

/// Bring `tokens.json` up to date with the main and subagent transcripts.
pub fn update(dir: &Path, st: &State, transcript: Option<&str>) -> TokenCache {
    let mut c = cached(dir);
    let before = c.clone();
    if let Some(main) = transcript {
        read_ledger(Path::new(main), &mut c.main);
    }
    for a in st.agents.iter().filter(|a| a.id != "main") {
        let path = a
            .transcript
            .clone()
            .or_else(|| transcript.map(|m| subagent_path(m, &a.id)));
        if let Some(p) = path {
            read_ledger(Path::new(&p), c.agents.entry(a.id.clone()).or_default());
        }
    }
    c.agents.retain(|_, l| l.offset > 0);
    if c != before
        && let Ok(bytes) = serde_json::to_vec(&c)
    {
        let _ = crate::paths::write_atomic(&dir.join("tokens.json"), &bytes);
    }
    c
}

/// The main transcript: from the statusline payload, else what the hook saw first.
pub fn transcript_for(dir: &Path, u: &crate::usage::Usage) -> Option<String> {
    u.transcript_path.clone().or_else(|| {
        let meta: Value =
            serde_json::from_slice(&std::fs::read(dir.join("meta.json")).ok()?).ok()?;
        meta.get("transcript")?.as_str().map(String::from)
    })
}

/// Milliseconds since the transcript last changed.
pub fn transcript_age(path: &str, now_ms: u64) -> Option<u64> {
    let t = std::fs::metadata(path).ok()?.modified().ok()?;
    let ms = t.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64;
    Some(now_ms.saturating_sub(ms))
}

/// For the statusline: read transcripts at most once per 2 s, otherwise use the cache.
pub fn update_debounced(dir: &Path, st: &State, u: &crate::usage::Usage) -> TokenCache {
    let fresh = std::fs::metadata(dir.join("tokens.json"))
        .and_then(|m| m.modified())
        .is_ok_and(|t| {
            t.elapsed()
                .is_ok_and(|e| e < std::time::Duration::from_secs(2))
        });
    if fresh {
        return cached(dir);
    }
    let c = update(dir, st, transcript_for(dir, u).as_deref());
    // Rewrite even when nothing changed: the file's age is the debounce clock.
    if let Ok(bytes) = serde_json::to_vec(&c) {
        let _ = crate::paths::write_atomic(&dir.join("tokens.json"), &bytes);
    }
    c
}

/// Dollars saved by cache reads versus paying full input price (≈, see DECISIONS D18).
pub fn savings(l: &Ledger, cost_usd: Option<f64>, prices: &BTreeMap<String, Price>) -> Option<f64> {
    if l.cache_read == 0 {
        return None;
    }
    let priced: f64 = l
        .by_model
        .iter()
        .filter_map(|(m, t)| {
            prices
                .get(m)
                .map(|p| t.cache_read as f64 * (p.input - p.cache_read) / 1e6)
        })
        .sum();
    if l.by_model.keys().all(|m| prices.contains_key(m)) && !l.by_model.is_empty() {
        return Some(priced);
    }
    // Anthropic's ratios: output 5×, cache write 1.25×, cache read 0.1× the input price.
    let cost = cost_usd.filter(|c| *c > 0.0)?;
    let weight = l.input as f64
        + 1.25 * l.cache_write as f64
        + 0.1 * l.cache_read as f64
        + 5.0 * l.output as f64;
    (weight > 0.0).then(|| 0.9 * (cost / weight) * l.cache_read as f64)
}

#[cfg(test)]
mod tests {
    #[test]
    fn away_summary_keeps_the_last_first_line() {
        let l = ledger_from(
            "{\"type\":\"system\",\"subtype\":\"away_summary\",\"content\":\"old\"}\n\
             {\"type\":\"system\",\"subtype\":\"away_summary\",\"content\":\"\\nnew one\\nsecond\"}\n",
        );
        assert_eq!(l.away.as_deref(), Some("new one"));
    }

    use super::*;
    use crate::state::Agent;

    /// An assistant entry: message id, input side split as input + cache_read, tool_use blocks.
    fn asst(id: &str, input: u64, cache_read: u64, uses: &[(&str, &str)]) -> String {
        let content: Vec<Value> = uses
            .iter()
            .map(|(i, n)| serde_json::json!({"type": "tool_use", "id": i, "name": n}))
            .collect();
        serde_json::json!({"type": "assistant", "message": {"id": id, "model": "m",
            "usage": {"input_tokens": input, "cache_read_input_tokens": cache_read},
            "content": content}})
        .to_string()
    }
    fn results(ids: &[&str]) -> String {
        let content: Vec<Value> = ids
            .iter()
            .map(|i| serde_json::json!({"type": "tool_result", "tool_use_id": i}))
            .collect();
        serde_json::json!({"type": "user", "message": {"role": "user", "content": content}})
            .to_string()
    }
    fn fill(l: &Ledger) -> Vec<(&str, u64)> {
        l.fill.iter().map(|(k, v)| (k.as_str(), *v)).collect()
    }

    #[test]
    fn fill_starts_with_base() {
        let l = ledger_from(&asst("m1", 100, 900, &[]));
        assert_eq!(l.ctx, 1000);
        assert_eq!(fill(&l), [("base", 1000)]);
    }

    #[test]
    fn fill_splits_growth_by_tool_results() {
        let t = [
            asst("m1", 100, 900, &[("tu1", "Read"), ("tu2", "Bash")]),
            results(&["tu1", "tu2"]),
            asst("m2", 101, 1500, &[]),
        ]
        .join("\n");
        let l = ledger_from(&t);
        assert_eq!(fill(&l), [("Bash", 300), ("Read", 301), ("base", 1000)]);
        assert_eq!(l.ctx, 1601);
    }

    #[test]
    fn tool_use_in_a_repeated_entry_still_counts() {
        let t = [
            asst("m1", 100, 900, &[]),
            asst("m1", 100, 900, &[("tu1", "Read")]),
            results(&["tu1"]),
            asst("m2", 100, 1100, &[]),
        ]
        .join("\n");
        assert_eq!(fill(&ledger_from(&t)), [("Read", 200), ("base", 1000)]);
    }

    #[test]
    fn growth_without_tool_results_is_messages() {
        let t = [asst("m1", 100, 900, &[]), asst("m2", 100, 1100, &[])].join("\n");
        assert_eq!(fill(&ledger_from(&t)), [("base", 1000), ("messages", 200)]);
    }

    #[test]
    fn shrink_resets_to_base() {
        let t = [
            asst("m1", 100, 900, &[]),
            asst("m2", 100, 1100, &[]),
            asst("m3", 100, 400, &[]),
        ]
        .join("\n");
        let l = ledger_from(&t);
        assert_eq!(fill(&l), [("base", 500)]);
        assert_eq!(l.ctx, 500);
    }

    fn fixture(p: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(p)
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("ari-tokens-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn fixture_totals_match_reference() {
        // Reference numbers from an independent Python pass over the same files (deduped by id).
        let mut l = Ledger::default();
        read_ledger(&fixture("basic/transcript.jsonl"), &mut l);
        assert_eq!(
            (l.requests, l.input, l.cache_write, l.cache_read, l.output),
            (10, 22, 20933, 293088, 2040)
        );
        let mut l = Ledger::default();
        read_ledger(&fixture("multi/transcript.jsonl"), &mut l);
        assert_eq!(
            (l.requests, l.input, l.cache_write, l.cache_read, l.output),
            (13, 26, 21274, 393467, 2694)
        );
        assert_eq!(l.by_model.keys().collect::<Vec<_>>(), ["claude-haiku-5-5"]);
    }

    #[test]
    fn update_reads_main_and_agents() {
        let dir = tmp("update");
        let main = fixture("multi/transcript.jsonl");
        let mut st = State::default();
        for id in ["a3a6a01361cc3173c", "a5114050eef69c100"] {
            let p = fixture(&format!("multi/subagents/agent-{id}.jsonl"));
            st.agents.push(Agent {
                id: id.into(),
                transcript: Some(p.to_string_lossy().into()),
                ..Default::default()
            });
        }
        let c = update(&dir, &st, main.to_str());
        assert_eq!(c.agents["a3a6a01361cc3173c"].output, 192);
        assert_eq!(c.agents["a5114050eef69c100"].cache_read, 22432);
        let t = c.total();
        assert_eq!((t.requests, t.output), (17, 2694 + 192 + 235));
        let key = |c: &TokenCache| (c.main.offset, c.total().total(), c.agents.len());
        assert_eq!(key(&cached(&dir)), key(&c), "persisted");
        assert_eq!(
            key(&update(&dir, &st, main.to_str())),
            key(&c),
            "second pass adds nothing"
        );
    }

    #[test]
    fn model_names() {
        assert_eq!(short_model("claude-opus-5-5-20261001"), "opus-5-5");
        assert_eq!(short_model("claude-haiku-5-5"), "haiku-5-5");
        assert_eq!(short_model("deepseek-chat"), "deepseek-chat");
        let line = |id: &str, model: &str| {
            format!(
                r#"{{"type":"assistant","message":{{"id":"{id}","model":"{model}","usage":{{"input_tokens":1,"output_tokens":1}}}}}}"#
            )
        };
        let l = ledger_from(&format!(
            "{}\n{}\n",
            line("a", "claude-opus-5-5"),
            line("b", "claude-haiku-5-5")
        ));
        assert_eq!(l.last_model.as_deref(), Some("claude-haiku-5-5"));
        let l = ledger_from(&format!(
            "{}\n{}\n",
            line("a", "claude-opus-5-5"),
            line("b", "<synthetic>")
        ));
        assert_eq!(
            l.last_model.as_deref(),
            Some("claude-opus-5-5"),
            "placeholder entries after interrupts are not a model"
        );
    }

    #[test]
    fn dedupe_partial_and_rotation() {
        let dir = tmp("partial");
        let p = dir.join("t.jsonl");
        let entry = |id: &str, read: u64| {
            format!(
                "{{\"type\":\"assistant\",\"message\":{{\"id\":\"{id}\",\"model\":\"m\",\"usage\":{{\"input_tokens\":10,\"cache_read_input_tokens\":{read},\"cache_creation_input_tokens\":0,\"output_tokens\":5}}}}}}\n"
            )
        };
        let full = entry("b", 100);
        std::fs::write(&p, entry("a", 9000) + &entry("a", 9000) + &full[..20]).unwrap();
        let mut l = Ledger::default();
        read_ledger(&p, &mut l);
        assert_eq!((l.requests, l.cache_read), (1, 9000));
        std::fs::write(
            &p,
            entry("a", 9000) + &entry("a", 9000) + &full + &entry("c", 50),
        )
        .unwrap();
        read_ledger(&p, &mut l);
        assert_eq!((l.requests, l.cache_read), (3, 9150));
        std::fs::write(&p, entry("z", 1)).unwrap();
        let mut l2 = Ledger {
            offset: 10_000,
            ..Default::default()
        };
        read_ledger(&p, &mut l2);
        assert_eq!(l2.requests, 1, "shorter file starts over");
        let mut none = Ledger::default();
        read_ledger(&dir.join("missing.jsonl"), &mut none);
        assert_eq!(none, Ledger::default());
    }

    #[test]
    fn cache_drop_alert() {
        let c = TokenCache {
            main: Ledger {
                prev_hit: Some(0.9),
                last_hit: Some(0.1),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(c.drop_alert(), Some((0.9, 0.1)));
        let ok = TokenCache {
            main: Ledger {
                prev_hit: Some(0.9),
                last_hit: Some(0.6),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(ok.drop_alert(), None);
    }

    #[test]
    fn savings_paths() {
        let mut l = Ledger {
            cache_read: 1_000_000,
            ..Default::default()
        };
        l.by_model.insert(
            "m".into(),
            ModelTokens {
                cache_read: 1_000_000,
                ..Default::default()
            },
        );
        let prices = BTreeMap::from([(
            "m".to_string(),
            Price {
                input: 5.0,
                cache_read: 0.5,
                ..Default::default()
            },
        )]);
        assert!((savings(&l, None, &prices).unwrap() - 4.5).abs() < 1e-9);
        // Derived: $5 for 1M input tokens means p = $5/M; 1M cache reads would have saved 0.9 × $5.
        let l = Ledger {
            input: 1_000_000,
            cache_read: 1_000_000,
            ..Default::default()
        };
        let derived = savings(&l, Some(5.0 * (1.0 + 0.1)), &BTreeMap::new()).unwrap();
        assert!((derived - 4.5).abs() < 1e-9, "{derived}");
        assert_eq!(savings(&l, None, &BTreeMap::new()), None);
        assert_eq!(
            savings(&Ledger::default(), Some(1.0), &BTreeMap::new()),
            None
        );
    }
}
