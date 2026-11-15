use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// User-defined scan path with ignore rules (mode 3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomPath {
    pub path: String,
    /// Minimum age in days for entries under this path (default 90).
    #[serde(default = "default_min_age")]
    pub min_age_days: u64,
    /// Glob-like suffixes / names to ignore (matched as substring of file name).
    #[serde(default)]
    pub ignore: Vec<String>,
}

impl Default for CustomPath {
    fn default() -> Self {
        Self {
            path: String::new(),
            min_age_days: default_min_age(),
            ignore: Vec::new(),
        }
    }
}

fn default_min_age() -> u64 {
    90
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Age threshold (days) for the "Stale files" mode.
    #[serde(default = "default_min_age")]
    pub min_age_days: u64,
    /// Scan ~/.cache as junk?
    #[serde(default = "default_true")]
    pub scan_cache: bool,
    /// Scan ~/.local/share/Trash as junk?
    #[serde(default = "default_true")]
    pub scan_trash: bool,
    /// Detect empty dirs and broken symlinks under scanned roots?
    #[serde(default = "default_true")]
    pub detect_empty_and_broken: bool,
    /// Never scan or trash these paths (in addition to built-in blacklist).
    #[serde(default)]
    pub protected_paths: Vec<String>,
    /// User-defined scan paths (mode 3).
    #[serde(default)]
    pub custom_paths: Vec<CustomPath>,
    /// Names never to report (always ignored).
    #[serde(default = "default_ignore_names")]
    pub ignore_names: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn default_ignore_names() -> Vec<String> {
    vec![
        ".git".into(),
        ".gitignore".into(),
        "node_modules".into(),
        ".venv".into(),
        "venv".into(),
        "__pycache__".into(),
        ".cargo".into(),
        ".rustup".into(),
        ".gnupg".into(),
        ".ssh".into(),
        ".password-store".into(),
        ".config".into(),
        ".mozilla".into(),
        ".thunderbird".into(),
        "Desktop".into(),
        "Documents".into(),
        "Downloads".into(),
        "Music".into(),
        "Pictures".into(),
        "Videos".into(),
    ]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            min_age_days: default_min_age(),
            scan_cache: true,
            scan_trash: true,
            detect_empty_and_broken: true,
            protected_paths: Vec::new(),
            custom_paths: Vec::new(),
            ignore_names: default_ignore_names(),
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("archfinder")
            .join("config.toml")
    }

    /// Load config, falling back to defaults. Returns (config, path_used).
    pub fn load() -> (Self, Option<PathBuf>) {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str(&text) {
                Ok(cfg) => (cfg, Some(path)),
                Err(_) => (Self::default(), Some(path)),
            },
            Err(_) => (Self::default(), None),
        }
    }

    /// Built-in + user protected paths (prefix match; "/" handled separately in action_allowed).
    pub fn protected(&self) -> Vec<PathBuf> {
        let builtin = ["/boot", "/dev", "/etc", "/proc", "/sys", "/usr", "/var"];
        builtin
            .iter()
            .map(PathBuf::from)
            .chain(self.protected_paths.iter().map(PathBuf::from))
            .collect()
    }
}
