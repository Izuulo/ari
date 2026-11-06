const USAGE: &str = "ari — a zero-token status panel for Claude Code

usage:
  ari hook              hook entry point (called by Claude Code)
  ari statusline        statusline entry point (called by Claude Code)
  ari setup [--yes] [--dry-run]
                        wire ARI into your statusline (chains the existing one)
  ari uninstall [--yes] [--purge]
                        restore your original statusline
  ari watch [--session <id> | --all] [--once] [--ascii] [--hub]
                        full panel for this directory's session
  ari hub [--session <id> | --all] [--ascii]
                        the hub: every agent, its messages and context
  ari doctor            check that binary, plugin, statusline and hooks are wired
  ari demo [--speed <x>] [--ascii] [--once]
                        replay a bundled session in the panel (no Claude needed)
  ari recap [--session <id>] [--since <start|prompt|30m>] [--md]
                        what this directory's session got done
  ari replay <dir>      print a recorded session as a timeline
  ari --version";

use ari_cli::{demo, doctor, hook, recap, replay, setup, statusline, watch};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("hook") => hook::run(),
        Some("statusline") => statusline::run(),
        Some("setup") => setup::run_setup(&args[1..]),
        Some("uninstall") => setup::run_uninstall(&args[1..]),
        Some("watch") => watch::run(&args[1..]),
        Some("hub") => watch::run(&[&["--hub".to_string()], &args[1..]].concat()),
        Some("doctor") => doctor::run(&args[1..]),
        Some("demo") => demo::run(&args[1..]),
        Some("recap") => recap::run(&args[1..]),
        Some("replay") => replay::run(&args[1..]),
        Some("--version" | "-V") => println!("ari {}", env!("CARGO_PKG_VERSION")),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}
