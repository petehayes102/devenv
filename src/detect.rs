use std::path::Path;

use walkdir::WalkDir;

// Detect a reasonable base image based on project files
pub fn detect_base_image(project_dir: &Path) -> Option<String> {
    // Quick checks by presence of common files in root
    let root = project_dir;

    let check = |name: &str| root.join(name).exists();

    if check("Cargo.toml") {
        return Some("rust:trixie".to_string());
    }
    if check("package.json") {
        return Some("node:current-bookworm".to_string());
    }
    if check("pyproject.toml") || check("requirements.txt") {
        return Some("python:bookworm".to_string());
    }
    if check("go.mod") {
        return Some("golang:bookworm".to_string());
    }
    if check("Gemfile") {
        return Some("ruby:bookworm".to_string());
    }
    if check("pom.xml") || has_gradle_files(root) {
        return Some("eclipse-temurin:latest".to_string());
    }
    if has_extension(root, "csproj") {
        return Some("mcr.microsoft.com/dotnet/sdk:8.0".to_string());
    }
    if check("composer.json") {
        return Some("php:bookworm".to_string());
    }
    if check("mix.exs") {
        return Some("elixir:slim".to_string());
    }

    None
}

fn has_gradle_files(root: &Path) -> bool {
    root.join("build.gradle").exists() || root.join("build.gradle.kts").exists()
}

fn has_extension(root: &Path, ext: &str) -> bool {
    for e in WalkDir::new(root).max_depth(2).into_iter().flatten() {
        if e.file_type().is_file() && e.path().extension().is_some_and(|e2| e2 == ext) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn detects_rust() {
        let td = TempDir::new().unwrap();
        fs::write(td.path().join("Cargo.toml"), "[package]\nname='x'\n").unwrap();
        let img = detect_base_image(td.path());
        assert_eq!(img.as_deref(), Some("rust:trixie"));
    }

    #[test]
    fn detects_node() {
        let td = TempDir::new().unwrap();
        fs::write(td.path().join("package.json"), "{}\n").unwrap();
        assert_eq!(
            detect_base_image(td.path()).as_deref(),
            Some("node:current-bookworm")
        );
    }

    #[test]
    fn detects_python() {
        let td = TempDir::new().unwrap();
        fs::write(td.path().join("requirements.txt"), "requests\n").unwrap();
        assert_eq!(
            detect_base_image(td.path()).as_deref(),
            Some("python:bookworm")
        );
    }

    #[test]
    fn detects_java_gradle() {
        let td = TempDir::new().unwrap();
        fs::write(td.path().join("build.gradle"), "plugins {}\n").unwrap();
        assert_eq!(
            detect_base_image(td.path()).as_deref(),
            Some("eclipse-temurin:latest")
        );
    }

    #[test]
    fn detects_dotnet_csproj() {
        let td = TempDir::new().unwrap();
        let sub = td.path().join("src");
        std::fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("app.csproj"), "<Project/>\n").unwrap();
        assert_eq!(
            detect_base_image(td.path()).as_deref(),
            Some("mcr.microsoft.com/dotnet/sdk:8.0")
        );
    }

    #[test]
    fn returns_none_when_unknown() {
        let td = TempDir::new().unwrap();
        assert_eq!(detect_base_image(td.path()), None);
    }
}
