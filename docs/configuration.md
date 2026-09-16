# Configuration

keifu can be configured via `~/.config/keifu/config.toml`. All settings are optional.

## Auto-refresh

By default, keifu automatically refreshes the commit graph every 10 seconds and fetches from origin every 60 seconds.

```toml
[refresh]
# Enable auto-refresh for local state (default: true)
auto_refresh = true

# Interval in seconds for local refresh (default: 10, minimum: 1)
refresh_interval = 10

# Enable auto-fetch from origin (default: true)
auto_fetch = true

# Interval in seconds for remote fetch (default: 60, minimum: 10)
fetch_interval = 60
```

## Graph display

By default, keifu shows remote branches and commits that are reachable only from
remote branches. You can hide them by default:

```toml
[graph]
# Show remote branches by default (default: true)
show_remote_branches = false
```

Press `o` in the TUI to toggle remote branches for the current session.

By default, keifu shows tag labels on commits. You can hide them by default:

```toml
[graph]
# Show tag labels by default (default: true)
show_tags = false
```

Press `t` in the TUI to toggle tags for the current session.

## Layout

The graph, commit detail, and changed-files panes can be arranged vertically or
horizontally. By default, they are stacked vertically with a 50 / 25 / 25 split:

```toml
[layout]
direction = "vertical"
graph = 50
commit = 25
files = 25
```

The percentages apply only to the main content area. The one-row bottom status
line is kept separate and is not included in the 100% total.

To place the panes side by side instead:

```toml
[layout]
direction = "horizontal"
graph = 50
commit = 25
files = 25
```

`graph + commit + files` must equal `100`, and each value must be greater than
zero. Invalid combinations fall back to the default `50 / 25 / 25` split.

This setting controls pane layout only. It does not hide panes or change the
contents rendered inside graph, commit detail, or files.

## Text selection

Commit Detail, the inline file-diff preview, and the full-screen diff support
pane-aware mouse drag selection. Selection is limited to the pane content: it
does not include borders, titles, scrollbars, or neighboring panes. The logical
selection is preserved when the pane scrolls.

By default, releasing the mouse immediately copies the selected text using the
same OSC 52 clipboard support as the existing copy commands:

```toml
[selection]
auto_copy = true
```

Set `auto_copy = false` to keep the selection highlighted without copying it on
mouse release. Press `y` to copy an existing selection manually. When there is
no selection, Normal mode keeps the existing `y` behavior for copying the
selected commit hash.

### Options

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `refresh.auto_refresh` | bool | `true` | Enable auto-refresh for local state (commits, branches, working tree) |
| `refresh.refresh_interval` | integer | `10` | Interval in seconds for local refresh (minimum: 1) |
| `refresh.auto_fetch` | bool | `true` | Enable auto-fetch from origin |
| `refresh.fetch_interval` | integer | `60` | Interval in seconds for remote fetch (minimum: 10) |
| `graph.show_remote_branches` | bool | `true` | Show remote branches and commits reachable only from remote branches |
| `graph.show_tags` | bool | `true` | Show tag labels on commits |
| `layout.direction` | `vertical` / `horizontal` | `vertical` | Direction of the graph / commit / files pane split |
| `layout.graph` | integer | `50` | Percentage of the main content area used by the graph pane |
| `layout.commit` | integer | `25` | Percentage of the main content area used by commit detail |
| `layout.files` | integer | `25` | Percentage of the main content area used by changed files |
| `selection.auto_copy` | bool | `true` | Copy a completed Keifu text selection to the clipboard on mouse release |

### Disabling auto-refresh

To disable automatic updates entirely:

```toml
[refresh]
auto_refresh = false
auto_fetch = false
```

You can still manually refresh with `R` and fetch with `f`.
