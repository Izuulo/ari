//! `ari replay <dir|file>`: print a recorded session as a timeline.

use crate::event::{self, Event};
use serde_json::Value;
use std::io;
use std::path::Path;

/// A dir resolves to `events.jsonl`, else `raw.jsonl`. Raw lines (`{"t","p"}`) are normalized.
pub fn load(path: &Path) -> io::Result<Vec<Event>> {
    let file = if path.is_dir() {
        ["events.jsonl", "raw.jsonl"]
            .iter()
            .map(|f| path.join(f))
            .find(|p| p.exists())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "no events.jsonl or raw.jsonl in that directory",
                )
            })?
    } else {
        path.to_path_buf()
    };
    Ok(parse(&std::fs::read_to_string(file)?))
}

pub fn parse(text: &str) -> Vec<Event> {
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| match v.get("p") {
            Some(p) => event::normalize(p, v.get("t").and_then(Value::as_u64).unwrap_or(0)),
            None => serde_json::from_value(v).ok(),
        })
        .collect()
}

pub fn format(events: &[Event]) -> String {
    let t0 = events.first().map_or(0, |e| e.t);
    events
        .iter()
        .map(|ev| {
            let glyph = match (ev.e.as_str(), ev.ok) {
                ("PreToolUse", _) => '▶',
                (_, Some(true)) => '✓',
                (_, Some(false)) => '✗',
                _ => '·',
            };
            let agent = ev.at.as_deref().or(ev.a.as_deref()).unwrap_or("main");
            let line = format!(
                "{:>7}  {:<10} {glyph} {:<18} {}",
                offset(ev.t.saturating_sub(t0)),
                agent,
                ev.e,
                detail(ev)
            );
            line.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn offset(ms: u64) -> String {
    if ms < 60_000 {
        format!("+{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("+{}m{:02}s", ms / 60_000, ms / 1000 % 60)
    }
}

fn detail(ev: &Event) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(act) = &ev.act
        && ev.task.is_none()
        && ev.todos.is_none()
    {
        parts.push(act.clone());
    }
    if let Some(t) = &ev.task {
        let mut s = format!("task #{}", t.id);
        for x in [&t.subject, &t.status].into_iter().flatten() {
            s.push(' ');
            s.push_str(x);
        }
        parts.push(s);
    }
    if let Some(todos) = &ev.todos {
        let done = todos.iter().filter(|t| t.s == "completed").count();
        parts.push(format!("todos {done}/{}", todos.len()));
    }
    parts.extend(ev.kind.clone());
    if let Some(bg) = ev.bg.filter(|n| *n > 0) {
        parts.push(format!("{bg} background running"));
    }
    parts.extend(ev.msg.clone());
    parts.join(" · ")
}

pub fn run(args: &[String]) -> ! {
    let Some(path) = args.first() else {
        eprintln!("usage: ari replay <session-dir|events.jsonl|raw.jsonl>");
        std::process::exit(2);
    };
    match load(Path::new(path)) {
        Ok(evs) if !evs.is_empty() => {
            println!("{}", format(&evs));
            std::process::exit(0)
        }
        Ok(_) => eprintln!("ari replay: {path}: no events"),
        Err(e) => eprintln!("ari replay: {path}: {e}"),
    }
    std::process::exit(1)
}
