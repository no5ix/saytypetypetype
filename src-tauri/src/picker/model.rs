//! Picker session state, independent of Tauri/AppKit so it can be unit tested.
//!
//! A session owns the original transcript, the configured template options, and
//! their asynchronous results. The generation id is bumped on every open; a late
//! result from an older session is rejected instead of mutating the new one.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::Serialize;

use crate::display::TargetDisplay;

pub(crate) const ORIGINAL_OPTION_ID: &str = "__original__";

pub(crate) fn prompt_option_id(prompt_id: &str) -> String {
    format!("prompt:{prompt_id}")
}

/// A template option as resolved from the user's configured prompt list.
#[derive(Clone, Debug)]
pub(crate) struct PickerOptionSpec {
    pub id: String,
    pub title: String,
    pub hotkey: String,
    pub prompt_id: String,
}

/// Serialized to the picker WebView.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PickerOptionView {
    pub id: String,
    pub title: String,
    /// Formatted accelerator label ("R ⌥", "T"), or empty when unbound.
    pub hotkey: String,
    /// "original" | "template"
    pub kind: String,
    /// "ready" | "loading" | "failed"
    pub status: String,
    pub text: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PickerView {
    pub session_id: u64,
    pub options: Vec<PickerOptionView>,
    pub delivering: bool,
    pub all_completed: bool,
    pub error: Option<String>,
    pub theme: String,
    pub target_app: Option<String>,
}

/// Everything the delivery path needs, copied out of the session while the
/// store lock is held. `history_text` is always the original ASR transcript.
#[derive(Clone, Debug)]
pub(crate) struct PickerSelection {
    pub session_id: u64,
    pub option_id: String,
    pub text: String,
    pub history_text: String,
    pub prompt_id: Option<String>,
    pub target: Option<TargetDisplay>,
    /// Recording epoch that owns the lowered system volume. Session-scoped
    /// restore ensures a stale picker cannot restore a newer recording.
    pub volume_session: u64,
}

#[derive(Clone, Debug)]
pub(crate) enum PickerResult {
    Ready(String),
    Failed(String),
}

#[derive(Clone, Debug)]
struct PickerOption {
    view: PickerOptionView,
    prompt_id: Option<String>,
}

#[derive(Clone, Debug)]
struct PickerSession {
    id: u64,
    original: String,
    options: Vec<PickerOption>,
    delivering: bool,
    error: Option<String>,
    theme: String,
    target: Option<TargetDisplay>,
    volume_session: u64,
}

/// Shared picker state held by `AppInner`.
#[derive(Default)]
pub(crate) struct PickerStore {
    generation: AtomicU64,
    session: Mutex<Option<PickerSession>>,
}

impl PickerStore {
    /// Start a new session. Any older session is dropped: callers must have
    /// cancelled or delivered it first, and dropping guarantees its late
    /// results can no longer touch the new state.
    pub(crate) fn open(
        &self,
        original: String,
        templates: Vec<PickerOptionSpec>,
        original_hotkey: String,
        theme: String,
        target: Option<TargetDisplay>,
        volume_session: u64,
    ) -> PickerView {
        let id = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let mut options = Vec::with_capacity(templates.len() + 1);
        options.push(PickerOption {
            view: PickerOptionView {
                id: ORIGINAL_OPTION_ID.to_string(),
                title: "原始转写".to_string(),
                hotkey: original_hotkey,
                kind: "original".to_string(),
                status: "ready".to_string(),
                text: Some(original.clone()),
                error: None,
            },
            prompt_id: None,
        });
        for template in templates {
            options.push(PickerOption {
                view: PickerOptionView {
                    id: template.id,
                    title: template.title,
                    hotkey: template.hotkey,
                    kind: "template".to_string(),
                    status: "loading".to_string(),
                    text: None,
                    error: None,
                },
                prompt_id: Some(template.prompt_id),
            });
        }

        let session = PickerSession {
            id,
            original,
            options,
            delivering: false,
            error: None,
            theme,
            target,
            volume_session,
        };
        let view = session.view();
        *self.session.lock().unwrap() = Some(session);
        view
    }

    pub(crate) fn view(&self, session_id: u64) -> Option<PickerView> {
        self.session
            .lock()
            .unwrap()
            .as_ref()
            .filter(|session| session.id == session_id)
            .map(PickerSession::view)
    }

    #[cfg(test)]
    pub(crate) fn is_active(&self) -> bool {
        self.session.lock().unwrap().is_some()
    }

    pub(crate) fn current_id(&self) -> Option<u64> {
        self.session
            .lock()
            .unwrap()
            .as_ref()
            .map(|session| session.id)
    }

    pub(crate) fn set_result(
        &self,
        session_id: u64,
        option_id: &str,
        result: PickerResult,
    ) -> bool {
        let mut guard = self.session.lock().unwrap();
        let Some(session) = guard.as_mut().filter(|session| session.id == session_id) else {
            // A newer picker session owns the UI; stale results are dropped.
            return false;
        };
        let Some(option) = session
            .options
            .iter_mut()
            .find(|option| option.view.id == option_id)
        else {
            return false;
        };
        if option.view.status != "loading" {
            return false;
        }
        match result {
            PickerResult::Ready(text) => {
                option.view.status = "ready".to_string();
                option.view.text = Some(text);
                option.view.error = None;
            }
            PickerResult::Failed(error) => {
                option.view.status = "failed".to_string();
                option.view.text = None;
                option.view.error = Some(error);
            }
        }
        true
    }

    /// Reserve the option for delivery. Only a ready option in the current,
    /// non-delivering session can be taken, so a shortcut and a click racing
    /// each other paste at most once.
    pub(crate) fn begin_delivery(
        &self,
        session_id: u64,
        option_id: &str,
    ) -> Option<PickerSelection> {
        let mut guard = self.session.lock().unwrap();
        let session = guard.as_mut().filter(|session| session.id == session_id)?;
        if session.delivering {
            return None;
        }
        let option = session.options.iter().find(|option| {
            option.view.id == option_id
                && option.view.status == "ready"
                && option.view.text.is_some()
        })?;
        session.delivering = true;
        session.error = None;
        Some(PickerSelection {
            session_id,
            option_id: option.view.id.clone(),
            text: option.view.text.clone().unwrap_or_default(),
            history_text: session.original.clone(),
            prompt_id: option.prompt_id.clone(),
            target: session.target.clone(),
            volume_session: session.volume_session,
        })
    }

    /// Recording epoch whose volume snapshot this picker session owns.
    pub(crate) fn volume_session(&self, session_id: u64) -> Option<u64> {
        self.session
            .lock()
            .unwrap()
            .as_ref()
            .filter(|session| session.id == session_id)
            .map(|session| session.volume_session)
    }

    /// Finish a delivery attempt. Success ends the session; failure keeps it
    /// open (and re-enables selection) so the user can retry without losing the
    /// text.
    pub(crate) fn finish_delivery(&self, session_id: u64, success: bool, error: Option<String>) {
        let mut guard = self.session.lock().unwrap();
        let Some(session) = guard.as_mut().filter(|session| session.id == session_id) else {
            return;
        };
        if success {
            *guard = None;
            return;
        }
        session.delivering = false;
        session.error = error;
    }

    /// ESC / close. A delivery already in flight wins; cancelling then would
    /// race the paste and could restore state under it.
    pub(crate) fn cancel(&self, session_id: u64) -> bool {
        let mut guard = self.session.lock().unwrap();
        let Some(session) = guard.as_ref() else {
            return false;
        };
        if session.id != session_id || session.delivering {
            return false;
        }
        *guard = None;
        true
    }

    /// Force-drop the session (new recording starting, app shutdown).
    pub(crate) fn take(&self) -> Option<PickerView> {
        self.session
            .lock()
            .unwrap()
            .take()
            .map(|session| session.view())
    }
}

impl PickerSession {
    fn view(&self) -> PickerView {
        PickerView {
            session_id: self.id,
            options: self
                .options
                .iter()
                .map(|option| option.view.clone())
                .collect(),
            delivering: self.delivering,
            all_completed: self
                .options
                .iter()
                .all(|option| option.view.status != "loading"),
            error: self.error.clone(),
            theme: self.theme.clone(),
            target_app: self
                .target
                .as_ref()
                .and_then(|target| target.target.as_ref())
                .map(|app| app.name.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::{Display, Rect};

    fn target() -> Option<TargetDisplay> {
        Some(TargetDisplay {
            display: Display {
                frame: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 1920.0,
                    height: 1080.0,
                },
                work_area: Rect {
                    x: 0.0,
                    y: 25.0,
                    width: 1920.0,
                    height: 1055.0,
                },
            },
            target: None,
        })
    }

    fn specs() -> Vec<PickerOptionSpec> {
        vec![
            PickerOptionSpec {
                id: prompt_option_id("prompt-general"),
                title: "通用整理".into(),
                hotkey: "R ⇧".into(),
                prompt_id: "prompt-general".into(),
            },
            PickerOptionSpec {
                id: prompt_option_id("prompt-translate"),
                title: "翻译文本".into(),
                hotkey: "T".into(),
                prompt_id: "prompt-translate".into(),
            },
        ]
    }

    fn open(store: &PickerStore) -> PickerView {
        store.open(
            "原始 转写".into(),
            specs(),
            "R ⌥".into(),
            "dark".into(),
            target(),
            1,
        )
    }

    #[test]
    fn original_is_first_and_immediately_ready() {
        let store = PickerStore::default();
        let view = open(&store);
        assert_eq!(view.options.len(), 3);
        assert_eq!(view.options[0].id, ORIGINAL_OPTION_ID);
        assert_eq!(view.options[0].title, "原始转写");
        assert_eq!(view.options[0].status, "ready");
        assert_eq!(view.options[0].text.as_deref(), Some("原始 转写"));
        assert_eq!(view.options[0].hotkey, "R ⌥");
        assert_eq!(view.options[1].status, "loading");
    }

    #[test]
    fn template_order_matches_configuration() {
        let store = PickerStore::default();
        let view = open(&store);
        assert_eq!(view.options[1].title, "通用整理");
        assert_eq!(view.options[2].title, "翻译文本");
        assert_eq!(view.options[1].kind, "template");
    }

    #[test]
    fn results_are_keyed_by_template_not_completion_order() {
        let store = PickerStore::default();
        let view = open(&store);
        let id = view.session_id;
        assert!(store.set_result(
            id,
            &prompt_option_id("prompt-translate"),
            PickerResult::Ready("translated".into())
        ));
        assert!(store.set_result(
            id,
            &prompt_option_id("prompt-general"),
            PickerResult::Ready("polished".into())
        ));
        let view = store.view(id).unwrap();
        assert_eq!(
            view.options[1].text.as_deref(),
            Some("polished"),
            "general must keep its own result"
        );
        assert_eq!(
            view.options[2].text.as_deref(),
            Some("translated"),
            "translate must keep its own result"
        );
        assert!(view.all_completed);
    }

    #[test]
    fn stale_session_results_are_ignored() {
        let store = PickerStore::default();
        let first = open(&store);
        let first_id = first.session_id;
        let second = open(&store);
        assert_ne!(first_id, second.session_id);
        assert!(!store.set_result(
            first_id,
            &prompt_option_id("prompt-general"),
            PickerResult::Ready("stale".into())
        ));
        let current = store.view(second.session_id).unwrap();
        assert_eq!(current.options[1].status, "loading");
        assert!(current.options[1].text.is_none());
    }

    #[test]
    fn failed_option_does_not_block_other_selections() {
        let store = PickerStore::default();
        let view = open(&store);
        let id = view.session_id;
        assert!(store.set_result(
            id,
            &prompt_option_id("prompt-general"),
            PickerResult::Failed("配置缺失".into())
        ));
        assert!(store.set_result(
            id,
            &prompt_option_id("prompt-translate"),
            PickerResult::Ready("hello".into())
        ));
        assert!(store
            .begin_delivery(id, &prompt_option_id("prompt-general"))
            .is_none());
        assert!(store.begin_delivery(id, ORIGINAL_OPTION_ID).is_some());
        // One delivery at a time: finish (or fail) it before taking another.
        store.finish_delivery(id, false, None);
        assert!(store
            .begin_delivery(id, &prompt_option_id("prompt-translate"))
            .is_some());
    }

    #[test]
    fn selection_records_original_history_text() {
        let store = PickerStore::default();
        let view = open(&store);
        let id = view.session_id;
        store.set_result(
            id,
            &prompt_option_id("prompt-general"),
            PickerResult::Ready("润色后的文本".into()),
        );
        let selection = store
            .begin_delivery(id, &prompt_option_id("prompt-general"))
            .unwrap();
        assert_eq!(selection.text, "润色后的文本");
        assert_eq!(selection.history_text, "原始 转写");
        assert_eq!(selection.prompt_id.as_deref(), Some("prompt-general"));
    }

    #[test]
    fn selecting_an_option_twice_delivers_once() {
        let store = PickerStore::default();
        let view = open(&store);
        let id = view.session_id;
        assert!(store.begin_delivery(id, ORIGINAL_OPTION_ID).is_some());
        assert!(store.begin_delivery(id, ORIGINAL_OPTION_ID).is_none());
        store.finish_delivery(id, true, None);
        assert!(!store.is_active());
        assert!(store.begin_delivery(id, ORIGINAL_OPTION_ID).is_none());
    }

    #[test]
    fn failed_delivery_keeps_the_text_and_allows_retry() {
        let store = PickerStore::default();
        let view = open(&store);
        let id = view.session_id;
        let first = store.begin_delivery(id, ORIGINAL_OPTION_ID).unwrap();
        store.finish_delivery(id, false, Some("粘贴失败".into()));
        assert_eq!(store.view(id).unwrap().error.as_deref(), Some("粘贴失败"));
        let retry = store.begin_delivery(id, ORIGINAL_OPTION_ID).unwrap();
        assert_eq!(first.text, retry.text);
    }

    #[test]
    fn cancel_clears_the_session_and_blocks_late_results() {
        let store = PickerStore::default();
        let view = open(&store);
        let id = view.session_id;
        assert!(store.cancel(id));
        assert!(!store.is_active());
        assert!(!store.set_result(
            id,
            &prompt_option_id("prompt-general"),
            PickerResult::Ready("late".into())
        ));
    }

    #[test]
    fn cancel_is_ignored_while_delivering() {
        let store = PickerStore::default();
        let view = open(&store);
        let id = view.session_id;
        assert!(store.begin_delivery(id, ORIGINAL_OPTION_ID).is_some());
        assert!(!store.cancel(id));
        assert!(store.is_active());
    }

    #[test]
    fn new_session_gets_a_new_generation() {
        let store = PickerStore::default();
        let first = open(&store);
        store.cancel(first.session_id);
        let second = open(&store);
        assert!(second.session_id > first.session_id);
        assert_eq!(store.current_id(), Some(second.session_id));
    }
}
