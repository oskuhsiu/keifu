//! Configuration management

use std::fs;

use serde::Deserialize;

/// Application configuration
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub refresh: RefreshConfig,
    pub graph: GraphConfig,
    pub layout: LayoutConfig,
    pub selection: SelectionConfig,
}

/// Commit graph display configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GraphConfig {
    /// Show remote branches and commits only reachable from remote branches
    pub show_remote_branches: bool,
    pub show_tags: bool,
    /// Fold uniquely-owned merged side history into its target lane.
    pub compact_merged_history: bool,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            show_remote_branches: true,
            show_tags: true,
            compact_merged_history: true,
        }
    }
}

/// Main pane layout direction
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutDirection {
    #[default]
    Vertical,
    Horizontal,
}

/// Main pane layout configuration.
///
/// Percentages apply to the main area only. The one-row bottom status line is
/// excluded before the graph / commit detail / files split is calculated.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LayoutConfig {
    pub direction: LayoutDirection,
    pub graph: u16,
    pub commit: u16,
    pub files: u16,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            direction: LayoutDirection::Vertical,
            graph: 50,
            commit: 25,
            files: 25,
        }
    }
}

impl LayoutConfig {
    /// Return validated graph / commit / files percentages.
    ///
    /// Invalid values fall back to the default 50 / 25 / 25 layout. A zero
    /// percentage is considered invalid because hiding panes is not part of
    /// the layout configuration contract.
    pub fn percentages(&self) -> [u16; 3] {
        let total = u32::from(self.graph) + u32::from(self.commit) + u32::from(self.files);
        if self.graph > 0 && self.commit > 0 && self.files > 0 && total == 100 {
            [self.graph, self.commit, self.files]
        } else {
            let default = Self::default();
            [default.graph, default.commit, default.files]
        }
    }
}

/// Pane-aware text selection configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SelectionConfig {
    /// Automatically copy a completed mouse selection to the clipboard.
    pub auto_copy: bool,
    /// Clear the highlight after a successful automatic copy.
    pub clear_after_copy: bool,
}

impl Default for SelectionConfig {
    fn default() -> Self {
        Self {
            auto_copy: true,
            clear_after_copy: true,
        }
    }
}

/// Auto-refresh configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RefreshConfig {
    /// Enable auto-refresh for local state (commits, branches, working tree)
    pub auto_refresh: bool,
    /// Interval in seconds for local refresh (minimum: 1, default: 300)
    #[serde(deserialize_with = "deserialize_refresh_interval")]
    pub refresh_interval: u64,
    /// Enable automatic remote fetch (default: false; use f to fetch manually)
    pub auto_fetch: bool,
    /// Interval in seconds when auto-fetch is enabled (minimum: 10, default: 3600)
    #[serde(deserialize_with = "deserialize_fetch_interval")]
    pub fetch_interval: u64,
}

impl Default for RefreshConfig {
    fn default() -> Self {
        Self {
            auto_refresh: true,
            refresh_interval: 300,
            auto_fetch: false,
            fetch_interval: 3600,
        }
    }
}

fn deserialize_refresh_interval<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = u64::deserialize(deserializer)?;
    Ok(value.max(1))
}

fn deserialize_fetch_interval<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = u64::deserialize(deserializer)?;
    Ok(value.max(10))
}

impl Config {
    /// Load from the OS configuration directory. On macOS this is
    /// ~/Library/Application Support/keifu/config.toml, NOT ~/.config/keifu.
    /// Missing files use defaults; unreadable/invalid files also emit a warning.
    pub fn load() -> Self {
        let Some(path) = dirs::config_dir().map(|p| p.join("keifu/config.toml")) else {
            return Self::default();
        };
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(error) => {
                eprintln!("keifu: cannot read {}: {error}; using defaults", path.display());
                return Self::default();
            }
        };
        match toml::from_str(&content) {
            Ok(config) => config,
            Err(error) => {
                eprintln!("keifu: invalid config {}: {error}; using defaults", path.display());
                Self::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LayoutConfig, LayoutDirection, SelectionConfig};

    #[test]
    fn layout_defaults_to_vertical_50_25_25() {
        let layout = LayoutConfig::default();
        assert_eq!(layout.direction, LayoutDirection::Vertical);
        assert_eq!(layout.percentages(), [50, 25, 25]);
    }

    #[test]
    fn valid_layout_percentages_are_preserved() {
        let layout = LayoutConfig {
            direction: LayoutDirection::Horizontal,
            graph: 60,
            commit: 20,
            files: 20,
        };
        assert_eq!(layout.percentages(), [60, 20, 20]);
    }

    #[test]
    fn invalid_layout_percentages_fall_back_to_defaults() {
        let invalid_total = LayoutConfig {
            graph: 60,
            commit: 30,
            files: 30,
            ..LayoutConfig::default()
        };
        assert_eq!(invalid_total.percentages(), [50, 25, 25]);

        let hidden_pane = LayoutConfig {
            graph: 50,
            commit: 50,
            files: 0,
            ..LayoutConfig::default()
        };
        assert_eq!(hidden_pane.percentages(), [50, 25, 25]);
    }

    #[test]
    fn selection_copy_defaults_to_auto_and_clear() {
        let selection = SelectionConfig::default();
        assert!(selection.auto_copy);
        assert!(selection.clear_after_copy);
    }

    #[test]
    fn compact_merged_history_defaults_to_true() {
        assert!(super::GraphConfig::default().compact_merged_history);
    }

    #[test]
    fn refresh_defaults_are_local_five_minutes_and_manual_fetch() {
        for text in ["", "[refresh]\n", "[refresh]\nauto_refresh = true\n"] {
            let config: super::Config = toml::from_str(text).unwrap();
            assert!(config.refresh.auto_refresh);
            assert_eq!(config.refresh.refresh_interval, 300);
            assert!(!config.refresh.auto_fetch);
            assert_eq!(config.refresh.fetch_interval, 3600);
        }
    }

    #[test]
    fn explicit_refresh_and_fetch_settings_are_preserved() {
        let config: super::Config = toml::from_str(
            "[refresh]\nauto_refresh = false\nrefresh_interval = 30\nauto_fetch = true\nfetch_interval = 120\n",
        ).unwrap();
        assert!(!config.refresh.auto_refresh);
        assert_eq!(config.refresh.refresh_interval, 30);
        assert!(config.refresh.auto_fetch);
        assert_eq!(config.refresh.fetch_interval, 120);
    }

    #[test]
    fn zero_intervals_still_use_the_existing_minimums() {
        let config: super::Config = toml::from_str(
            "[refresh]\nrefresh_interval = 0\nfetch_interval = 0\n",
        ).unwrap();
        assert_eq!(config.refresh.refresh_interval, 1);
        assert_eq!(config.refresh.fetch_interval, 10);
    }
}
