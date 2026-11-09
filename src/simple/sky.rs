//! Constellation: each session as an animated star system.

use crate::hub::{HubView, Input, put};
use ratatui::{buffer::Buffer, layout::Rect, style::Style, text::Line};

const SOON: &str = "constellation (coming soon)";

/// Room enough to draw a sky.
pub fn fits(w: u16, h: u16) -> bool {
    w >= 30 && h >= 8
}

pub fn panel(_i: &Input, _w: u16, _h: u16, _color: bool) -> Vec<Line<'static>> {
    vec![Line::from(SOON)]
}

pub fn hub(buf: &mut Buffer, i: &Input, _v: &HubView) {
    system(buf, buf.area, i, true);
}

pub fn system(buf: &mut Buffer, area: Rect, _i: &Input, _big: bool) {
    put(buf, area.x, area.y, SOON, area.width, Style::default());
}

pub fn galaxy(buf: &mut Buffer, area: Rect, _i: &Input) {
    put(buf, area.x, area.y, SOON, area.width, Style::default());
}
