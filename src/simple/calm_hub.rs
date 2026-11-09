//! Calm hub: the hub's layout in plain English, without the numbers.

use crate::hub::{HubView, Input, put};
use ratatui::{buffer::Buffer, layout::Rect, style::Style};

pub fn hub(buf: &mut Buffer, i: &Input, _v: &HubView) {
    cards(buf, buf.area, i);
}

pub fn cards(buf: &mut Buffer, area: Rect, _i: &Input) {
    let text = "calm (coming soon)";
    put(buf, area.x, area.y, text, area.width, Style::default());
}
