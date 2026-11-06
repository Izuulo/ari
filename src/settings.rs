//! The settings screen in `ari watch` (`s`): every row, its value, how it steps, and the screen.

use crate::config::{COLOR_CYCLE, Config, Position, THEMES};

pub struct Item {
    pub group: &'static str,
    /// A `Config` field, `color.<role>` or `hide.<feature id>`.
    pub key: &'static str,
    pub label: &'static str,
}

const ITEMS: [(&str, &str, &str); 33] = [
    ("LOOK", "theme", "theme"),
    ("LOOK", "face", "face"),
    ("LOOK", "ascii", "ascii glyphs"),
    ("LOOK", "position", "statusline row"),
    ("LOOK", "max_width", "statusline width"),
    ("COLOURS", "color.good", "good"),
    ("COLOURS", "color.warn", "warn"),
    ("COLOURS", "color.bad", "bad"),
    ("COLOURS", "color.accent", "accent"),
    ("COLOURS", "color.highlight", "highlight"),
    ("COLOURS", "color.info", "info"),
    ("COLOURS", "color.muted", "muted"),
    ("COLOURS", "color.bar", "bar"),
    ("FEATURES", "hide.tasks", "task checklist"),
    ("FEATURES", "hide.agents", "agents · subagents"),
    ("FEATURES", "hide.errors", "errors · failed calls"),
    ("FEATURES", "hide.usage", "usage · context, limits"),
    ("FEATURES", "hide.session", "session · cost, time"),
    ("FEATURES", "hide.turns", "turns · recent turns"),
    ("FEATURES", "hide.conflicts", "conflicts · file clashes"),
    ("FEATURES", "hide.now", "now · current step"),
    ("FEATURES", "hide.tests", "tests · test results"),
    ("FEATURES", "hide.ctx", "ctx · context meter"),
    ("FEATURES", "hide.5h", "5h · 5-hour limit"),
    ("FEATURES", "hide.7d", "7d · weekly limit"),
    ("FEATURES", "hide.cost", "cost · session cost"),
    ("FEATURES", "hide.tokens", "tokens · token count"),
    ("ALERTS", "bell", "bell"),
    ("ALERTS", "ctx_warn", "context warn"),
    ("ALERTS", "ctx_crit", "context critical"),
    ("ALERTS", "stuck_repeats", "stuck after"),
    ("ALERTS", "compact_pct", "compact at"),
    ("DATA", "retention_days", "keep logs"),
];

pub fn items() -> Vec<Item> {
    ITEMS
        .iter()
        .map(|&(group, key, label)| Item { group, key, label })
        .collect()
}

fn on(b: bool) -> String {
    if b { "on" } else { "off" }.into()
}

/// What the row shows.
pub fn value(cfg: &Config, key: &str) -> String {
    let t = &cfg.thresholds;
    if let Some(role) = key.strip_prefix("color.") {
        return cfg
            .colors
            .get(role)
            .cloned()
            .unwrap_or_else(|| "theme".into());
    }
    if let Some(id) = key.strip_prefix("hide.") {
        return on(!cfg.hide.iter().any(|h| h == id));
    }
    match key {
        "theme" => cfg.theme.clone(),
        "face" => on(cfg.face),
        "ascii" => on(cfg.ascii),
        "bell" => on(cfg.bell),
        "position" => match cfg.position {
            Position::Top => "top".into(),
            Position::Bottom => "bottom".into(),
        },
        "max_width" if cfg.max_width == 0 => "auto".into(),
        "max_width" => cfg.max_width.to_string(),
        "ctx_warn" => format!("{}%", t.ctx_warn),
        "ctx_crit" => format!("{}%", t.ctx_crit),
        "compact_pct" => format!("{}%", t.compact_pct),
        "stuck_repeats" => format!("{} repeats", t.stuck_repeats),
        "retention_days" if cfg.retention_days == 1 => "1 day".into(),
        "retention_days" => format!("{} days", cfg.retention_days),
        _ => String::new(),
    }
}

/// `n` moved `by`, kept within `lo..=hi`.
fn nudge(n: u8, by: i32, lo: u8, hi: u8) -> u8 {
    (n as i32 + by).clamp(lo as i32, hi as i32) as u8
}

/// Change the row one step: `dir` 1 forward, -1 back. On/off rows flip either way.
pub fn step(cfg: &mut Config, key: &str, dir: i8) {
    let d = if dir >= 0 { 1 } else { -1 };
    if let Some(role) = key.strip_prefix("color.") {
        // `theme` sits at both ends of the named colours; a hand-typed value steps to `theme`.
        let cur = cfg.colors.get(role).map(String::as_str);
        let next = match cur.map(|c| COLOR_CYCLE.iter().position(|n| *n == c)) {
            None if d > 0 => Some(COLOR_CYCLE[0]),
            None => Some(COLOR_CYCLE[COLOR_CYCLE.len() - 1]),
            Some(None) => None,
            Some(Some(i)) => COLOR_CYCLE.get((i as i32 + d) as usize).copied(),
        };
        match next {
            Some(c) => cfg.colors.insert(role.into(), c.into()),
            None => cfg.colors.remove(role),
        };
        return;
    }
    if let Some(id) = key.strip_prefix("hide.") {
        match cfg.hide.iter().position(|h| h == id) {
            Some(i) => {
                cfg.hide.remove(i);
            }
            None => cfg.hide.push(id.into()),
        }
        return;
    }
    let t = &mut cfg.thresholds;
    match key {
        "theme" => {
            // An unknown theme counts as `default`.
            let i = THEMES.iter().position(|n| *n == cfg.theme).unwrap_or(0);
            let n = THEMES.len();
            cfg.theme = THEMES[if d > 0 { (i + 1) % n } else { (i + n - 1) % n }].into();
        }
        "face" => cfg.face ^= true,
        "ascii" => cfg.ascii ^= true,
        "bell" => cfg.bell ^= true,
        "position" => {
            cfg.position = match cfg.position {
                Position::Top => Position::Bottom,
                Position::Bottom => Position::Top,
            }
        }
        "max_width" => {
            cfg.max_width = match (cfg.max_width, d > 0) {
                (0, true) => 40,
                (w, true) => (w + 20).min(200),
                (w, false) if w <= 40 => 0,
                (w, false) => w - 20,
            }
        }
        "ctx_warn" => t.ctx_warn = nudge(t.ctx_warn, 5 * d, 5, t.ctx_crit.saturating_sub(5)),
        "ctx_crit" => t.ctx_crit = nudge(t.ctx_crit, 5 * d, t.ctx_warn + 5, 100),
        "stuck_repeats" => t.stuck_repeats = nudge(t.stuck_repeats, d, 2, 10),
        "compact_pct" => t.compact_pct = nudge(t.compact_pct, 5 * d, 50, 100),
        "retention_days" => {
            cfg.retention_days = (cfg.retention_days as i64 + d as i64).clamp(1, 90) as u64
        }
        _ => {}
    }
}

/// Put the row back to its default.
pub fn reset(cfg: &mut Config, key: &str) {
    let def = Config::default();
    if let Some(role) = key.strip_prefix("color.") {
        cfg.colors.remove(role);
        return;
    }
    if let Some(id) = key.strip_prefix("hide.") {
        cfg.hide.retain(|h| h != id);
        return;
    }
    let (t, dt) = (&mut cfg.thresholds, &def.thresholds);
    match key {
        "theme" => cfg.theme = def.theme,
        "face" => cfg.face = def.face,
        "ascii" => cfg.ascii = def.ascii,
        "bell" => cfg.bell = def.bell,
        "position" => cfg.position = def.position,
        "max_width" => cfg.max_width = def.max_width,
        "ctx_warn" => t.ctx_warn = dt.ctx_warn,
        "ctx_crit" => t.ctx_crit = dt.ctx_crit,
        "stuck_repeats" => t.stuck_repeats = dt.stuck_repeats,
        "compact_pct" => t.compact_pct = dt.compact_pct,
        "retention_days" => cfg.retention_days = def.retention_days,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, FEATURES, ROLES};

    #[test]
    fn items_cover_every_role_and_feature() {
        let it = items();
        assert_eq!(it.len(), 33);
        for r in ROLES {
            assert!(it.iter().any(|i| i.key == format!("color.{r}")), "{r}");
        }
        for f in FEATURES {
            assert!(it.iter().any(|i| i.key == format!("hide.{f}")), "{f}");
        }
        let groups: Vec<&str> = it.iter().map(|i| i.group).collect();
        assert_eq!(groups[0], "LOOK");
        assert_eq!(groups[32], "DATA");
    }

    #[test]
    fn every_item_has_a_value_and_steps_back() {
        for i in items() {
            let mut c = Config::default();
            let before = value(&c, i.key);
            assert!(!before.is_empty(), "{}", i.key);
            step(&mut c, i.key, 1);
            assert_ne!(value(&c, i.key), before, "{} didn't change", i.key);
            step(&mut c, i.key, -1);
            assert_eq!(value(&c, i.key), before, "{} didn't come back", i.key);
            assert_eq!(c, Config::default(), "{}", i.key);
        }
    }

    #[test]
    fn theme_cycles_and_wraps() {
        let mut c = Config::default();
        let mut seen = Vec::new();
        for _ in 0..5 {
            step(&mut c, "theme", 1);
            seen.push(c.theme.clone());
        }
        assert_eq!(seen, ["soft", "vivid", "colorblind", "mono", "default"]);
        step(&mut c, "theme", -1);
        assert_eq!(c.theme, "mono");
        c.theme = "neon".into();
        step(&mut c, "theme", 1);
        assert_eq!(c.theme, "soft");
    }

    #[test]
    fn thresholds_stay_ordered() {
        let mut c = Config::default();
        for _ in 0..20 {
            step(&mut c, "ctx_warn", 1);
        }
        assert_eq!(c.thresholds.ctx_warn, 75);
        for _ in 0..20 {
            step(&mut c, "ctx_crit", -1);
        }
        assert_eq!(c.thresholds.ctx_crit, 80);
        let mut c = Config::default();
        for _ in 0..20 {
            step(&mut c, "ctx_crit", -1);
        }
        assert_eq!(c.thresholds.ctx_crit, 65);
        for _ in 0..20 {
            step(&mut c, "ctx_crit", 1);
        }
        assert_eq!(c.thresholds.ctx_crit, 100);
        for _ in 0..20 {
            step(&mut c, "ctx_warn", -1);
        }
        assert_eq!(c.thresholds.ctx_warn, 5);
        for (key, lo, hi) in [
            ("stuck_repeats", 2, 10),
            ("compact_pct", 50, 100),
            ("retention_days", 1, 90),
        ] {
            let mut c = Config::default();
            for _ in 0..200 {
                step(&mut c, key, -1);
            }
            let low = value(&c, key);
            for _ in 0..200 {
                step(&mut c, key, 1);
            }
            let high = value(&c, key);
            assert!(low.starts_with(&lo.to_string()), "{key} {low}");
            assert!(high.starts_with(&hi.to_string()), "{key} {high}");
        }
        let mut c = Config::default();
        assert_eq!(value(&c, "max_width"), "auto");
        step(&mut c, "max_width", 1);
        assert_eq!(c.max_width, 40);
        step(&mut c, "max_width", 1);
        assert_eq!(c.max_width, 60);
        for _ in 0..20 {
            step(&mut c, "max_width", 1);
        }
        assert_eq!(c.max_width, 200);
    }

    #[test]
    fn color_cycle() {
        let mut c = Config::default();
        assert_eq!(value(&c, "color.good"), "theme");
        step(&mut c, "color.good", 1);
        assert_eq!(value(&c, "color.good"), "red");
        step(&mut c, "color.good", 1);
        assert_eq!(value(&c, "color.good"), "green");
        let mut c = Config::default();
        step(&mut c, "color.good", -1);
        assert_eq!(value(&c, "color.good"), "lightcyan");
        c.colors.insert("good".into(), "#123456".into());
        assert_eq!(value(&c, "color.good"), "#123456");
        step(&mut c, "color.good", 1);
        assert_eq!(value(&c, "color.good"), "theme");
        assert!(c.colors.is_empty());
    }

    #[test]
    fn feature_toggle() {
        let mut c = Config::default();
        step(&mut c, "hide.tasks", 1);
        assert_eq!(c.hide, ["tasks"]);
        assert_eq!(value(&c, "hide.tasks"), "off");
        step(&mut c, "hide.cost", -1);
        assert_eq!(c.hide, ["tasks", "cost"]);
        step(&mut c, "hide.tasks", 1);
        assert_eq!(c.hide, ["cost"]);
        assert_eq!(value(&c, "hide.tasks"), "on");
    }

    #[test]
    fn reset_restores_default() {
        let mut c = Config::default();
        for k in [
            "theme",
            "color.bad",
            "hide.agents",
            "bell",
            "face",
            "ctx_warn",
        ] {
            step(&mut c, k, 1);
        }
        assert_ne!(c, Config::default());
        for k in [
            "theme",
            "color.bad",
            "hide.agents",
            "bell",
            "face",
            "ctx_warn",
        ] {
            reset(&mut c, k);
        }
        assert_eq!(c, Config::default());
    }

    #[test]
    fn task_checklist_is_listed_and_on() {
        let it = items();
        let t = it.iter().find(|i| i.label == "task checklist").unwrap();
        assert_eq!(t.key, "hide.tasks");
        assert_eq!(value(&Config::default(), t.key), "on");
    }
}
