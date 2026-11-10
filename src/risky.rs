//! Risky shell commands that ran: recursive deletes, deletes outside the repo,
//! `git reset --hard` and other discards, force pushes, deleted branches, SQL drops
//! and raw disk writes. Watch-only: ARI flags them after the fact and never blocks one.
//! Matching is by text, so it cannot see commands hidden in scripts or task runners,
//! variables and substitutions it can't resolve, aliases, other languages doing the
//! deleting, file or MCP tools, symlinks, or PowerShell.

use crate::event::truncate;

pub const OUTSIDE: &str = "delete outside repo";
pub const RECURSIVE: &str = "recursive delete";
pub const DISCARD: &str = "discards uncommitted work";
pub const FORCE_PUSH: &str = "rewrites remote history";
pub const BRANCH: &str = "deletes a branch";
pub const DROP: &str = "drops data";
pub const DISK: &str = "writes a raw disk";

const PREFIXES: [&str; 11] = [
    "sudo", "doas", "env", "command", "builtin", "exec", "nohup", "time", "nice", "timeout",
    "xargs",
];
const SHELLS: [&str; 4] = ["sh", "bash", "zsh", "dash"];
const TEMP_DIRS: [&str; 4] = [
    "/tmp/",
    "/private/tmp/",
    "/var/folders/",
    "/private/var/folders/",
];
const TEXT_TOOLS: [&str; 14] = [
    "echo", "printf", "grep", "egrep", "rg", "ag", "cat", "less", "head", "tail", "sed", "awk",
    "git", "gh",
];
const SQL: [&str; 4] = [
    "DROP TABLE",
    "DROP DATABASE",
    "DROP SCHEMA",
    "TRUNCATE TABLE",
];

/// Simple commands, each as words with quotes removed.
pub fn commands(cmd: &str) -> Vec<Vec<String>> {
    let (mut out, mut words, mut w) = (Vec::new(), Vec::new(), String::new());
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = cmd.chars().peekable();
    while let Some(ch) = chars.next() {
        match quote {
            Some('\'') if ch == '\'' => quote = None,
            Some('\'') => w.push(ch),
            Some(_) => match ch {
                '"' => quote = None,
                '\\' if matches!(chars.peek(), Some('"' | '\\' | '$' | '`')) => {
                    w.extend(chars.next())
                }
                _ => w.push(ch),
            },
            None => match ch {
                '\'' | '"' => {
                    quote = Some(ch);
                    in_word = true;
                }
                '\\' => {
                    if let Some(n) = chars.next().filter(|&n| n != '\n') {
                        w.push(n);
                        in_word = true;
                    }
                }
                ';' | '&' | '|' | '(' | ')' | '\n' | ' ' | '\t' | '\r' => {
                    if in_word {
                        words.push(std::mem::take(&mut w));
                        in_word = false;
                    }
                    if !matches!(ch, ' ' | '\t' | '\r') && !words.is_empty() {
                        out.push(std::mem::take(&mut words));
                    }
                }
                _ => {
                    w.push(ch);
                    in_word = true;
                }
            },
        }
    }
    if in_word {
        words.push(w);
    }
    if !words.is_empty() {
        out.push(words);
    }
    out
}

/// The first risky simple command in `cmd`, as (rule, its words cut to 80 chars).
/// `repo` is the project root and `cwd` the shell's folder (it can sit inside the repo);
/// either empty skips the outside rule. Paths resolve by text only.
pub fn check(cmd: &str, repo: &str, cwd: &str, home: &str) -> Option<(&'static str, String)> {
    let known = |p: &str| if p.is_empty() { String::new() } else { norm(p) };
    walk(cmd, &known(repo), &mut known(cwd), home)
}

fn walk(cmd: &str, repo: &str, cur: &mut String, home: &str) -> Option<(&'static str, String)> {
    for words in commands(cmd) {
        let w = strip_prefixes(&words);
        let Some(prog) = w.first().map(|p| p.rsplit('/').next().unwrap_or(p)) else {
            continue;
        };
        if prog == "cd" || prog == "pushd" {
            *cur = match w.get(1).map(String::as_str) {
                None | Some("~") => home.to_string(),
                Some("-") => String::new(),
                Some(d) => resolve(d, cur, home).unwrap_or_default(),
            };
            continue;
        }
        if SHELLS.contains(&prog) {
            let flag = w.iter().position(|a| is_short(a) && a.contains('c'));
            if let Some(script) = flag.and_then(|i| w.get(i + 1)) {
                if let Some(hit) = walk(script, repo, cur, home) {
                    return Some(hit);
                }
                continue;
            }
        }
        if let Some(r) = rule(prog, &w[1..], repo, cur, home) {
            return Some((r, truncate(&w.join(" "), 80)));
        }
    }
    None
}

/// Drops `sudo`, `env`, `VAR=1` and the like from the front, with their options.
fn strip_prefixes(words: &[String]) -> &[String] {
    let mut i = 0;
    loop {
        while words.get(i).is_some_and(|w| is_assignment(w)) {
            i += 1;
        }
        let Some(p) = words.get(i).filter(|p| PREFIXES.contains(&p.as_str())) else {
            return &words[i.min(words.len())..];
        };
        let timeout = p == "timeout";
        i += 1;
        while let Some(a) = words.get(i).filter(|a| a.starts_with('-')) {
            i += if matches!(a.as_str(), "-u" | "-g" | "-n" | "-I") {
                2
            } else {
                1
            };
        }
        if timeout {
            i += 1;
        }
    }
}

fn is_assignment(w: &str) -> bool {
    w.split_once('=').is_some_and(|(name, _)| {
        name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// `-rf`, `-D`: one dash, then flag letters.
fn is_short(a: &str) -> bool {
    a.len() > 1 && a.starts_with('-') && !a.starts_with("--")
}

fn rule(prog: &str, args: &[String], repo: &str, cur: &str, home: &str) -> Option<&'static str> {
    let has = |s: &str| args.iter().any(|a| a == s);
    // Options end at `--`; everything after it is a target.
    let opts = &args[..args.iter().position(|a| a == "--").unwrap_or(args.len())];
    if matches!(prog, "rm" | "rmdir" | "unlink" | "shred") && !repo.is_empty() && !cur.is_empty() {
        // `> /dev/null`: a redirect and its file are not targets.
        let redirect =
            |i: usize| args[i].contains(['<', '>']) || (i > 0 && args[i - 1].ends_with(['<', '>']));
        let targets = args.iter().enumerate().filter(|&(i, a)| {
            !redirect(i) && (i > opts.len() || (i < opts.len() && !a.starts_with('-')))
        });
        for (_, t) in targets {
            if let Some(p) = resolve(t, cur, home) {
                let temp = TEMP_DIRS.iter().any(|d| p.starts_with(d));
                if !temp && !p.starts_with(&format!("{repo}/")) {
                    return Some(OUTSIDE);
                }
            }
        }
    }
    match prog {
        "rm" if opts.iter().any(|a| {
            a == "--recursive" || (is_short(a) && (a.contains('r') || a.contains('R')))
        }) =>
        {
            return Some(RECURSIVE);
        }
        "find" => {
            let exec_rm = args.windows(2).any(|p| {
                matches!(p[0].as_str(), "-exec" | "-execdir")
                    && p[1].rsplit('/').next() == Some("rm")
            });
            if has("-delete") || exec_rm {
                return Some(RECURSIVE);
            }
        }
        "git" => return git(args),
        _ => {}
    }
    if !TEXT_TOOLS.contains(&prog) {
        let up = format!("{prog} {}", args.join(" ")).to_uppercase();
        let delete_all = up
            .find("DELETE FROM")
            .is_some_and(|i| !up[i..].contains("WHERE"));
        if delete_all || SQL.iter().any(|s| up.contains(s)) {
            return Some(DROP);
        }
    }
    let raw = prog == "dd" && args.iter().any(|a| a.starts_with("of=/dev/"));
    (raw || prog.starts_with("mkfs")).then_some(DISK)
}

fn git(args: &[String]) -> Option<&'static str> {
    let mut i = 0;
    while let Some(a) = args.get(i).filter(|a| a.starts_with('-')) {
        i += if a == "-C" || a == "-c" { 2 } else { 1 };
    }
    let sub = args.get(i)?.as_str();
    let rest = &args[i + 1..];
    let has = |s: &str| rest.iter().any(|a| a == s);
    let short = |c: char| rest.iter().any(|a| is_short(a) && a.contains(c));
    match sub {
        "reset" => has("--hard").then_some(DISCARD),
        "checkout" => {
            let paths = rest
                .iter()
                .position(|a| a == "--")
                .is_some_and(|i| i + 1 < rest.len());
            (has(".") || has("-f") || has("--force") || paths).then_some(DISCARD)
        }
        "restore" => {
            let staged = has("--staged") || has("-S");
            (!staged || has("--worktree") || has("-W")).then_some(DISCARD)
        }
        "clean" => {
            let dry = has("--dry-run") || short('n');
            ((has("--force") || short('f')) && !dry).then_some(DISCARD)
        }
        "push" => {
            let force = has("--force")
                || short('f')
                || rest
                    .iter()
                    .any(|a| a.starts_with("--force-with-lease") || a.starts_with('+'));
            force.then_some(FORCE_PUSH)
        }
        "branch" => short('D').then_some(BRANCH),
        _ => None,
    }
}

/// `word` as a normalized absolute path, or None when it can't be known by text.
fn resolve(word: &str, cur: &str, home: &str) -> Option<String> {
    let in_home = ["~", "$HOME", "${HOME}"].iter().find_map(|h| {
        word.strip_prefix(h)
            .filter(|r| r.is_empty() || r.starts_with('/'))
    });
    let path = if let Some(rest) = in_home {
        (!home.is_empty()).then(|| format!("{home}{rest}"))?
    } else if word.starts_with(['$', '~']) || word.contains('`') {
        return None;
    } else if word.starts_with('/') {
        word.to_string()
    } else {
        (!cur.is_empty()).then(|| format!("{cur}/{word}"))?
    };
    Some(norm(&path))
}

/// Lexical `.`, `..`, `//` and trailing `/` cleanup of an absolute path.
fn norm(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(p),
        }
    }
    format!("/{}", parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(cmd: &str) -> Option<&'static str> {
        check(cmd, "/home/u/proj", "/home/u/proj", "/home/u").map(|(r, _)| r)
    }

    #[test]
    fn splits_on_unquoted_separators() {
        assert_eq!(
            commands("a 'b c' && d \"e;f\" | g\\;h"),
            vec![vec!["a", "b c"], vec!["d", "e;f"], vec!["g;h"]]
        );
        assert_eq!(commands("(cd x; rm -r y)\nls").len(), 3);
        assert!(commands("").is_empty());
    }

    #[test]
    fn flags_each_rule() {
        assert_eq!(c("rm -rf build"), Some(RECURSIVE));
        assert_eq!(c("rm -r build"), Some(RECURSIVE));
        assert_eq!(c("rm -fR build"), Some(RECURSIVE));
        assert_eq!(c("find . -name '*.o' -delete"), Some(RECURSIVE));
        assert_eq!(c("find . -exec rm {} \\;"), Some(RECURSIVE));
        assert_eq!(c("git reset --hard HEAD~1"), Some(DISCARD));
        assert_eq!(c("git checkout -- ."), Some(DISCARD));
        assert_eq!(c("git checkout ."), Some(DISCARD));
        assert_eq!(c("git restore src/a.rs"), Some(DISCARD));
        assert_eq!(c("git clean -fd"), Some(DISCARD));
        assert_eq!(c("git clean -xdf"), Some(DISCARD));
        assert_eq!(c("git push --force"), Some(FORCE_PUSH));
        assert_eq!(c("git push -f origin main"), Some(FORCE_PUSH));
        assert_eq!(c("git push --force-with-lease"), Some(FORCE_PUSH));
        assert_eq!(c("git push origin +main"), Some(FORCE_PUSH));
        assert_eq!(c("git branch -D old"), Some(BRANCH));
        assert_eq!(c("psql -c \"drop table users\""), Some(DROP));
        assert_eq!(c("sqlite3 a.db 'DELETE FROM users'"), Some(DROP));
        assert_eq!(c("mysql -e 'truncate table logs'"), Some(DROP));
        assert_eq!(c("dd if=x.img of=/dev/disk2"), Some(DISK));
        assert_eq!(c("mkfs.ext4 /dev/sdb1"), Some(DISK));
    }

    #[test]
    fn safe_commands_pass() {
        for ok in [
            "rm notes.txt",
            "rm -- -rf",
            "rm -f a.o",
            "git reset --soft HEAD~1",
            "git restore --staged a.rs",
            "git clean -n",
            "git push -u origin feat",
            "git push --follow-tags",
            "git branch -d merged",
            "git checkout main",
            "sqlite3 a.db 'DELETE FROM t WHERE id = 1'",
            "truncate -s 0 app.log",
            "cargo test",
            "ls -la",
        ] {
            assert_eq!(c(ok), None, "{ok}");
        }
    }

    #[test]
    fn prefixes_chains_and_shells() {
        assert_eq!(c("sudo rm -rf build"), Some(RECURSIVE));
        assert_eq!(c("sudo -u root rm -rf build"), Some(RECURSIVE));
        assert_eq!(c("FOO=1 BAR=2 rm -r build"), Some(RECURSIVE));
        assert_eq!(c("env -i git push -f"), Some(FORCE_PUSH));
        assert_eq!(c("timeout 5 git reset --hard"), Some(DISCARD));
        assert_eq!(c("cargo build && git push --force"), Some(FORCE_PUSH));
        assert_eq!(c("ls || rm -rf build"), Some(RECURSIVE));
        assert_eq!(c("bash -lc 'git reset --hard'"), Some(DISCARD));
        assert_eq!(c("find . | xargs rm -rf"), Some(RECURSIVE));
        assert_eq!(c("git -C ../other reset --hard"), Some(DISCARD));
        assert_eq!(
            check("cd x && rm -rf y", "/p", "/p", "/h").unwrap().1,
            "rm -rf y"
        );
    }

    #[test]
    fn quoted_and_printed_text_is_not_flagged() {
        for ok in [
            "git commit -m \"rm -rf old code\"",
            "echo \"a; rm -rf /\"",
            "grep -rn \"DROP TABLE\" migrations/",
            "echo 'git push --force'",
        ] {
            assert_eq!(c(ok), None, "{ok}");
        }
    }

    #[test]
    fn outside_targets_resolve_by_text() {
        assert_eq!(c("rm -rf ../x"), Some(OUTSIDE));
        assert_eq!(c("cd .. && rm -rf proj"), Some(OUTSIDE));
        assert_eq!(c("rm -rf \"$HOME/notes\""), Some(OUTSIDE));
        assert_eq!(c("rm -rf ~"), Some(OUTSIDE));
        assert_eq!(c("rm -rf /*"), Some(OUTSIDE));
        assert_eq!(c("rm ../other/file.txt"), Some(OUTSIDE));
        assert_eq!(c("rm -rf ."), Some(OUTSIDE)); // the repo itself
        assert_eq!(c("rm -rf ./src/../build"), Some(RECURSIVE));
        assert_eq!(c("rm -rf /tmp/scratch"), Some(RECURSIVE)); // temp: only the recursive rule
        assert_eq!(c("rm /tmp/a.txt"), None);
        assert_eq!(c("rm -rf $DIR"), Some(RECURSIVE)); // unknown target: not OUTSIDE
        assert_eq!(c("cd - && rm a"), None);
        assert_eq!(
            check("rm -rf ../x", "", "", "/h").map(|r| r.0),
            Some(RECURSIVE)
        ); // no cwd
        assert_eq!(check("rm ../x", "", "", "/h"), None);
    }

    #[test]
    fn review_false_alarms() {
        // `gh` bodies are prose, not SQL.
        assert_eq!(
            c("gh pr create --title t --body \"we delete from the cache\""),
            None
        );
        // A redirect target is not a delete target.
        assert_eq!(c("rm -f a.o > /dev/null"), None);
        assert_eq!(c("rm -f a.o 2> /dev/null"), None);
        assert_eq!(c("rm -rf ../x 2>/dev/null"), Some(OUTSIDE));
        // The shell sits in a subfolder: the repo is the project dir, not the cwd.
        let sub =
            |cmd: &str| check(cmd, "/home/u/proj", "/home/u/proj/src", "/home/u").map(|r| r.0);
        assert_eq!(sub("rm ../README.md"), None);
        assert_eq!(sub("rm ../../other.txt"), Some(OUTSIDE));
    }

    #[test]
    fn command_text_is_cut() {
        let long = format!("rm -rf {}", "a".repeat(200));
        assert_eq!(
            check(&long, "/p", "/p", "/h").unwrap().1.chars().count(),
            80
        );
    }
}
