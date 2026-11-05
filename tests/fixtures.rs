use ari_cli::{event, replay};
use std::path::Path;

fn fixture(name: &str) -> Vec<event::Event> {
    replay::load(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(name),
    )
    .unwrap()
}

#[test]
fn basic_events() {
    let lines: String = fixture("basic").iter().map(event::to_line).collect();
    insta::assert_snapshot!(lines);
}

#[test]
fn basic_timeline() {
    insta::assert_snapshot!(replay::format(&fixture("basic")));
}

#[test]
fn basic_verdicts() {
    use ari_cli::state::State;
    let events = fixture("basic");
    let t0 = events[0].t;
    let mut s = State::default();
    let mut last = None;
    let mut out = Vec::new();
    for ev in &events {
        s.apply(ev);
        let v = s.verdict(ev.t);
        if last != Some(v) {
            let (done, total) = s.tasks_done();
            out.push(format!(
                "+{:>5.1}s {:<9} after {:<18} tasks {done}/{total} agents {} errors {}",
                (ev.t - t0) as f64 / 1000.0,
                v.label(),
                ev.e,
                s.running_agents(ev.t),
                s.open_errors()
            ));
            last = Some(v);
        }
    }
    insta::assert_snapshot!(out.join("\n"));
}
