//! Colour themes. ARI draws with eight base colours; a `Palette` maps each to its role's
//! colour at the very end, so drawing code never changes (spec: 2026-11-06-settings-design.md).

use crate::app::App;
use crate::config::{Config, ROLES};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub good: Color,
    pub warn: Color,
    pub bad: Color,
    pub accent: Color,
    pub highlight: Color,
    pub info: Color,
    pub muted: Color,
    pub bar: Color,
}

/// The base colours ARI draws with, in `ROLES` order.
const BASE: [Color; 8] = [
    Color::Green,
    Color::Yellow,
    Color::Red,
    Color::Cyan,
    Color::Magenta,
    Color::Blue,
    Color::DarkGray,
    Color::Indexed(236),
];

fn preset(theme: &str) -> [Color; 8] {
    use Color::*;
    match theme {
        "soft" => [114, 179, 167, 116, 176, 110, 243, 236].map(Indexed),
        "vivid" => [
            LightGreen,
            LightYellow,
            LightRed,
            LightCyan,
            LightMagenta,
            LightBlue,
            Gray,
            Indexed(234),
        ],
        "colorblind" => [39, 220, 208, 255, 175, 111, 243, 236].map(Indexed),
        "claude" => [
            Green,
            Yellow,
            Red,
            Rgb(0xD9, 0x77, 0x57),
            Rgb(0xE8, 0xA3, 0x83),
            Rgb(0xB5, 0xAD, 0xA4),
            Indexed(244),
            Indexed(237),
        ],
        "codex" => [
            Green,
            Yellow,
            Red,
            Rgb(0xEC, 0xEC, 0xEC),
            Indexed(252),
            Indexed(110),
            Indexed(243),
            Indexed(235),
        ],
        "deepseek" => [
            Green,
            Yellow,
            Red,
            Rgb(0x4D, 0x6B, 0xFE),
            Rgb(0x8F, 0xA4, 0xFF),
            Rgb(0x6F, 0x8C, 0xC9),
            Indexed(103),
            Indexed(236),
        ],
        "mono" => [
            Reset,
            Reset,
            Reset,
            Reset,
            Reset,
            Reset,
            DarkGray,
            Indexed(236),
        ],
        _ => BASE,
    }
}

impl App {
    pub fn accent(self) -> Color {
        preset(self.theme())[3]
    }
}

impl Palette {
    /// The theme's colours (unknown theme: default), then each readable `[colors]` override.
    pub fn new(cfg: &Config) -> Palette {
        Self::build(preset(&cfg.theme), cfg)
    }

    /// Like `new`, but `auto` takes the colours of the session's app.
    pub fn for_app(cfg: &Config, app: App) -> Palette {
        if cfg.theme == "auto" {
            Self::build(preset(app.theme()), cfg)
        } else {
            Self::new(cfg)
        }
    }

    fn build(mut c: [Color; 8], cfg: &Config) -> Palette {
        for (i, role) in ROLES.iter().enumerate() {
            if let Some(v) = cfg.colors.get(*role).and_then(|v| Color::from_str(v).ok()) {
                c[i] = v;
            }
        }
        let [good, warn, bad, accent, highlight, info, muted, bar] = c;
        Palette {
            good,
            warn,
            bad,
            accent,
            highlight,
            info,
            muted,
            bar,
        }
    }

    fn all(&self) -> [Color; 8] {
        [
            self.good,
            self.warn,
            self.bad,
            self.accent,
            self.highlight,
            self.info,
            self.muted,
            self.bar,
        ]
    }

    pub fn role(&self, name: &str) -> Option<Color> {
        ROLES.iter().position(|r| *r == name).map(|i| self.all()[i])
    }

    fn map(&self, c: Color) -> Color {
        BASE.iter()
            .position(|b| *b == c)
            .map_or(c, |i| self.all()[i])
    }

    fn style(&self, s: Style) -> Style {
        Style {
            fg: s.fg.map(|c| self.map(c)),
            bg: s.bg.map(|c| self.map(c)),
            ..s
        }
    }
}

/// Recolour finished lines: each base colour becomes its role's colour, once.
pub fn lines(mut lines: Vec<Line<'static>>, p: &Palette) -> Vec<Line<'static>> {
    for l in &mut lines {
        l.style = p.style(l.style);
        for s in &mut l.spans {
            s.style = p.style(s.style);
        }
    }
    lines
}

/// Recolour ARI's own ANSI row: the base foreground params inside each `ESC[…m`.
pub fn ansi(s: &str, p: &Palette) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("\x1b[") {
        out.push_str(&rest[..i + 2]);
        rest = &rest[i + 2..];
        let Some(end) = rest.find('m') else { break };
        let params: Vec<&str> = rest[..end].split(';').collect();
        let mut new = Vec::new();
        let mut k = 0;
        while k < params.len() {
            match params[k] {
                // Extended colours are already explicit; copy them whole.
                "38" | "48" => {
                    let n = if params.get(k + 1) == Some(&"2") {
                        5
                    } else {
                        3
                    };
                    new.extend(
                        params[k..(k + n).min(params.len())]
                            .iter()
                            .map(|x| x.to_string()),
                    );
                    k += n;
                    continue;
                }
                code => {
                    let base = match code {
                        "32" => Some(Color::Green),
                        "33" => Some(Color::Yellow),
                        "31" => Some(Color::Red),
                        "36" => Some(Color::Cyan),
                        "35" => Some(Color::Magenta),
                        "34" => Some(Color::Blue),
                        "90" => Some(Color::DarkGray),
                        _ => None,
                    };
                    new.push(base.map_or(code.to_string(), |b| sgr_fg(p.map(b))));
                }
            }
            k += 1;
        }
        out.push_str(&new.join(";"));
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn sgr_fg(c: Color) -> String {
    use Color::*;
    let named = [
        Black,
        Red,
        Green,
        Yellow,
        Blue,
        Magenta,
        Cyan,
        Gray,
        DarkGray,
        LightRed,
        LightGreen,
        LightYellow,
        LightBlue,
        LightMagenta,
        LightCyan,
        White,
    ];
    match c {
        Reset => "39".into(),
        Indexed(n) => format!("38;5;{n}"),
        Rgb(r, g, b) => format!("38;2;{r};{g};{b}"),
        _ => {
            let i = named.iter().position(|n| *n == c).unwrap_or(7);
            (if i < 8 { 30 + i } else { 90 + i - 8 }).to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use ratatui::style::{Color, Style};
    use ratatui::text::{Line, Span};

    fn cfg(theme: &str, colors: &[(&str, &str)]) -> Config {
        Config {
            theme: theme.into(),
            colors: colors
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Config::default()
        }
    }

    fn sample() -> Vec<Line<'static>> {
        vec![Line::from(vec![
            Span::styled("ok", Style::new().fg(Color::Green)),
            Span::styled("bad", Style::new().fg(Color::Red)),
            Span::styled("bar", Style::new().bg(Color::Indexed(236))),
            Span::raw("plain"),
        ])]
    }

    #[test]
    fn app_themes_keep_status_colours() {
        for t in ["claude", "codex", "deepseek"] {
            let p = Palette::new(&cfg(t, &[]));
            assert_eq!(
                (p.good, p.warn, p.bad),
                (Color::Green, Color::Yellow, Color::Red)
            );
        }
    }

    #[test]
    fn auto_follows_app() {
        let auto = cfg("auto", &[]);
        assert_eq!(
            Palette::for_app(&auto, App::Codex).accent,
            Color::Rgb(0xEC, 0xEC, 0xEC)
        );
        assert_eq!(
            Palette::for_app(&auto, App::Claude).accent,
            Color::Rgb(0xD9, 0x77, 0x57)
        );
        assert_eq!(App::Dsh.accent(), Color::Rgb(0x4D, 0x6B, 0xFE));
        let soft = cfg("soft", &[]);
        assert_eq!(Palette::for_app(&soft, App::Codex), Palette::new(&soft));
    }

    #[test]
    fn auto_alone_is_default() {
        assert_eq!(
            Palette::new(&cfg("auto", &[])),
            Palette::new(&cfg("default", &[]))
        );
    }

    #[test]
    fn default_palette_is_identity() {
        let p = Palette::new(&Config::default());
        assert_eq!(lines(sample(), &p), sample());
        let s = "\x1b[1;33mhi\x1b[0m \x1b[2m│\x1b[0m";
        assert_eq!(ansi(s, &p), s);
    }

    #[test]
    fn theme_preset_and_override() {
        let p = Palette::new(&cfg("soft", &[("good", "red")]));
        assert_eq!(p.good, Color::Red);
        assert_eq!(p.bad, Color::Indexed(167));
        assert_eq!(p.role("bad"), Some(Color::Indexed(167)));
        assert_eq!(p.role("nope"), None);
        let def = Palette::new(&Config::default());
        assert_eq!(Palette::new(&cfg("neon", &[])), def);
        assert_eq!(Palette::new(&cfg("default", &[("good", "#zz")])), def);
    }

    #[test]
    fn override_equal_to_other_base_maps_once() {
        let p = Palette::new(&cfg("default", &[("good", "red"), ("bad", "blue")]));
        let out = lines(sample(), &p);
        assert_eq!(out[0].spans[0].style.fg, Some(Color::Red));
        assert_eq!(out[0].spans[1].style.fg, Some(Color::Blue));
    }

    #[test]
    fn ansi_rewrites_color_params_only() {
        let p = Palette::new(&cfg("soft", &[]));
        assert_eq!(
            ansi("\x1b[1;32mok\x1b[0m \x1b[2m│\x1b[0m", &p),
            "\x1b[1;38;5;114mok\x1b[0m \x1b[2m│\x1b[0m"
        );
    }

    #[test]
    fn mono_resets_colors() {
        let p = Palette::new(&cfg("mono", &[]));
        assert_eq!(ansi("\x1b[31mx", &p), "\x1b[39mx");
        let out = lines(sample(), &p);
        assert_eq!(out[0].spans[0].style.fg, Some(Color::Reset));
    }

    #[test]
    fn sgr_for_each_kind() {
        assert_eq!(sgr_fg(Color::Red), "31");
        assert_eq!(sgr_fg(Color::LightCyan), "96");
        assert_eq!(sgr_fg(Color::Gray), "37");
        assert_eq!(sgr_fg(Color::DarkGray), "90");
        assert_eq!(sgr_fg(Color::Indexed(7)), "38;5;7");
        assert_eq!(sgr_fg(Color::Rgb(1, 2, 3)), "38;2;1;2;3");
        assert_eq!(sgr_fg(Color::Reset), "39");
    }
}
