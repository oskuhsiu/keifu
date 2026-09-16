//! UI components

pub mod commit_detail;
pub mod dialog;
pub mod file_diff_view;
pub mod file_preview;
pub mod graph_view;
pub mod help_popup;
pub mod search_dropdown;
pub mod status_bar;

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{
        Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Widget,
    },
    Frame,
};

use crate::{
    app::{App, AppMode, InputAction},
    config::{LayoutConfig, LayoutDirection},
    selection,
};

use self::{
    commit_detail::{CommitDetailWidget, FileListWidget},
    dialog::{BranchInfoPopup, ConfirmDialog, InputDialog},
    file_diff_view::FileDiffViewWidget,
    graph_view::GraphViewWidget,
    help_popup::HelpPopup,
    search_dropdown::{calculate_dropdown_height, SearchDropdown},
    status_bar::StatusBar,
};

/// Minimum terminal width required for rendering
const MIN_WIDTH: u16 = 20;
/// Minimum terminal height required for rendering
const MIN_HEIGHT: u16 = 6;

/// Minimum widget dimensions for safe rendering
pub const MIN_WIDGET_WIDTH: u16 = 12;
pub const MIN_WIDGET_HEIGHT: u16 = 3;

/// Border style for a pane depending on focus
pub fn pane_border_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

/// Standard pane block: rounded borders, focus-aware border and title styles
pub fn pane_block(title: &str, focused: bool) -> Block<'static> {
    let title_style = if focused {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    Block::default()
        .title(Span::styled(format!(" {} ", title), title_style))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(pane_border_style(focused))
}

/// Render a vertical scrollbar inside a bordered pane when content overflows
pub fn render_scrollbar(
    frame: &mut Frame,
    area: Rect,
    content_length: usize,
    viewport_length: usize,
    position: usize,
) {
    if content_length <= viewport_length || area.height <= 2 {
        return;
    }
    let mut state =
        ScrollbarState::new(content_length.saturating_sub(viewport_length)).position(position);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(None)
            .thumb_style(Style::default().fg(Color::DarkGray)),
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut state,
    );
}

/// Render a placeholder block when widget area is too small
pub fn render_placeholder_block(area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray));
    block.render(area, buf);
}

fn selected_graph_context(app: &App) -> String {
    let Some(index) = app.graph_list_state.selected() else {
        return "none".to_string();
    };
    let Some(node) = app.graph_layout.nodes.get(index) else {
        return format!("missing:{index}");
    };
    if node.is_uncommitted {
        "uncommitted".to_string()
    } else if let Some(commit) = &node.commit {
        commit.oid.to_string()
    } else {
        format!("connector:{index}")
    }
}

/// Render the main UI
pub fn draw(frame: &mut Frame, app: &mut App, layout_config: &LayoutConfig) {
    selection::begin_frame();

    // Update the diff cache once before rendering
    app.update_diff_cache();

    let area = frame.area();

    // Check minimum terminal size to prevent buffer overflow panics
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let msg = format!(
            "Terminal too small ({}x{}). Need at least {}x{}.",
            area.width, area.height, MIN_WIDTH, MIN_HEIGHT
        );
        let paragraph = Paragraph::new(msg).style(Style::default().fg(Color::Red));
        frame.render_widget(paragraph, area);
        return;
    }

    // Returning to the graph restores the commit-detail pane and its previous scroll position.
    file_preview::restore_commit_detail_when_normal(app);

    // FileDiff mode: full-screen diff view
    if let AppMode::FileDiff {
        content,
        rendered_lines,
        scroll_offset,
        horizontal_offset,
        file_index,
        file_list,
        ..
    } = &app.mode
    {
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);

        // Update viewport dimensions for scroll calculations (minus borders)
        app.diff_viewport_height = vertical[0].height.saturating_sub(2);
        app.diff_viewport_width = vertical[0].width.saturating_sub(2);

        let total_lines = rendered_lines.len();
        let scroll_position = *scroll_offset;

        frame.render_widget(
            FileDiffViewWidget::new(
                content,
                rendered_lines,
                *scroll_offset,
                *horizontal_offset,
                *file_index,
                file_list.len(),
            ),
            vertical[0],
        );
        render_scrollbar(
            frame,
            vertical[0],
            total_lines,
            app.diff_viewport_height as usize,
            scroll_position,
        );

        app.layout.status_bar = vertical[1];
        let status_bar = StatusBar::new(app);
        app.status_hints = status_bar.hint_regions(vertical[1]);
        frame.render_widget(status_bar, vertical[1]);

        let selectable = vertical[0].inner(Margin {
            vertical: 1,
            horizontal: 1,
        });
        selection::set_viewport(
            selectable,
            *scroll_offset,
            *horizontal_offset,
            format!(
                "full-diff:{}:{}",
                selected_graph_context(app),
                content.path.to_string_lossy()
            ),
        );
        selection::render_overlay(frame);
        return;
    }

    // Split the screen into the configurable main area and the fixed 1-row status bar.
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let main_area = vertical[0];
    let status_area = vertical[1];

    // Split the main area into graph / commit detail / files according to config.
    // The status bar above is excluded from these percentages.
    let direction = match layout_config.direction {
        LayoutDirection::Vertical => Direction::Vertical,
        LayoutDirection::Horizontal => Direction::Horizontal,
    };
    let [graph_percent, commit_percent, files_percent] = layout_config.percentages();
    let content = Layout::default()
        .direction(direction)
        .constraints([
            Constraint::Percentage(graph_percent),
            Constraint::Percentage(commit_percent),
            Constraint::Percentage(files_percent),
        ])
        .split(main_area);

    let graph_area = content[0];
    let commit_area = content[1];
    let files_area = content[2];

    // Record pane regions for mouse hit-testing
    app.layout = crate::app::LayoutMap {
        graph: graph_area,
        commit_detail: commit_area,
        files: files_area,
        status_bar: status_area,
    };

    // The middle pane becomes a live file-diff preview for the entire FileSelect mode.
    // It changes back to Commit Detail only after FileSelect is exited to the graph.
    let showing_file_preview = matches!(app.mode, AppMode::FileSelect { .. });
    if showing_file_preview {
        file_preview::render(frame, app, commit_area);
    } else {
        app.detail_viewport_height = commit_area.height.saturating_sub(2);
        let commit_widget = CommitDetailWidget::new(app);
        app.detail_content_height =
            commit_widget.estimated_height(commit_area.width.saturating_sub(2));
        app.scroll_detail(0);
        frame.render_widget(commit_widget.with_scroll(app.detail_scroll), commit_area);
    }

    let files_widget = FileListWidget::new(app);
    app.files_pane_scroll = files_widget.scroll_offset(files_area);

    // Render widgets
    frame.render_stateful_widget(
        GraphViewWidget::new(app, graph_area.width),
        graph_area,
        &mut app.graph_list_state,
    );
    frame.render_widget(files_widget, files_area);

    // Scrollbars
    render_scrollbar(
        frame,
        graph_area,
        app.graph_layout.nodes.len(),
        graph_area.height.saturating_sub(2) as usize,
        app.graph_list_state.offset(),
    );
    render_scrollbar(
        frame,
        commit_area,
        app.detail_content_height as usize,
        app.detail_viewport_height as usize,
        app.detail_scroll as usize,
    );

    let status_bar = StatusBar::new(app);
    app.status_hints = status_bar.hint_regions(status_area);
    frame.render_widget(status_bar, status_area);

    let selectable = commit_area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });
    match &app.mode {
        AppMode::Normal => selection::set_viewport(
            selectable,
            app.detail_scroll as usize,
            0,
            format!("commit-detail:{}", selected_graph_context(app)),
        ),
        AppMode::FileSelect {
            selected_index,
            file_list,
        } => {
            let path = file_list
                .get(*selected_index)
                .map(|file| file.path.to_string_lossy())
                .unwrap_or_default();
            selection::set_viewport(
                selectable,
                app.detail_scroll as usize,
                0,
                format!(
                    "inline-diff:{}:{}:{}",
                    selected_graph_context(app),
                    selected_index,
                    path
                ),
            );
        }
        _ => {}
    }
    selection::render_overlay(frame);

    // Branch info popup (when multiple branches exist on selected node)
    render_branch_info_popup(frame, app, graph_area);

    // Popups
    match &app.mode {
        AppMode::Help => {
            let popup_area = centered_rect(60, 70, area);
            let viewport_height = popup_area.height.saturating_sub(2) as usize;
            let line_count = HelpPopup::line_count();
            let max_scroll = line_count.saturating_sub(viewport_height) as u16;
            app.help_scroll = app.help_scroll.min(max_scroll);
            frame.render_widget(HelpPopup::new(app.help_scroll), popup_area);
            render_scrollbar(
                frame,
                popup_area,
                line_count,
                viewport_height,
                app.help_scroll as usize,
            );
        }
        AppMode::Input {
            input,
            action: InputAction::Search,
            ..
        } => {
            // Search dropdown at bottom of screen
            let results = app.search_results();
            let height = calculate_dropdown_height(results.len());
            let popup_area = bottom_rect(60, height, area);
            frame.render_widget(
                SearchDropdown::new(
                    input,
                    results,
                    &app.branch_positions,
                    app.search_selection(),
                ),
                popup_area,
            );
        }
        AppMode::Input { title, input, .. } => {
            let popup_area = centered_rect(50, 20, area);
            frame.render_widget(InputDialog::new(title, input), popup_area);
        }
        AppMode::Confirm { message, .. } => {
            let popup_area = centered_rect(50, 20, area);
            frame.render_widget(ConfirmDialog::new(message), popup_area);
        }
        _ => {}
    }
}

/// Render branch info popup when multiple branches exist on selected node
fn render_branch_info_popup(frame: &mut Frame, app: &App, graph_area: Rect) {
    let selected_branches = app.selected_node_branches();

    // Only show popup in Normal mode with multiple branches
    if selected_branches.len() <= 1 || !matches!(app.mode, crate::app::AppMode::Normal) {
        return;
    }

    let popup_height = (selected_branches.len() + 2).min(10) as u16;
    let max_branch_len = selected_branches
        .iter()
        .map(|b| b.len())
        .max()
        .unwrap_or(10);
    let popup_width = (max_branch_len + 6).min(50) as u16;

    // Calculate selected row's screen position (add 1 for border)
    let selected_idx = app.graph_list_state.selected().unwrap_or(0);
    let offset = app.graph_list_state.offset();
    let selected_screen_y = graph_area.y + 1 + selected_idx.saturating_sub(offset) as u16;

    // Position popup at right side of graph area
    let popup_x = graph_area.x + graph_area.width.saturating_sub(popup_width + 2);
    let default_popup_y = graph_area.y + 1;

    // Shift down only if popup overlaps with selected row
    let overlaps_selected =
        selected_screen_y >= default_popup_y && selected_screen_y < default_popup_y + popup_height;
    let popup_y = if overlaps_selected {
        (selected_screen_y + 1).min(graph_area.y + graph_area.height - popup_height)
    } else {
        default_popup_y
    };

    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);
    frame.render_widget(
        BranchInfoPopup::new(&selected_branches, app.selected_branch_name()),
        popup_area,
    );
}

/// Calculate a centered rectangle
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Calculate a bottom-aligned rectangle (for dropdowns)
fn bottom_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let clamped_height = height.min(area.height.saturating_sub(2));
    let y = area.y + area.height.saturating_sub(clamped_height + 1);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(area);

    Rect::new(horizontal[1].x, y, horizontal[1].width, clamped_height)
}
