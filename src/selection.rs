//! Pane-aware text selection for commit details and diff views.

use std::cell::RefCell;

use anyhow::Result;
use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::Modifier,
    widgets::Widget,
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::tui;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TextPoint {
    row: usize,
    col: usize,
}

#[derive(Debug, Clone)]
struct SelectableViewport {
    rect: Rect,
    row_offset: usize,
    col_offset: usize,
    context: String,
}

impl SelectableViewport {
    fn map_screen(&self, x: u16, y: u16, clamp: bool) -> Option<TextPoint> {
        if self.rect.width == 0 || self.rect.height == 0 {
            return None;
        }

        let position = Position { x, y };
        let (x, y) = if self.rect.contains(position) {
            (x, y)
        } else if clamp {
            let max_x = self
                .rect
                .x
                .saturating_add(self.rect.width.saturating_sub(1));
            let max_y = self
                .rect
                .y
                .saturating_add(self.rect.height.saturating_sub(1));
            (x.clamp(self.rect.x, max_x), y.clamp(self.rect.y, max_y))
        } else {
            return None;
        };

        Some(TextPoint {
            row: self.row_offset + usize::from(y.saturating_sub(self.rect.y)),
            col: self.col_offset + usize::from(x.saturating_sub(self.rect.x)),
        })
    }

    fn logical_row_range(&self) -> std::ops::Range<usize> {
        self.row_offset..self.row_offset + usize::from(self.rect.height)
    }

    fn logical_col_range(&self) -> std::ops::Range<usize> {
        self.col_offset..self.col_offset + usize::from(self.rect.width)
    }
}

#[derive(Debug, Clone)]
struct TextSelection {
    context: String,
    anchor: TextPoint,
    cursor: TextPoint,
    line_min_col: usize,
    line_max_col: usize,
    dragged: bool,
    finalized: bool,
    captured: bool,
    pending_auto_copy: bool,
    text: String,
}

impl TextSelection {
    fn ordered_points(&self) -> (TextPoint, TextPoint) {
        if self.anchor <= self.cursor {
            (self.anchor, self.cursor)
        } else {
            (self.cursor, self.anchor)
        }
    }

    fn col_range_for_row(&self, row: usize) -> Option<(usize, usize)> {
        let (start, end) = self.ordered_points();
        if row < start.row || row > end.row {
            return None;
        }
        if start.row == end.row {
            return Some((start.col.min(end.col), start.col.max(end.col)));
        }
        if row == start.row {
            return Some((start.col, self.line_max_col));
        }
        if row == end.row {
            return Some((self.line_min_col, end.col));
        }
        Some((self.line_min_col, self.line_max_col))
    }
}

struct SelectionRuntime {
    viewport: Option<SelectableViewport>,
    selection: Option<TextSelection>,
    auto_copy: bool,
}

impl Default for SelectionRuntime {
    fn default() -> Self {
        Self {
            viewport: None,
            selection: None,
            auto_copy: true,
        }
    }
}

thread_local! {
    static STATE: RefCell<SelectionRuntime> = RefCell::new(SelectionRuntime::default());
}

/// Configure whether releasing a completed selection copies it immediately.
pub fn configure(auto_copy: bool) {
    STATE.with(|state| state.borrow_mut().auto_copy = auto_copy);
}

/// Reset only the current frame's selectable viewport. The logical selection
/// remains intact and will be shown again when the same context is rendered.
pub fn begin_frame() {
    STATE.with(|state| state.borrow_mut().viewport = None);
}

/// Declare the selectable text region rendered in the current frame.
/// `row_offset` and `col_offset` are logical content offsets caused by scrolling.
pub fn set_viewport(
    rect: Rect,
    row_offset: usize,
    col_offset: usize,
    context: impl Into<String>,
) {
    let context = context.into();
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state
            .selection
            .as_ref()
            .is_some_and(|selection| selection.context != context)
        {
            state.selection = None;
        }
        state.viewport = (rect.width > 0 && rect.height > 0).then_some(SelectableViewport {
            rect,
            row_offset,
            col_offset,
            context,
        });
    });
}

/// Start selecting if the mouse-down happened inside the active text viewport.
/// Returns true when a selection gesture was started.
pub fn begin(x: u16, y: u16) -> bool {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(viewport) = state.viewport.clone() else {
            return false;
        };
        let Some(point) = viewport.map_screen(x, y, false) else {
            return false;
        };
        let col_range = viewport.logical_col_range();
        let line_max_col = col_range.end.saturating_sub(1);
        state.selection = Some(TextSelection {
            context: viewport.context,
            anchor: point,
            cursor: point,
            line_min_col: col_range.start,
            line_max_col,
            dragged: false,
            finalized: false,
            captured: false,
            pending_auto_copy: false,
            text: String::new(),
        });
        true
    })
}

/// Extend an active selection. Dragging outside the pane clamps to its content edge.
pub fn drag(x: u16, y: u16) -> bool {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(viewport) = state.viewport.clone() else {
            return false;
        };
        let Some(point) = viewport.map_screen(x, y, true) else {
            return false;
        };
        let Some(selection) = state.selection.as_mut() else {
            return false;
        };
        if selection.context != viewport.context || selection.finalized {
            return false;
        }
        selection.cursor = point;
        selection.dragged |= point != selection.anchor;
        selection.captured = false;
        true
    })
}

/// Finish an active selection. A plain click does not create a one-cell selection.
pub fn finish(x: u16, y: u16) -> bool {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(viewport) = state.viewport.clone() else {
            return false;
        };
        let Some(point) = viewport.map_screen(x, y, true) else {
            return false;
        };
        let auto_copy = state.auto_copy;
        let Some(selection) = state.selection.as_mut() else {
            return false;
        };
        if selection.context != viewport.context {
            state.selection = None;
            return false;
        }
        selection.cursor = point;
        selection.dragged |= point != selection.anchor;
        if !selection.dragged {
            state.selection = None;
            return false;
        }
        selection.finalized = true;
        selection.captured = false;
        selection.pending_auto_copy = auto_copy;
        true
    })
}

pub fn clear() {
    STATE.with(|state| state.borrow_mut().selection = None);
}

pub fn has_selected_text() -> bool {
    STATE.with(|state| {
        state
            .borrow()
            .selection
            .as_ref()
            .is_some_and(|selection| selection.finalized && !selection.text.is_empty())
    })
}

/// Copy the completed selection manually (used by `y` when a selection exists).
pub fn copy_selected() -> Result<bool> {
    let text = STATE.with(|state| {
        state
            .borrow()
            .selection
            .as_ref()
            .filter(|selection| selection.finalized && !selection.text.is_empty())
            .map(|selection| selection.text.clone())
    });
    let Some(text) = text else {
        return Ok(false);
    };
    tui::copy_to_clipboard(&text)?;
    Ok(true)
}

/// Flush one pending automatic copy after the frame has captured the final text.
pub fn flush_auto_copy() -> Result<bool> {
    let text = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let selection = state.selection.as_mut()?;
        if !selection.pending_auto_copy || !selection.captured || selection.text.is_empty() {
            return None;
        }
        selection.pending_auto_copy = false;
        Some(selection.text.clone())
    });
    let Some(text) = text else {
        return Ok(false);
    };
    tui::copy_to_clipboard(&text)?;
    Ok(true)
}

/// Apply selection highlighting to the already-rendered terminal buffer and
/// capture the selected visible text when the drag is finalized.
pub fn render_overlay(frame: &mut Frame) {
    frame.render_widget(SelectionOverlay, frame.area());
}

struct SelectionOverlay;

impl Widget for SelectionOverlay {
    fn render(self, _area: Rect, buf: &mut Buffer) {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let Some(viewport) = state.viewport.clone() else {
                return;
            };
            let Some(selection) = state.selection.as_mut() else {
                return;
            };
            if selection.context != viewport.context || !selection.dragged {
                return;
            }

            if selection.finalized && !selection.captured {
                if let Some(text) = extract_text(buf, &viewport, selection) {
                    selection.text = text;
                    selection.captured = true;
                }
            }

            highlight(buf, &viewport, selection);
        });
    }
}

fn highlight(buf: &mut Buffer, viewport: &SelectableViewport, selection: &TextSelection) {
    let rows = viewport.logical_row_range();
    let cols = viewport.logical_col_range();
    let (start, end) = selection.ordered_points();
    let first_row = start.row.max(rows.start);
    let last_row = end.row.min(rows.end.saturating_sub(1));
    if first_row > last_row {
        return;
    }

    for logical_row in first_row..=last_row {
        let Some((selection_start, selection_end)) = selection.col_range_for_row(logical_row)
        else {
            continue;
        };
        let first_col = selection_start.max(cols.start);
        let last_col = selection_end.min(cols.end.saturating_sub(1));
        if first_col > last_col {
            continue;
        }

        let screen_y = viewport.rect.y + (logical_row - viewport.row_offset) as u16;
        for logical_col in first_col..=last_col {
            let screen_x = viewport.rect.x + (logical_col - viewport.col_offset) as u16;
            if let Some(cell) = buf.cell_mut((screen_x, screen_y)) {
                let style = cell.style().add_modifier(Modifier::REVERSED);
                cell.set_style(style);
            }
        }
    }
}

fn extract_text(
    buf: &Buffer,
    viewport: &SelectableViewport,
    selection: &TextSelection,
) -> Option<String> {
    let (start, end) = selection.ordered_points();
    let rows = viewport.logical_row_range();
    let cols = viewport.logical_col_range();

    // Selection gestures originate from one visible viewport. Capturing only
    // when the complete finalized range is still visible prevents copying
    // unrelated text if the viewport changes before the next render.
    if start.row < rows.start || end.row >= rows.end {
        return None;
    }

    let mut lines = Vec::with_capacity(end.row - start.row + 1);
    for logical_row in start.row..=end.row {
        let (selection_start, selection_end) = selection.col_range_for_row(logical_row)?;
        if selection_start < cols.start || selection_end >= cols.end {
            return None;
        }
        let screen_y = viewport.rect.y + (logical_row - viewport.row_offset) as u16;
        let start_x = viewport.rect.x + (selection_start - viewport.col_offset) as u16;
        let end_x = viewport.rect.x + (selection_end - viewport.col_offset) as u16;
        lines.push(extract_screen_row(buf, screen_y, start_x, end_x));
    }
    Some(lines.join("\n"))
}

fn extract_screen_row(buf: &Buffer, y: u16, start_x: u16, end_x: u16) -> String {
    let mut text = String::new();
    let mut x = start_x;
    while x <= end_x {
        let Some(cell) = buf.cell((x, y)) else {
            break;
        };
        let symbol = cell.symbol();
        text.push_str(symbol);
        let width = UnicodeWidthStr::width(symbol).max(1).min(u16::MAX as usize) as u16;
        let next = x.saturating_add(width);
        if next <= x {
            break;
        }
        x = next;
    }
    text.trim_end_matches(' ').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{buffer::Buffer, layout::Rect, style::Style};

    fn viewport() -> SelectableViewport {
        SelectableViewport {
            rect: Rect::new(10, 5, 6, 3),
            row_offset: 20,
            col_offset: 4,
            context: "test".to_string(),
        }
    }

    #[test]
    fn maps_and_clamps_screen_coordinates_to_content_coordinates() {
        let viewport = viewport();
        assert_eq!(
            viewport.map_screen(12, 6, false),
            Some(TextPoint { row: 21, col: 6 })
        );
        assert_eq!(viewport.map_screen(2, 2, false), None);
        assert_eq!(
            viewport.map_screen(2, 99, true),
            Some(TextPoint { row: 22, col: 4 })
        );
    }

    #[test]
    fn multiline_selection_uses_visible_line_edges() {
        let selection = TextSelection {
            context: "test".to_string(),
            anchor: TextPoint { row: 20, col: 6 },
            cursor: TextPoint { row: 22, col: 7 },
            line_min_col: 4,
            line_max_col: 9,
            dragged: true,
            finalized: true,
            captured: false,
            pending_auto_copy: false,
            text: String::new(),
        };
        assert_eq!(selection.col_range_for_row(20), Some((6, 9)));
        assert_eq!(selection.col_range_for_row(21), Some((4, 9)));
        assert_eq!(selection.col_range_for_row(22), Some((4, 7)));
    }

    #[test]
    fn extracts_multiline_text_without_right_padding() {
        let viewport = SelectableViewport {
            rect: Rect::new(0, 0, 8, 3),
            row_offset: 0,
            col_offset: 0,
            context: "test".to_string(),
        };
        let selection = TextSelection {
            context: "test".to_string(),
            anchor: TextPoint { row: 0, col: 1 },
            cursor: TextPoint { row: 1, col: 2 },
            line_min_col: 0,
            line_max_col: 7,
            dragged: true,
            finalized: true,
            captured: false,
            pending_auto_copy: false,
            text: String::new(),
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, 8, 3));
        buf.set_string(0, 0, "abcdef", Style::default());
        buf.set_string(0, 1, "xyz", Style::default());

        assert_eq!(
            extract_text(&buf, &viewport, &selection).as_deref(),
            Some("bcdef\nxyz")
        );
    }
}
