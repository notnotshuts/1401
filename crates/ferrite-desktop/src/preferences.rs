use ferrite_core::i18n::Language;
use std::{fs, path::PathBuf};
fn path() -> PathBuf {
    let root = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        return ferrite_core::workspace::default_workspace().join("preferences.json");
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
    };
    root.unwrap_or_else(ferrite_core::workspace::default_workspace)
        .join("ferrite/preferences.json")
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Preferences {
    language: Language,
}
pub fn load() -> Language {
    if let Ok(raw) = fs::read(path())
        && let Ok(p) = serde_json::from_slice::<Preferences>(&raw)
    {
        return p.language;
    }
    #[cfg(target_os = "macos")]
    if let Ok(raw) = ferrite_core::workspace::run_bounded(
        "/usr/bin/defaults",
        &["read", "-g", "AppleLanguages"],
        std::time::Duration::from_secs(2),
    ) {
        let text = String::from_utf8_lossy(&raw);
        if let Some(locale) = text
            .split(['\n', '"', '(', ')', ',', ' '])
            .find(|s| !s.is_empty())
        {
            return Language::from_locale(locale);
        }
    }
    Language::from_locale(
        &std::env::var("LC_ALL")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| std::env::var("LC_MESSAGES").ok())
            .or_else(|| std::env::var("LANG").ok())
            .unwrap_or_else(|| "ru".into()),
    )
}
pub fn save(language: Language) -> anyhow::Result<()> {
    save_to(&path(), language)
}
fn save_to(path: &std::path::Path, language: Language) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("preferences directory"))?;
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write;
    file.write_all(&serde_json::to_vec_pretty(&Preferences { language })?)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preference_round_trip_and_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        save_to(&path, Language::En).unwrap();
        save_to(&path, Language::Es).unwrap();
        let p: Preferences = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(p.language, Language::Es);
    }
}
