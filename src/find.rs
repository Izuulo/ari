//! `ari find <text>`: search every Claude Code transcript, newest first,
//! and print how to resume each match.

pub const FIND_LIMIT: usize = 10;
pub const MAX_LIMIT: usize = 500;

#[derive(Debug, PartialEq)]
pub struct Opts {
    pub query: String,
    pub here: bool,
    pub limit: usize,
}

/// `Err` is the message after `ari find: `.
pub fn parse(args: &[String]) -> Result<Opts, String> {
    let (mut words, mut here, mut limit) = (Vec::new(), false, FIND_LIMIT);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--here" => here = true,
            "--limit" => {
                limit = it
                    .next()
                    .and_then(|n| n.parse().ok())
                    .filter(|n| (1..=MAX_LIMIT).contains(n))
                    .ok_or(format!("--limit takes a number from 1 to {MAX_LIMIT}"))?;
            }
            f if f.starts_with("--") => {
                return Err(format!("unknown option {f} (try --here, --limit <n>)"));
            }
            w => words.push(w),
        }
    }
    // One space between words, so the snippet's collapsed text can match.
    let query = words
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if query.is_empty() {
        return Err("what to look for? (ari find <text> [--here] [--limit <n>])".into());
    }
    Ok(Opts { query, here, limit })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flags_and_joins_words() {
        let a = |s: &[&str]| parse(&s.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        assert_eq!(
            a(&["token", "refresh"]),
            Ok(Opts {
                query: "token refresh".into(),
                here: false,
                limit: FIND_LIMIT
            })
        );
        assert_eq!(
            a(&["--here", "--limit", "3", "x"]),
            Ok(Opts {
                query: "x".into(),
                here: true,
                limit: 3
            })
        );
        assert!(a(&[]).is_err());
        assert!(a(&["--here"]).is_err());
        assert!(a(&["  "]).is_err());
        assert_eq!(a(&["a  b", "c"]).unwrap().query, "a b c");
        assert!(a(&["x", "--limit", "0"]).is_err());
        assert!(a(&["x", "--limit", "501"]).is_err());
        assert!(a(&["x", "--limit"]).is_err());
        assert!(a(&["x", "--nope"]).is_err());
    }
}
