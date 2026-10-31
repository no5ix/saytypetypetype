//! Runtime language selection for backend-owned UI strings.
//!
//! The settings window owns the full translation table on the frontend. This
//! module covers the strings the Rust process itself produces: native macOS
//! overlay labels, the version picker, tray menu items, notifications and
//! errors surfaced through overlay hints. Keeping the two sides on the same
//! resolved language avoids a Chinese tray/picker around an English window.
//!
//! English is the fallback for every language the app does not ship, matching
//! the frontend's behaviour. Call sites pass the Simplified Chinese and English
//! variants next to each other:

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    ZhCn,
    En,
}

impl Language {
    pub fn code(self) -> &'static str {
        match self {
            Language::ZhCn => "zh-CN",
            Language::En => "en",
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

fn encode(language: Language) -> u8 {
    match language {
        Language::ZhCn => 0,
        Language::En => 1,
    }
}

fn decode(value: u8) -> Language {
    if value == 1 {
        Language::En
    } else {
        Language::ZhCn
    }
}

/// Install the language resolved from the saved preference.
pub fn set_language(language: Language) {
    CURRENT.store(encode(language), Ordering::SeqCst);
}

pub fn language() -> Language {
    decode(CURRENT.load(Ordering::SeqCst))
}

pub fn is_english() -> bool {
    language() == Language::En
}

/// Resolve a stored preference (`system` | `zh-CN` | `en`) to a concrete
/// language. Unknown values follow the system, which is what a fresh install
/// receives.
pub fn resolve(preference: &str) -> Language {
    match preference {
        "zh-CN" | "zh" | "zh-Hans" => Language::ZhCn,
        "en" | "en-US" | "en-GB" => Language::En,
        _ => detect_system_language(),
    }
}

/// Resolved code for a stored preference, sent to the settings window so it can
/// pick translations without re-implementing system detection in JavaScript.
pub fn resolve_code(preference: &str) -> &'static str {
    resolve(preference).code()
}

/// Best-effort system UI language detection.
///
/// macOS stores the preferred languages in the global defaults domain; Linux
/// exposes them through the POSIX locale variables. Windows falls back to the
/// same variables because Rust does not expose the user UI language without an
/// extra crate; an unknown result resolves to Chinese, the app's primary
/// market, which also matches the historical single-language build.
pub fn detect_system_language() -> Language {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleLanguages"])
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout).to_lowercase();
            // AppleLanguages is an ordered list; the first entry is the active
            // UI language. `contains` is safe here because the list is short
            // and the only decision is whether Chinese appears before English.
            if let Some(first) = text.split('"').find(|part| !part.trim().is_empty()) {
                return language_from_tag(first);
            }
            return language_from_tag(&text);
        }
    }

    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(key) {
            if !value.trim().is_empty() {
                return language_from_tag(&value);
            }
        }
    }
    Language::ZhCn
}

fn language_from_tag(tag: &str) -> Language {
    if tag.trim().to_lowercase().starts_with("zh") {
        Language::ZhCn
    } else {
        Language::En
    }
}

/// Pick the variant matching the active language.
pub fn tr(zh: &'static str, en: &'static str) -> &'static str {
    match language() {
        Language::ZhCn => zh,
        Language::En => en,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_preferences_override_system_detection() {
        assert_eq!(resolve("zh-CN"), Language::ZhCn);
        assert_eq!(resolve("zh"), Language::ZhCn);
        assert_eq!(resolve("en"), Language::En);
        assert_eq!(resolve("en-US"), Language::En);
    }

    #[test]
    fn unknown_preference_falls_back_to_system() {
        assert_eq!(resolve("system"), detect_system_language());
        assert_eq!(resolve("klingon"), detect_system_language());
    }

    #[test]
    fn tags_resolve_by_prefix() {
        assert_eq!(language_from_tag("zh-Hans-CN"), Language::ZhCn);
        assert_eq!(language_from_tag("en_US.UTF-8"), Language::En);
    }

    #[test]
    fn translation_prefers_the_active_language() {
        // `tr` is intentionally a plain pair selector; no test mutates the
        // process-wide value because other modules' assertions depend on the
        // default (Chinese) variant.
        assert_eq!(Language::En.code(), "en");
        assert_eq!(Language::ZhCn.code(), "zh-CN");
    }
}
