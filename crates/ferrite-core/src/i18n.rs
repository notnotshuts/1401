//! JSON-backed user-facing text. Technical reports retain their original data.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{
        OnceLock,
        atomic::{AtomicU8, Ordering},
    },
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    Ru,
    En,
    Es,
}
impl Language {
    pub const ALL: [Self; 3] = [Self::Ru, Self::En, Self::Es];
    pub fn code(self) -> &'static str {
        match self {
            Self::Ru => "ru",
            Self::En => "en",
            Self::Es => "es",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Ru => "Русский",
            Self::En => "English",
            Self::Es => "Español",
        }
    }
    pub fn from_locale(locale: &str) -> Self {
        match locale
            .split(['-', '_', '.'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "es" => Self::Es,
            "en" => Self::En,
            _ => Self::Ru,
        }
    }
}
static CURRENT: AtomicU8 = AtomicU8::new(0);
static CATALOGS: OnceLock<[BTreeMap<String, String>; 3]> = OnceLock::new();
fn catalogs() -> &'static [BTreeMap<String, String>; 3] {
    CATALOGS.get_or_init(|| {
        [
            include_str!("../locales/ru.json"),
            include_str!("../locales/en.json"),
            include_str!("../locales/es.json"),
        ]
        .map(|s| serde_json::from_str(s).expect("Invalid bundled translation JSON"))
    })
}
fn index(language: Language) -> usize {
    match language {
        Language::Ru => 0,
        Language::En => 1,
        Language::Es => 2,
    }
}
pub fn set_language(language: Language) {
    CURRENT.store(index(language) as u8, Ordering::Relaxed);
}
pub fn language() -> Language {
    Language::ALL[CURRENT.load(Ordering::Relaxed) as usize]
}
pub fn text_for(language: Language, key: &'static str) -> &'static str {
    catalogs()[index(language)]
        .get(key)
        .or_else(|| catalogs()[1].get(key))
        .map(String::as_str)
        .unwrap_or(key)
}
pub fn tr(key: &'static str) -> &'static str {
    text_for(language(), key)
}
pub fn format_for(language: Language, key: &'static str, params: &[(&str, String)]) -> String {
    let mut text = text_for(language, key);
    let mut out = String::new();
    while let Some(start) = text.find('{') {
        out.push_str(&text[..start]);
        let Some(end) = text[start..].find('}').map(|n| start + n) else {
            out.push_str(&text[start..]);
            return out;
        };
        let name = &text[start + 1..end];
        if let Some((_, value)) = params.iter().find(|(key, _)| *key == name) {
            out.push_str(value);
        } else {
            out.push_str(&text[start..=end]);
        }
        text = &text[end + 1..];
    }
    out.push_str(text);
    out
}
pub fn tf(key: &'static str, params: &[(&str, String)]) -> String {
    format_for(language(), key, params)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn placeholders(s: &str) -> Vec<&str> {
        let mut out = vec![];
        let mut s = s;
        while let Some(start) = s.find('{') {
            let end = s[start..].find('}').expect("closing placeholder") + start;
            out.push(&s[start + 1..end]);
            s = &s[end + 1..];
        }
        out.sort_unstable();
        out
    }
    #[test]
    fn same_keys_and_parameters_in_every_language() {
        let all = catalogs();
        for catalog in &all[1..] {
            assert_eq!(
                all[0].keys().collect::<Vec<_>>(),
                catalog.keys().collect::<Vec<_>>()
            );
            for (key, value) in &all[0] {
                assert!(!catalog[key].trim().is_empty(), "{key}");
                assert_eq!(placeholders(value), placeholders(&catalog[key]), "{key}");
            }
        }
    }
    #[test]
    fn formatting_does_not_expand_user_data() {
        assert_eq!(
            format_for(
                Language::En,
                "report.import",
                &[("0", "{text}/file.json".into())]
            ),
            "Import: {text}/file.json"
        );
    }
    #[test]
    fn language_switch_updates_text() {
        let previous = language();
        set_language(Language::En);
        assert_eq!(tr("nav.guide"), "Guide");
        set_language(Language::Es);
        assert_eq!(tr("nav.guide"), "Guía");
        set_language(previous);
    }
    #[test]
    fn detects_regional_locales() {
        assert_eq!(Language::from_locale("es_MX.UTF-8"), Language::Es);
        assert_eq!(Language::from_locale("en-US"), Language::En);
        assert_eq!(Language::from_locale("ru_RU"), Language::Ru);
    }
}
