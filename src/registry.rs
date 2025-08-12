use anyhow::{Context, Result};
use dirs::config_dir;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Serialize, Deserialize)]
struct Registry {
    envs: BTreeMap<String, PathBuf>,
}

fn registry_path() -> PathBuf {
    // Prefer XDG_CONFIG_HOME when set (useful for tests and Linux setups),
    // otherwise fall back to platform default via dirs::config_dir.
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(config_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("devenv").join("registry.json")
}

fn load_registry() -> Result<Registry> {
    let path = registry_path();
    if let Ok(data) = fs::read_to_string(&path) {
        let reg: Registry = serde_json::from_str(&data).with_context(|| "Parsing registry.json")?;
        Ok(reg)
    } else {
        Ok(Registry::default())
    }
}

fn save_registry(reg: &Registry) -> Result<()> {
    let path = registry_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(reg)?;
    fs::write(path, data)?;
    Ok(())
}

pub fn register_env(name: &str, path: &Path) -> Result<()> {
    let mut reg = load_registry()?;
    if matches!(reg.envs.get(name), Some(existing) if existing != path) {
        anyhow::bail!(
            "An environment named '{}' already exists at {}",
            name,
            reg.envs.get(name).unwrap().display()
        );
    }
    reg.envs.insert(name.to_string(), path.to_path_buf());
    save_registry(&reg)
}

pub fn lookup_env(name: &str) -> Result<PathBuf> {
    let reg = load_registry()?;
    reg.envs
        .get(name)
        .cloned()
        .with_context(|| format!("Environment '{name}' not found in registry"))
}

pub fn unregister_env(name: &str) -> Result<bool> {
    let mut reg = load_registry()?;
    let removed = reg.envs.remove(name).is_some();
    if removed {
        save_registry(&reg)?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::env;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    #[serial]
    fn register_and_lookup_roundtrip() {
        let td = TempDir::new().unwrap();
        // Isolate config in a temp XDG config home
        unsafe {
            env::set_var("XDG_CONFIG_HOME", td.path());
        }

        let project = td.path().join("proj");
        fs::create_dir_all(&project).unwrap();
        register_env("foo", &project).unwrap();
        let got = lookup_env("foo").unwrap();
        assert_eq!(got, project);
    }

    #[test]
    #[serial]
    fn duplicate_name_different_path_errors() {
        let td = TempDir::new().unwrap();
        unsafe {
            env::set_var("XDG_CONFIG_HOME", td.path());
        }

        let p1 = td.path().join("p1");
        let p2 = td.path().join("p2");
        fs::create_dir_all(&p1).unwrap();
        fs::create_dir_all(&p2).unwrap();

        register_env("dup", &p1).unwrap();
        let err = register_env("dup", &p2).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("already exists"));
    }

    #[test]
    #[serial]
    fn unregister_removes_entry() {
        let td = TempDir::new().unwrap();
        unsafe {
            env::set_var("XDG_CONFIG_HOME", td.path());
        }

        let project = td.path().join("proj");
        fs::create_dir_all(&project).unwrap();
        register_env("gone", &project).unwrap();
        assert!(unregister_env("gone").unwrap());
        let err = lookup_env("gone").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("not found"));
    }
}
