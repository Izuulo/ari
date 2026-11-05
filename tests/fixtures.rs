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
