//! XDG path resolution for config / cache / state.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ConfigPaths {
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub state_dir: PathBuf,
}

impl ConfigPaths {
    /// Resolve from XDG variables, falling back to `$HOME/.config/mc-rs` etc.
    /// `$MC_RS_CONFIG_DIR` overrides the config dir.
    #[must_use]
    pub fn discover() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));

        let config_dir = std::env::var_os("MC_RS_CONFIG_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(|x| PathBuf::from(x).join("mc-rs")))
            .unwrap_or_else(|| home.join(".config").join("mc-rs"));

        let cache_dir = std::env::var_os("XDG_CACHE_HOME")
            .map(|x| PathBuf::from(x).join("mc-rs"))
            .unwrap_or_else(|| home.join(".cache").join("mc-rs"));

        let state_dir = std::env::var_os("XDG_STATE_HOME")
            .map(|x| PathBuf::from(x).join("mc-rs"))
            .unwrap_or_else(|| home.join(".local").join("state").join("mc-rs"));

        Self {
            config_dir,
            cache_dir,
            state_dir,
        }
    }

    #[must_use]
    pub fn main_config(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    #[must_use]
    pub fn keymap(&self) -> PathBuf {
        self.config_dir.join("keymap.toml")
    }

    #[must_use]
    pub fn skin(&self) -> PathBuf {
        self.config_dir.join("skin.toml")
    }

    #[must_use]
    pub fn log_dir(&self) -> PathBuf {
        self.cache_dir.join("log")
    }

    /// Path to the user-defined menu file (the F2 / Command → User menu source).
    #[must_use]
    pub fn user_menu(&self) -> PathBuf {
        self.config_dir.join("menu.toml")
    }

    /// Path to the file-extension binding rules.
    #[must_use]
    pub fn extbind(&self) -> PathBuf {
        self.config_dir.join("extbind.toml")
    }

    /// Path to the file-highlighting rules.
    #[must_use]
    pub fn filehighlight(&self) -> PathBuf {
        self.config_dir.join("filehighlight.toml")
    }
}
