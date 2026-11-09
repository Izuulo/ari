//! Calm watch: today's panel layout in plain English, without the numbers.

use crate::hub::Input;
use ratatui::text::Line;

pub fn panel(_i: &Input, _w: u16, _h: u16, _color: bool) -> Vec<Line<'static>> {
    vec![Line::from("calm (coming soon)")]
}
