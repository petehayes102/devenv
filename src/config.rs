use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::detect::detect_base_image;

const FILENAME: &str = "devenv.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub devenv: DevEnvConfig,
    #[serde(skip)]
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DevEnvConfig {
    /// Unique environment name (defaults to directory name)
    pub name: String,
    /// Base Docker image to use (auto-detected if empty)
    pub image: String,
    /// Path to SSH private key to mount into the container (optional)
    pub ssh_private_key: Option<String>,
    /// Extra OS packages to install (apt-based)
    pub packages: Vec<String>,
    /// Commands to run after container start (provisioning)
    pub commands: Vec<String>,
    /// Optional Zed remote configuration
    pub zed_remote: Option<ZedRemote>,
    /// Optional path to a public key to add to authorized_keys inside the container
    pub ssh_public_key: Option<String>,
    /// Optional non-root user configuration for container login/ownership
    pub user_name: Option<String>,
    pub user_uid: Option<u32>,
    pub user_gid: Option<u32>,
    /// Run provisioning commands as non-root user if available
    pub provision_as_non_root: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZedRemote {
    pub enabled: bool,
    /// SSH port published on the host; defaults to 2222
    pub ssh_port: Option<u16>,
    /// SSH username (container user); defaults to root
    pub ssh_user: Option<String>,
}

impl Config {
    pub fn exists(path: impl AsRef<Path>) -> bool {
        make_path(path).exists()
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = make_path(path);
        let cfg =
            fs::read_to_string(&path).with_context(|| format!("Reading {}", path.display()))?;
        toml::from_str(&cfg).with_context(|| "Parsing devenv.toml")
    }

    pub fn create(cwd: impl AsRef<Path>) -> Result<Self> {
        let cwd = cwd.as_ref();
        let cfg_path = make_path(cwd);

        // Sanity checks
        if !cwd.is_dir() {
            bail!("Path must be a directory");
        } else if cfg_path.exists() {
            bail!("Config file already exists");
        }

        let mut this = Config {
            devenv: Default::default(),
            path: cfg_path,
        };

        // Set project name to directory name
        this.devenv.name = cwd
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("new_project")
            .to_string();

        // Try and set a sane Docker image
        this.devenv.image =
            detect_base_image(cwd).unwrap_or_else(|| "debian:bookworm-slim".to_string());

        // Write config to fs
        let toml_str = toml::to_string_pretty(&this)?;
        fs::write(&this.path, toml_str)?;

        Ok(this)
    }
}

fn make_path(path: impl AsRef<Path>) -> PathBuf {
    match path.as_ref().file_name() {
        Some(name) if name.to_str() == Some(FILENAME) => path.as_ref().to_path_buf(),
        _ => path.as_ref().join(FILENAME),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn exists_and_open_work_with_dir_and_file_paths() {
        let td = TempDir::new().unwrap();
        // Initially, no config exists
        assert!(!Config::exists(td.path()));
        // Create minimal config file
        let cfg_path = td.path().join(FILENAME);
        let content = r#"[devenv]
name = "sample"
image = "debian:bookworm-slim"
packages = []
commands = []
provision_as_non_root = false
"#;
        std::fs::write(&cfg_path, content).unwrap();
        assert!(Config::exists(td.path()));
        // Can open via directory or full file path
        let a = Config::open(td.path()).unwrap();
        let b = Config::open(&cfg_path).unwrap();
        assert_eq!(a.devenv.name, "sample");
        assert_eq!(b.devenv.image, "debian:bookworm-slim");
    }

    #[test]
    fn create_sets_defaults_and_detects_image() {
        let td = TempDir::new().unwrap();
        let dir = td.path().join("myproj");
        std::fs::create_dir_all(&dir).unwrap();
        // No recognizable files -> default image
        let cfg = Config::create(&dir).unwrap();
        assert_eq!(cfg.devenv.name, "myproj");
        assert_eq!(cfg.devenv.image, "debian:bookworm-slim");
        assert!(cfg.devenv.packages.is_empty());
        assert!(cfg.devenv.commands.is_empty());
        assert!(!cfg.devenv.provision_as_non_root);
        assert!(cfg.path.exists());

        // Recognizable language file -> detected image
        let dir2 = td.path().join("rustproj");
        std::fs::create_dir_all(&dir2).unwrap();
        std::fs::write(dir2.join("Cargo.toml"), "[package]\nname='x'\n").unwrap();
        let cfg2 = Config::create(&dir2).unwrap();
        assert_eq!(cfg2.devenv.name, "rustproj");
        assert_eq!(cfg2.devenv.image, "rust:trixie");
        assert_eq!(cfg2.path, dir2.join(FILENAME));
    }
}
