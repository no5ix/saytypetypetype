//! One-slot model cache shared by ASR engine sessions.
//!
//! Local sherpa-onnx recognizers are expensive to construct (hundreds of MB of
//! weights, seconds of CPU) and were previously rebuilt for every recording.
//! This cache keeps the selected recognizer alive across recordings while voice
//! input is active. A cache key fingerprints the model + effective parameters,
//! so changing the selected model, provider, threads, VAD-adjacent defaults, or
//! hotwords transparently rebuilds on the next session.
//!
//! The slot holds a `Mutex` across construction. That is intentional: two
//! sessions racing to initialize the same model must not build two copies (each
//! would double RAM and accelerator usage). Callers only take the cached value
//! out as a cheap clone (`Arc` in production), and construction failures leave
//! the slot empty so the next recording retries cleanly.

use std::sync::Mutex;
use std::time::Instant;

/// A generic one-entry cache with a textual key.
///
/// Generic over the loaded value so the concurrency/lifetime rules can be unit
/// tested with a lightweight fake instead of a real multi-hundred-MB model.
pub(crate) struct ModelSlot<T> {
    name: &'static str,
    inner: Mutex<SlotInner<T>>,
}

struct SlotInner<T> {
    key: Option<String>,
    value: Option<T>,
    loads: u64,
    unloads: u64,
}

impl<T> ModelSlot<T> {
    pub(crate) fn new(name: &'static str) -> Self {
        Self {
            name,
            inner: Mutex::new(SlotInner {
                key: None,
                value: None,
                loads: 0,
                unloads: 0,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SlotInner<T>> {
        // A panic in a loader must not permanently disable voice input.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Return the value cached under `key`, or build it once.
    ///
    /// Returns `(value, newly_loaded)`. On a cache miss the old value is
    /// dropped before loading, so switching models never keeps both recognizers
    /// alive at once. A failed load leaves the slot empty.
    pub(crate) fn get_or_load<F>(&self, key: &str, load: F) -> Result<(T, bool), String>
    where
        T: Clone,
        F: FnOnce() -> Result<T, String>,
    {
        let mut inner = self.lock();
        if inner.key.as_deref() == Some(key) {
            if let Some(value) = inner.value.as_ref() {
                log_asr!(
                    debug,
                    "model cache hit slot={} key_chars={} loads={}",
                    self.name,
                    key.chars().count(),
                    inner.loads
                );
                return Ok((value.clone(), false));
            }
        }

        // Drop the previous model before constructing the new one: a model
        // switch must not transiently hold two full recognizers.
        inner.value = None;
        inner.key = None;

        let started = Instant::now();
        let value = load()?;
        let elapsed = started.elapsed();
        inner.value = Some(value.clone());
        inner.key = Some(key.to_string());
        inner.loads += 1;
        log_asr!(
            info,
            "model cache loaded slot={} duration_ms={} loads={}",
            self.name,
            elapsed.as_millis(),
            inner.loads
        );
        Ok((value, true))
    }

    /// Release the loaded value. The next session rebuilds. Logged so lifetime
    /// changes are visible in the diagnostic log without any user content.
    pub(crate) fn unload(&self, reason: &str) {
        let mut inner = self.lock();
        if inner.value.take().is_some() {
            inner.key = None;
            inner.unloads += 1;
            log_asr!(
                info,
                "model cache unloaded slot={} reason={} unloads={}",
                self.name,
                reason,
                inner.unloads
            );
        }
    }

    #[cfg(test)]
    fn cached_key(&self) -> Option<String> {
        self.lock().key.clone()
    }

    #[cfg(test)]
    fn load_count(&self) -> u64 {
        self.lock().loads
    }

    #[cfg(test)]
    fn unload_count(&self) -> u64 {
        self.lock().unloads
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn repeated_access_reuses_the_same_model() {
        let slot: ModelSlot<Arc<u64>> = ModelSlot::new("test");
        let loads = AtomicUsize::new(0);
        let load = || {
            loads.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(42))
        };

        let (first, loaded) = slot.get_or_load("model-a", load).unwrap();
        assert!(loaded);
        let (second, loaded) = slot
            .get_or_load("model-a", || {
                loads.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(99))
            })
            .unwrap();
        assert!(!loaded);
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(*second, 42);
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        assert_eq!(slot.load_count(), 1);
    }

    #[test]
    fn a_failed_initialization_is_not_cached_and_retries() {
        let slot: ModelSlot<Arc<u64>> = ModelSlot::new("test");
        let loads = AtomicUsize::new(0);
        let error = slot
            .get_or_load("model-a", || {
                loads.fetch_add(1, Ordering::SeqCst);
                Err("missing model files".to_string())
            })
            .unwrap_err();
        assert!(error.contains("missing"));
        assert!(slot.cached_key().is_none());

        let (value, loaded) = slot
            .get_or_load("model-a", || {
                loads.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(7))
            })
            .unwrap();
        assert!(loaded);
        assert_eq!(*value, 7);
        assert_eq!(loads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn model_change_replaces_and_drops_the_old_instance() {
        let slot: ModelSlot<Arc<u64>> = ModelSlot::new("test");
        let (old, _) = slot.get_or_load("model-a", || Ok(Arc::new(1))).unwrap();
        let (new, loaded) = slot.get_or_load("model-b", || Ok(Arc::new(2))).unwrap();
        assert!(loaded);
        assert_eq!(*old, 1);
        assert_eq!(*new, 2);
        assert_eq!(slot.cached_key().as_deref(), Some("model-b"));
        assert_eq!(Arc::strong_count(&old), 1);
        assert_eq!(slot.load_count(), 2);
    }

    #[test]
    fn unload_releases_and_allows_reinitialization() {
        let slot: ModelSlot<Arc<u64>> = ModelSlot::new("test");
        let (value, _) = slot.get_or_load("model-a", || Ok(Arc::new(5))).unwrap();
        assert_eq!(Arc::strong_count(&value), 2);

        slot.unload("test deactivation");
        assert!(slot.cached_key().is_none());
        assert_eq!(slot.unload_count(), 1);
        // Only the test's local Arc remains, so the engine dropped its copy.
        assert_eq!(Arc::strong_count(&value), 1);

        let (_, loaded) = slot.get_or_load("model-a", || Ok(Arc::new(5))).unwrap();
        assert!(loaded);
        assert_eq!(slot.load_count(), 2);
    }

    #[test]
    fn inference_error_does_not_poison_the_loaded_model() {
        // The engine reuses the recognizer after a per-request failure; only an
        // explicit unload or key change removes it.
        let slot: ModelSlot<Arc<u64>> = ModelSlot::new("test");
        let (first, _) = slot.get_or_load("model-a", || Ok(Arc::new(11))).unwrap();
        // ... an inference request fails here; the cache is untouched.
        let (second, loaded) = slot.get_or_load("model-a", || Ok(Arc::new(12))).unwrap();
        assert!(!loaded);
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(slot.load_count(), 1);
    }

    #[test]
    fn concurrent_get_initializes_exactly_one_instance() {
        let slot: Arc<ModelSlot<Arc<u64>>> = Arc::new(ModelSlot::new("test"));
        let loads = Arc::new(AtomicU64::new(0));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let slot = Arc::clone(&slot);
            let loads = Arc::clone(&loads);
            handles.push(std::thread::spawn(move || {
                let (value, _) = slot
                    .get_or_load("model-a", || {
                        loads.fetch_add(1, Ordering::SeqCst);
                        // Hold the slot long enough for the other threads to
                        // queue behind the initialization.
                        std::thread::sleep(std::time::Duration::from_millis(30));
                        Ok(Arc::new(21))
                    })
                    .unwrap();
                value
            }));
        }
        let values: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        assert_eq!(slot.load_count(), 1);
        let first = &values[0];
        assert!(values.iter().all(|value| Arc::ptr_eq(first, value)));
    }

    #[test]
    fn poisoned_lock_recovers_instead_of_disabling_voice_input() {
        let slot: Arc<ModelSlot<Arc<u64>>> = Arc::new(ModelSlot::new("test"));
        let poisoned = Arc::clone(&slot);
        let _ = std::thread::spawn(move || {
            let _guard = poisoned.inner.lock().unwrap();
            panic!("simulated panic while holding the cache lock");
        })
        .join();

        let (value, loaded) = slot.get_or_load("model-a", || Ok(Arc::new(3))).unwrap();
        assert!(loaded);
        assert_eq!(*value, 3);
    }
}
