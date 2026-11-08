//! `ari statusline`: Claude Code's statusline entry point.
//! Chained statuslines print untouched; ARI adds its own row above or below.

use crate::config::{self, Position};
use crate::{bar, burn, chain, hook, paths, state, tokens, usage};
use serde_json::Value;
use std::io::{Read, Write};
use std::time::Duration;

pub fn run() -> ! {
    let mut input = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut input);
    let cfg = config::load();

    let cmds = cfg.chain.commands.clone();
    let stdin = input.clone();
    let timeout = Duration::from_millis(cfg.chain.timeout_ms);
    let chained = std::thread::spawn(move || chain::run_all(&cmds, &stdin, timeout));

    let row = std::panic::catch_unwind(|| own_row(&input, &cfg)).unwrap_or_else(|_| "ARI".into());
    let mut lines = chained.join().unwrap_or_default();
    match cfg.position {
        Position::Top => lines.insert(0, row.into_bytes()),
        Position::Bottom => lines.push(row.into_bytes()),
    }
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(&lines.join(&b'\n'));
    let _ = out.flush();
    std::process::exit(0)
}

fn own_row(input: &[u8], cfg: &config::Config) -> String {
    let raw: Value = serde_json::from_slice(input).unwrap_or(Value::Null);
    let u = usage::from_value(&raw);
    let now = hook::now_ms();
    let mut x = bar::Extras::default();
    let st = session_dir(&u)
        .map(|dir| {
            let root = paths::state_root();
            if raw.is_object() && std::fs::create_dir_all(&dir).is_ok() {
                let _ = paths::write_atomic(&dir.join("usage.json"), input);
                burn::record(&dir, &u, now);
                if let Some(root) = &root {
                    burn::record_week(root, u.seven_d, now);
                }
            }
            let st = state::load_session(&dir);
            x.compacts_in = burn::forecast(&dir, &st, cfg);
            x.plan = state::plan_file(&st);
            if let Some(root) = &root {
                x.conflicts = crate::conflicts::cached(root, &dir, now);
                x.week_eta = burn::week_projection(&burn::load_week(root, now), now);
            }
            x.tokens = Some(tokens::update_debounced(&dir, &st, &u));
            x.transcript_age_ms =
                tokens::transcript_for(&dir, &u).and_then(|t| tokens::transcript_age(&t, now));
            st
        })
        .unwrap_or_default();
    let width = std::env::var("COLUMNS")
        .ok()
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let color = std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    let row = bar::render(&st, &u, &x, cfg, now, width, color);
    if color {
        crate::theme::ansi(
            &row,
            &crate::theme::Palette::for_app(cfg, crate::app::App::Claude),
        )
    } else {
        row
    }
}

fn session_dir(u: &usage::Usage) -> Option<std::path::PathBuf> {
    paths::session_dir(&paths::state_root()?, u.session_id.as_deref()?)
}
