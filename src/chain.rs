//! Run the user's previous statusline commands with the same stdin, in parallel, under a deadline.

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

struct Running {
    child: Child,
    buf: Arc<Mutex<Vec<u8>>>,
    eof: Arc<AtomicBool>,
}

/// Outputs in config order, one trailing newline removed, bytes untouched.
/// Commands that fail, time out or can't start are left out.
pub fn run_all(cmds: &[String], stdin: &[u8], timeout: Duration) -> Vec<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let running: Vec<Running> = cmds
        .iter()
        .filter(|c| !c.contains("ari statusline")) // never chain ourselves
        .filter_map(|c| spawn(c, stdin))
        .collect();
    running
        .into_iter()
        .filter_map(|r| finish(r, deadline))
        .collect()
}

fn spawn(cmd: &str, stdin: &[u8]) -> Option<Running> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut sin = child.stdin.take()?;
    let input = stdin.to_vec();
    thread::spawn(move || {
        let _ = sin.write_all(&input);
    });
    let mut out = child.stdout.take()?;
    let buf = Arc::new(Mutex::new(Vec::new()));
    let eof = Arc::new(AtomicBool::new(false));
    let (b, e) = (buf.clone(), eof.clone());
    thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        while let Ok(n) = out.read(&mut chunk) {
            if n == 0 {
                break;
            }
            b.lock().unwrap().extend_from_slice(&chunk[..n]);
        }
        e.store(true, Ordering::Release);
    });
    Some(Running { child, buf, eof })
}

fn finish(mut r: Running, deadline: Instant) -> Option<Vec<u8>> {
    let tick = Duration::from_millis(2);
    let status = loop {
        if let Some(s) = r.child.try_wait().ok()? {
            break s;
        }
        if Instant::now() >= deadline {
            let _ = r.child.kill();
            let _ = r.child.wait();
            return None;
        }
        thread::sleep(tick);
    };
    if !status.success() {
        return None;
    }
    // A backgrounded grandchild can hold stdout open; take what arrived by the deadline.
    while !r.eof.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::sleep(tick);
    }
    let mut out = std::mem::take(&mut *r.buf.lock().unwrap());
    if out.last() == Some(&b'\n') {
        out.pop();
    }
    // A statusline that prints nothing (e.g. Orca's outside Orca) must not add a blank row.
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(cmds: &[&str], stdin: &str) -> Vec<String> {
        let cmds: Vec<String> = cmds.iter().map(|s| s.to_string()).collect();
        run_all(&cmds, stdin.as_bytes(), Duration::from_millis(300))
            .into_iter()
            .map(|b| String::from_utf8(b).unwrap())
            .collect()
    }

    #[test]
    fn keeps_order_and_bytes() {
        assert_eq!(
            run(
                &["printf 'a\\nb\\n'", "printf '\\033[32m[X]\\033[0m'", "cat"],
                "{\"s\":1}"
            ),
            vec!["a\nb", "\x1b[32m[X]\x1b[0m", "{\"s\":1}"]
        );
    }

    #[test]
    fn failures_and_timeouts_are_skipped() {
        let t = Instant::now();
        assert_eq!(
            run(
                &[
                    "echo before",
                    "exit 1",
                    "sleep 2; echo late",
                    "nope-not-a-cmd",
                    "echo after"
                ],
                ""
            ),
            vec!["before", "after"]
        );
        assert!(
            t.elapsed() < Duration::from_millis(600),
            "{:?}",
            t.elapsed()
        );
    }

    #[test]
    fn grandchild_holding_stdout() {
        let t = Instant::now();
        assert_eq!(run(&["(sleep 3 &); echo hi"], ""), vec!["hi"]);
        assert!(
            t.elapsed() < Duration::from_millis(600),
            "{:?}",
            t.elapsed()
        );
    }

    #[test]
    fn empty_output_adds_no_row() {
        assert_eq!(run(&["cat >/dev/null", "true", "echo x"], "{}"), vec!["x"]);
    }

    #[test]
    fn never_chains_itself() {
        assert!(run(&["ari statusline"], "").is_empty());
    }
}
