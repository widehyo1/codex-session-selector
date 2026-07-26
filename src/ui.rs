use ratatui::{
    layout::Rect,
    widgets::{Block, Borders},
};

pub(crate) fn pane_content_area(area: Rect) -> Rect {
    Block::new().borders(Borders::ALL).inner(area)
}

pub(crate) fn half_page_height(height: u16) -> u16 {
    (height / 2).max(1)
}

pub(crate) fn bottom_scroll_offset(line_count: usize, viewport_height: u16) -> u16 {
    let offset = line_count.saturating_sub(usize::from(viewport_height));
    offset.min(usize::from(u16::MAX)) as u16
}
