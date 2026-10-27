//! Build identity: production vs. development app.
//!
//! The production app keeps `com.yolanda.voicepaste`; development builds use
//! `com.yolanda.voicepaste.dev` (see `src-tauri/tauri.dev.conf.json`). Tauri
//! derives `app_data_dir()` from the bundle identifier, so the two identities
//! already get isolated preferences, history, logs and downloaded models — this
//! module only exposes that identity to code that must behave differently, such
//! as the updater (a dev build must never install a production release over
//! itself).

/// Bundle identifier of the signed production app.
pub const PRODUCTION_IDENTIFIER: &str = "com.yolanda.voicepaste";
/// Bundle identifier of builds made with `tauri.dev.conf.json`.
pub const DEVELOPMENT_IDENTIFIER: &str = "com.yolanda.voicepaste.dev";

/// Resolved identity of the running binary.
#[derive(Debug, Clone)]
pub struct BuildIdentity {
    /// Bundle identifier baked into the running `.app`.
    pub identifier: String,
    /// True only for the documented development identifier.
    pub is_development: bool,
}

impl BuildIdentity {
    /// Derive the identity from a bundle identifier.
    pub fn from_identifier(identifier: impl Into<String>) -> Self {
        let identifier = identifier.into();
        let is_development = identifier == DEVELOPMENT_IDENTIFIER;
        Self {
            identifier,
            is_development,
        }
    }

    /// Identity of the running app, as resolved from the Tauri context.
    pub fn of<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Self {
        Self::from_identifier(app.config().identifier.clone())
    }

    /// Human-readable profile for startup logs.
    pub fn profile_label(&self) -> &'static str {
        if self.identifier == PRODUCTION_IDENTIFIER {
            "production"
        } else if self.identifier == DEVELOPMENT_IDENTIFIER {
            "development"
        } else {
            "custom"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_identifier_is_not_development() {
        let identity = BuildIdentity::from_identifier(PRODUCTION_IDENTIFIER);
        assert!(!identity.is_development);
        assert_eq!(identity.identifier, "com.yolanda.voicepaste");
    }

    #[test]
    fn development_identifier_is_development() {
        let identity = BuildIdentity::from_identifier(DEVELOPMENT_IDENTIFIER);
        assert!(identity.is_development);
    }

    #[test]
    fn identifiers_are_distinct_prefix_related() {
        assert_ne!(PRODUCTION_IDENTIFIER, DEVELOPMENT_IDENTIFIER);
        assert!(DEVELOPMENT_IDENTIFIER.starts_with(PRODUCTION_IDENTIFIER));
        assert!(DEVELOPMENT_IDENTIFIER.ends_with(".dev"));
    }

    #[test]
    fn unknown_identifiers_are_treated_as_release_builds() {
        let identity = BuildIdentity::from_identifier("com.example.other");
        assert!(!identity.is_development);
    }
}
