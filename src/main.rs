const USAGE: &str = "ari — a zero-token status panel for Claude Code

usage:
  ari hook              hook entry point (called by Claude Code)
  ari replay <dir>      print a recorded session as a timeline
  ari --version";

mod event;
mod hook;
mod paths;
mod replay;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("hook") => hook::run(),
        Some("replay") => replay::run(&args[1..]),
        Some("--version" | "-V") => println!("ari {}", env!("CARGO_PKG_VERSION")),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}
