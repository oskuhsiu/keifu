//! keifu: a TUI tool that shows Git commit graphs

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Parser;
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind};

use keifu::{
    app::{App, AppMode},
    config::Config,
    debug_server,
    event::{EventReader, InputEvent},
    git::configure_git_extensions,
    keybindings::map_key_to_action,
    logging, mouse, selection, tui, ui,
};

const MAINTENANCE_IDLE_PERIOD: Duration = Duration::from_millis(250);
// Commit Detail contains relative dates. They need an occasional repaint,
// not ten full UI reconstructions per second and not a Git/network refresh.
const RELATIVE_DATE_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Parser)]
#[command(name = "keifu")]
#[command(
    version,
    about = "A TUI tool to visualize Git commit graphs with branch genealogy"
)]
struct Cli {
    /// Append debug logs and a perf summary on exit to this file
    /// (level via KEIFU_LOG, default "debug")
    #[arg(long, value_name = "PATH")]
    log_file: Option<PathBuf>,

    /// Listen for debug commands (NDJSON over TCP, e.g. 127.0.0.1:7167)
    #[arg(long, value_name = "ADDR")]
    debug_listen: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(path) = &cli.log_file {
        logging::init(path)?;
    }
    let debug_rx = match &cli.debug_listen {
        Some(addr) => Some(debug_server::spawn(addr)?),
        None => None,
    };

    // Restore the terminal on panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = tui::restore();
        original_hook(panic_info);
    }));

    configure_git_extensions()?;

    // Load once: the renderer and application must use the same configuration.
    let config = Config::load();
    let layout_config = config.layout.clone();
    let clear_selection_after_copy = config.selection.clear_after_copy;
    selection::configure(config.selection.auto_copy);
    let mut app = App::with_config(config)?;

    let mut terminal = tui::init()?;
    let mut event_reader = match EventReader::new() {
        Ok(reader) => reader,
        Err(error) => {
            let _ = tui::restore();
            return Err(error);
        }
    };
    let mut last_user_input = Instant::now();
    let mut last_relative_date = Instant::now();

    // The closure lets the terminal be restored on errors as well as normal exit.
    let result = (|| -> Result<()> {
        while !app.should_quit {
            let allow_refresh = last_user_input.elapsed() >= MAINTENANCE_IDLE_PERIOD;
            app.tick(allow_refresh);

            if last_relative_date.elapsed() >= RELATIVE_DATE_INTERVAL {
                last_relative_date = Instant::now();
                if matches!(app.mode, AppMode::Normal) {
                    app.request_redraw();
                }
            }

            if app.take_redraw_request() {
                let draw_started = Instant::now();
                terminal.draw(|frame| ui::draw(frame, &mut app, &layout_config))?;
                app.perf.record("draw", draw_started.elapsed());

                match selection::flush_auto_copy() {
                    Ok(true) if clear_selection_after_copy => {
                        selection::clear();
                        app.request_redraw();
                        // Remove the highlight immediately, without waiting for input.
                        continue;
                    }
                    Ok(_) => {}
                    Err(error) => {
                        app.set_message(format!("Copy failed: {error}"));
                        continue;
                    }
                }
            }

            let timeout = if debug_rx.is_some() {
                app.poll_timeout().min(Duration::from_millis(100))
            } else {
                app.poll_timeout()
            };
            let batch = event_reader.poll_events_with_timeout(timeout)?;
            if batch.had_input() {
                last_user_input = Instant::now();
            }
            let raw_count = batch.raw_count();
            let retained_count = batch.retained_count();
            if raw_count > 0 || retained_count > 0 {
                tracing::trace!(raw_count, retained_count, "input batch normalized");
            }
            let events = batch.into_events();
            if !events.is_empty() {
                let events_started = Instant::now();
                for event in events {
                    match event {
                        InputEvent::Terminal(Event::Key(key)) => {
                            if key.kind == KeyEventKind::Release {
                                continue;
                            }
                            // Do not let a stale selection consume text typed into a dialog.
                            let can_copy_selection = matches!(
                                app.mode,
                                AppMode::Normal | AppMode::FileSelect { .. } | AppMode::FileDiff { .. }
                            );
                            if can_copy_selection
                                && key.modifiers == KeyModifiers::NONE
                                && key.code == KeyCode::Char('y')
                                && selection::has_selected_text()
                            {
                                match selection::copy_selected() {
                                    Ok(true) => app.set_message("Copied selection"),
                                    Ok(false) => {}
                                    Err(error) => app.set_message(format!("Copy failed: {error}")),
                                }
                                continue;
                            }
                            if let Some(action) = map_key_to_action(key, &app.mode) {
                                if let Err(error) = app.handle_action(action) {
                                    app.show_error(error.to_string());
                                }
                            }
                        }
                        InputEvent::Terminal(Event::Mouse(event)) => {
                            if event.kind != MouseEventKind::Moved {
                                mouse::handle_mouse(&mut app, event);
                                app.request_redraw();
                            }
                        }
                        InputEvent::Scroll { mouse: event, steps } => {
                            mouse::handle_scroll(&mut app, event, steps);
                            app.request_redraw();
                        }
                        InputEvent::Terminal(Event::Resize(_, _)) => app.request_redraw(),
                        _ => {}
                    }
                    if app.should_quit {
                        break;
                    }
                }
                app.perf.record("events", events_started.elapsed());
            }

            if let Some(rx) = &debug_rx {
                while let Ok(command) = rx.try_recv() {
                    let size = terminal.size()?;
                    let response = debug_server::handle_request(
                        &mut app,
                        size.width,
                        size.height,
                        command.request,
                    );
                    let _ = command.reply.send(response);
                    app.request_redraw();
                }
            }
        }
        Ok(())
    })();

    app.perf.log_summary();
    let restored = tui::restore();
    result.and(restored)
}
