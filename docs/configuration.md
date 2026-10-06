# Configuration

All settings are optional. Keifu reads `keifu/config.toml` under the native OS
configuration directory:

| OS | File |
| --- | --- |
| macOS | `~/Library/Application Support/keifu/config.toml` |
| Linux | `$XDG_CONFIG_HOME/keifu/config.toml`, or `~/.config/keifu/config.toml` when unset |
| Windows | `%APPDATA%\keifu\config.toml` |

On macOS, `~/.config/keifu/config.toml` is **not** read. Create the native directory
and file manually when needed. Restart Keifu after editing. Missing files use
defaults; unreadable or invalid files print a warning with the path and then use
defaults. Existing explicit settings always override defaults and are never
rewritten by an upgrade.

## Auto-refresh

By default, Keifu refreshes local Git state every **300 seconds (5 minutes)**.
Remote fetch is **manual by default**: press `f` in Normal mode to fetch `origin`.
Press `R` to refresh local data immediately; it does not contact the remote.

```toml
[refresh]
# Refresh commits, branches, and working-tree state locally.
auto_refresh = true

# Seconds between local refreshes (default: 300, minimum: 1).
refresh_interval = 300

# Do not contact origin automatically (default: false).
auto_fetch = false

# Used ONLY when auto_fetch = true (default: 3600, minimum: 10).
fetch_interval = 3600
```

To opt into automatic hourly fetch, set `auto_fetch = true`. A completed fetch
updates the retry clock whether it succeeds or fails; failures wait for the
configured interval rather than immediately starting another process. Manual
`f` can retry immediately, provided no fetch is already running.

Repository refresh is deferred while viewing files/diffs, help, or dialogs, and
while recent input is being handled. These intervals are therefore minimum
spacing, not strict wall-clock appointments. Successful Git operations can also
request a local refresh.

If an older config explicitly contains `refresh_interval = 10` or
`auto_fetch = true`, remove or change those values to adopt the new defaults.
Merge these fields into an existing `[refresh]` table instead of adding a second
table with the same name.

### Idle rendering

Git refresh and terminal rendering are separate. Idle polling does not rebuild
the UI: input, resize, background results, message expiration, and clipboard
selection clearing request redraws. Normal mode also repaints relative dates
once per minute without reading Git or contacting the network. Background diff
results continue to be received even when no frame is drawn.

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

By default, keifu compacts merged side history when it can prove that the history
has one unambiguous external merge target. Folded commits remain visible and use
a `◇` marker on the target lane. Keifu keeps the full topology when the side
history still has a live branch ref, is merged into multiple external targets,
uses an octopus merge, or reaches beyond the loaded history window.

```toml
[graph]
# Fold safely-owned merged history into the target lane (default: true)
compact_merged_history = true
```

Press `z` in the TUI to toggle compact merged history for the current session.

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
same OSC 52 clipboard support as the existing copy commands, then clears the
selection highlight:

```toml
[selection]
auto_copy = true
clear_after_copy = true
```

Set `clear_after_copy = false` to keep the highlight after a successful automatic
copy. Set `auto_copy = false` to disable copy-on-release entirely; the selection
then remains highlighted until it is changed or cleared. Press `y` to copy an
existing selection manually. Manual `y` copy does not clear the selection. When
there is no selection, Normal mode keeps the existing `y` behavior for copying
the selected commit hash.

If automatic copy fails, the selection is kept even when `clear_after_copy = true`.

### Options

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `refresh.auto_refresh` | bool | `true` | Enable auto-refresh for local state (commits, branches, working tree) |
| `refresh.refresh_interval` | integer | `300` | Interval in seconds for local refresh (minimum: 1) |
| `refresh.auto_fetch` | bool | `false` | Opt into automatic fetch from origin; otherwise use `f` |
| `refresh.fetch_interval` | integer | `3600` | Interval in seconds when auto-fetch is enabled (minimum: 10) |
| `graph.show_remote_branches` | bool | `true` | Show remote branches and commits reachable only from remote branches |
| `graph.show_tags` | bool | `true` | Show tag labels on commits |
| `graph.compact_merged_history` | bool | `true` | Fold uniquely-owned merged side history into its target lane |
| `layout.direction` | `vertical` / `horizontal` | `vertical` | Direction of the graph / commit / files pane split |
| `layout.graph` | integer | `50` | Percentage of the main content area used by the graph pane |
| `layout.commit` | integer | `25` | Percentage of the main content area used by commit detail |
| `layout.files` | integer | `25` | Percentage of the main content area used by changed files |
| `selection.auto_copy` | bool | `true` | Copy a completed Keifu text selection to the clipboard on mouse release |
| `selection.clear_after_copy` | bool | `true` | Clear the selection after a successful automatic copy |

### Disabling auto-refresh

To disable automatic repository updates entirely:

```toml
[refresh]
auto_refresh = false
auto_fetch = false
```

You can still manually refresh with `R` and fetch with `f`.
