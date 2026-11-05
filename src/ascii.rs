//! `--ascii`: swap ARI's own glyphs for ASCII. One column in, one column out, so layout holds.
//! User text (paths, messages) is left alone.

use crate::bar::SPINNER;
use ratatui::text::{Line, Span};

fn glyph(c: char) -> Option<char> {
    if let Some(i) = SPINNER.iter().position(|s| *s == c) {
        return Some(['|', '/', '-', '\\'][i % 4]);
    }
    Some(match c {
        '█' | '▇' | '▰' => '#',
        '▁' | '▂' => '_',
        '▃' | '▄' => '-',
        '▅' | '▆' => '=',
        '░' | '▱' => '-',
        '╭' | '╮' | '╰' | '╯' | '├' | '┤' | '└' => '+',
        '─' | '—' => '-',
        '│' => '|',
        '●' => '*',
        '◉' => '@',
        '✗' | '×' => 'x',
        '↻' | '⟲' | '≈' => '~',
        '✓' => 'v',
        '◌' | '•' => 'o',
        '▸' | '→' => '>',
        '↺' => '@',
        '▲' => '^',
        '⚠' => '!',
        '…' => '.',
        '·' => '-',
        _ => return None,
    })
}

pub fn map(s: &str) -> String {
    s.chars().map(|c| glyph(c).unwrap_or(c)).collect()
}

pub fn map_lines(lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|l| {
            Line::from(
                l.spans
                    .into_iter()
                    .map(|s| Span::styled(map(&s.content), s.style))
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_glyphs_not_text() {
        assert_eq!(map("╭─ ARI (•_•) ─╮"), "+- ARI (o_o) -+");
        assert_eq!(
            map("● ON TRACK │ ██████░░░░ 4/7 │ ⠋ 2 agents"),
            "* ON TRACK | ######---- 4/7 | | 2 agents"
        );
        assert_eq!(
            map("ctx ▰▰▱ 71% · 5h 62% ↺1h40m · ▲ ⚠ ✓ ✗ ↻ ◉ ◌ ▸ …"),
            "ctx ##- 71% - 5h 62% @1h40m - ^ ! v x ~ @ o > ."
        );
        assert_eq!(map("日本語 café"), "日本語 café");
        let frames: String = SPINNER.iter().map(|c| map(&c.to_string())).collect();
        assert_eq!(frames, "|/-\\|/-\\|/");
    }
}
