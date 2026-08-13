//! Narrow operating-system adapter contracts for selection, overlays, and speech.

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

use async_trait::async_trait;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    thread::JoinHandle,
};

use crate::contracts::{AppError, SelectionSnapshot, TranslationResult};

/// Wakeable owner for one blocking platform observer worker.
pub struct ObserverLifecycle {
    stop: Box<dyn Fn() + Send + Sync>,
    worker: Mutex<Option<JoinHandle<()>>>,
    stopped: AtomicBool,
}

impl ObserverLifecycle {
    /// Binds a non-blocking wake action to the worker it terminates.
    pub fn new(stop: impl Fn() + Send + Sync + 'static, worker: JoinHandle<()>) -> Self {
        Self {
            stop: Box::new(stop),
            worker: Mutex::new(Some(worker)),
            stopped: AtomicBool::new(false),
        }
    }

    /// Wakes and joins the worker exactly once without polling.
    pub fn stop_and_join(&self) {
        if !self.stopped.swap(true, Ordering::AcqRel) {
            (self.stop)();
        }
        if let Some(worker) = self.worker.lock().expect("observer worker").take() {
            let _ = worker.join();
        }
    }

    /// Reports whether stop has been requested.
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
}

impl Drop for ObserverLifecycle {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

enum ObserverManagerState {
    Stopped,
    Starting,
    Running(ObserverLifecycle),
}

/// Serializes observer start, stop/join, and restart transitions.
pub struct ObserverManager {
    state: Mutex<ObserverManagerState>,
}

impl Default for ObserverManager {
    fn default() -> Self {
        Self {
            state: Mutex::new(ObserverManagerState::Stopped),
        }
    }
}

impl ObserverManager {
    /// Reserves the only observer start slot.
    pub fn begin_start(&self) -> bool {
        let mut state = self.state.lock().expect("observer manager");
        if matches!(*state, ObserverManagerState::Stopped) {
            *state = ObserverManagerState::Starting;
            true
        } else {
            false
        }
    }

    /// Publishes a successfully started observer.
    pub fn finish_start(&self, lifecycle: ObserverLifecycle) -> bool {
        let mut state = self.state.lock().expect("observer manager");
        if matches!(*state, ObserverManagerState::Starting) {
            *state = ObserverManagerState::Running(lifecycle);
            true
        } else {
            drop(state);
            lifecycle.stop_and_join();
            false
        }
    }

    /// Releases a start reservation after startup failure.
    pub fn cancel_start(&self) {
        let mut state = self.state.lock().expect("observer manager");
        if matches!(*state, ObserverManagerState::Starting) {
            *state = ObserverManagerState::Stopped;
        }
    }

    /// Wakes and joins the current observer before allowing restart.
    pub fn stop(&self) {
        let lifecycle = {
            let mut state = self.state.lock().expect("observer manager");
            match std::mem::replace(&mut *state, ObserverManagerState::Stopped) {
                ObserverManagerState::Running(lifecycle) => Some(lifecycle),
                _ => None,
            }
        };
        if let Some(lifecycle) = lifecycle {
            lifecycle.stop_and_join();
        }
    }

    /// Reports whether an observer has completed startup.
    pub fn is_running(&self) -> bool {
        matches!(
            *self.state.lock().expect("observer manager"),
            ObserverManagerState::Running(_)
        )
    }
}

impl Drop for ObserverManager {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Selection-specific limits passed to a native adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionPolicy {
    pub max_code_points: usize,
    pub excluded_application_id: Option<String>,
}

/// Resolves selected text and geometry from the focused native control.
#[async_trait]
pub trait SelectionAdapter: Send + Sync {
    /// Returns one eligible selection or a stable selection error.
    async fn resolve_selection(
        &self,
        policy: &SelectionPolicy,
    ) -> Result<SelectionSnapshot, AppError>;
}

/// Controls the reusable non-activating contextual surface.
#[async_trait]
pub trait OverlayController: Send + Sync {
    /// Shows the contextual translate button for a selection.
    async fn show_button(&self, selection: &SelectionSnapshot) -> Result<(), AppError>;

    /// Replaces the contextual surface with immediate loading feedback.
    async fn show_loading(&self, selection: &SelectionSnapshot) -> Result<(), AppError>;

    /// Shows a translation correlated to the current selection.
    async fn show_result(
        &self,
        selection: &SelectionSnapshot,
        result: &TranslationResult,
    ) -> Result<(), AppError>;

    /// Shows a stable user-facing error for the current selection.
    async fn show_error(
        &self,
        selection: &SelectionSnapshot,
        error: &AppError,
    ) -> Result<(), AppError>;

    /// Hides the contextual surface and drops displayed content.
    async fn hide(&self) -> Result<(), AppError>;
}

/// Provides operating-system speech synthesis without cloud audio.
#[async_trait]
pub trait SpeechAdapter: Send + Sync {
    /// Reports whether an installed voice can speak the language.
    async fn is_available(&self, language: &str) -> bool;

    /// Speaks text using an installed operating-system voice.
    async fn speak(&self, text: &str, language: &str) -> Result<(), AppError>;

    /// Stops current speech immediately.
    async fn stop(&self) -> Result<(), AppError>;
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc,
        },
        thread,
    };

    use super::{ObserverLifecycle, ObserverManager};

    #[test]
    fn observer_lifecycle_wakes_and_joins_a_blocking_worker_once() {
        let (wake, blocked) = mpsc::sync_channel(1);
        let exited = Arc::new(AtomicBool::new(false));
        let worker_exited = exited.clone();
        let worker = thread::spawn(move || {
            let _ = blocked.recv();
            worker_exited.store(true, Ordering::Release);
        });
        let lifecycle = ObserverLifecycle::new(
            move || {
                let _ = wake.try_send(());
            },
            worker,
        );

        lifecycle.stop_and_join();
        lifecycle.stop_and_join();

        assert!(exited.load(Ordering::Acquire));
        assert!(lifecycle.is_stopped());
    }

    #[test]
    fn observer_manager_allows_restart_only_after_a_joined_stop() {
        let manager = ObserverManager::default();
        let (wake, blocked) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let _ = blocked.recv();
        });

        assert!(manager.begin_start());
        assert!(manager.finish_start(ObserverLifecycle::new(
            move || {
                let _ = wake.try_send(());
            },
            worker,
        )));
        assert!(manager.is_running());
        assert!(!manager.begin_start());

        manager.stop();
        assert!(!manager.is_running());
        assert!(manager.begin_start());
        manager.cancel_start();
        assert!(manager.begin_start());
    }
}
