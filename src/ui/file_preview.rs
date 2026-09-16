//! Inline file diff preview shown while selecting files.

use std::cell::RefCell;
use std::path::PathBuf;

use git2::Oid;
use ratatui::{layout::Rect, text::Line, widgets::Paragraph, Frame};

use crate::{
    app::{App, AppMode},
    git::FileDiffContent,
};

use super::{
    file_diff_view::{build_highlighted_lines, FileDiffViewWidget},
    pane_block,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewTarget {
    Uncommitted,
    Commit(Oid),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreviewKey {
    repo_path: String,
    target: PreviewTarget,
    path: PathBuf,
}

enum PreviewData {
    Ready {
        content: FileDiffContent,
        rendered_lines: Vec<Line<'static>>,
    },
    Error(String),
}

struct PreviewCache {
    key: PreviewKey,
    data: PreviewData,
}

struct PreviewSession {
    original_detail_scroll: u16,
    cache: Option<PreviewCache>,
}

thread_local! {
    static PREVIEW_SESSION: RefCell<Option<PreviewSession>> = RefCell::new(None);
}

fn preview_key(app: &App, path: PathBuf) -> Option<PreviewKey> {
    let node = app
        .graph_list_state
        .selected()
        .and_then(|idx| app.graph_layout.nodes.get(idx))?;

    let target = if node.is_uncommitted {
        PreviewTarget::Uncommitted
    } else {
        PreviewTarget::Commit(node.commit.as_ref()?.oid)
    };

    Some(PreviewKey {
        repo_path: app.repo_path.clone(),
        target,
        path,
    })
}

fn load_preview(app: &App, key: &PreviewKey) -> PreviewData {
    let result = match key.target {
        PreviewTarget::Uncommitted => {
            FileDiffContent::from_working_tree(&app.repo.repo, &key.path)
        }
        PreviewTarget::Commit(oid) => FileDiffContent::from_commit(&app.repo.repo, oid, &key.path),
    };

    match result {
        Ok(content) => {
            let (rendered_lines, _) = build_highlighted_lines(&content);
            PreviewData::Ready {
                content,
                rendered_lines,
            }
        }
        Err(error) => PreviewData::Error(error.to_string()),
    }
}

/// Restore the commit-detail scroll position when leaving file selection and
/// returning to the normal graph view. Full-screen FileDiff keeps the preview
/// session alive so Esc can return to the same inline preview.
pub fn restore_commit_detail_when_normal(app: &mut App) {
    if !matches!(app.mode, AppMode::Normal) {
        return;
    }

    let original_scroll = PREVIEW_SESSION.with(|session| {
        session
            .borrow_mut()
            .take()
            .map(|session| session.original_detail_scroll)
    });

    if let Some(scroll) = original_scroll {
        app.detail_scroll = scroll;
    }
}

/// Render the currently selected file's diff inside the commit-detail pane.
/// The preview is cached by repository + graph target + file path, so Git I/O
/// only happens when the selected file or graph target changes.
pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    let (selected_index, file_count, path) = match &app.mode {
        AppMode::FileSelect {
            selected_index,
            file_list,
        } => {
            let Some(file) = file_list.get(*selected_index) else {
                frame.render_widget(
                    Paragraph::new(" No file selected").block(pane_block("File Diff", false)),
                    area,
                );
                return;
            };
            (*selected_index, file_list.len(), file.path.clone())
        }
        _ => return,
    };

    let Some(key) = preview_key(app, path) else {
        frame.render_widget(
            Paragraph::new(" Diff not available").block(pane_block("File Diff", false)),
            area,
        );
        return;
    };

    let mut reset_scroll = false;
    PREVIEW_SESSION.with(|session| {
        let mut session = session.borrow_mut();
        if session.is_none() {
            *session = Some(PreviewSession {
                original_detail_scroll: app.detail_scroll,
                cache: None,
            });
            reset_scroll = true;
        }

        let preview_session = session.as_mut().expect("preview session initialized");
        let needs_reload = preview_session
            .cache
            .as_ref()
            .map_or(true, |cache| cache.key != key);

        if needs_reload {
            preview_session.cache = Some(PreviewCache {
                key: key.clone(),
                data: load_preview(app, &key),
            });
            reset_scroll = true;
        }
    });

    if reset_scroll {
        app.detail_scroll = 0;
    }

    app.detail_viewport_height = area.height.saturating_sub(2);
    app.detail_content_height = PREVIEW_SESSION.with(|session| {
        let session = session.borrow();
        let line_count = session
            .as_ref()
            .and_then(|session| session.cache.as_ref())
            .map(|cache| match &cache.data {
                PreviewData::Ready { rendered_lines, .. } => rendered_lines.len(),
                PreviewData::Error(_) => 1,
            })
            .unwrap_or(0);
        line_count.min(u16::MAX as usize) as u16
    });
    app.scroll_detail(0);

    PREVIEW_SESSION.with(|session| {
        let session = session.borrow();
        let Some(cache) = session.as_ref().and_then(|session| session.cache.as_ref()) else {
            return;
        };

        match &cache.data {
            PreviewData::Ready {
                content,
                rendered_lines,
            } => frame.render_widget(
                FileDiffViewWidget::new(
                    content,
                    rendered_lines,
                    app.detail_scroll as usize,
                    0,
                    selected_index,
                    file_count,
                ),
                area,
            ),
            PreviewData::Error(error) => frame.render_widget(
                Paragraph::new(format!(" Cannot load diff: {error}"))
                    .block(pane_block("File Diff", false)),
                area,
            ),
        }
    });
}
