//! Low-level primary-button observation without cursor polling.

use std::{
    marker::PhantomData,
    ptr,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, AtomicPtr, AtomicU8, Ordering},
        mpsc, Arc,
    },
    thread::{self, JoinHandle},
};

use windows::Win32::{
    Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, PeekMessageW, PostThreadMessageW,
        SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, MSG, PM_NOREMOVE, WH_MOUSE_LL,
        WM_APP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_QUIT,
    },
};

use crate::contracts::{AppError, AppErrorCode};

/// Primary mouse transitions emitted by the process-wide low-level hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimaryMouseEvent {
    Pressed,
    Released,
}

struct HookSink {
    mailbox: Arc<GestureMailbox>,
}

static EVENT_SINK: AtomicPtr<HookSink> = AtomicPtr::new(ptr::null_mut());

const PENDING_PRESSED: u8 = 1 << 0;
const PENDING_RELEASED: u8 = 1 << 1;
const WM_PRIMARY_MOUSE_EVENT: u32 = WM_APP + 0x2d1;

struct GestureMailbox {
    pending: AtomicU8,
    notification_pending: AtomicBool,
    stopped: AtomicBool,
    receiver_thread_id: u32,
}

impl GestureMailbox {
    fn record(&self, event: PrimaryMouseEvent) -> bool {
        if self.stopped.load(Ordering::Acquire) {
            return false;
        }
        let bit = match event {
            PrimaryMouseEvent::Pressed => PENDING_PRESSED,
            PrimaryMouseEvent::Released => PENDING_RELEASED,
        };
        self.pending.fetch_or(bit, Ordering::Release);
        true
    }

    fn take_prioritized(&self) -> Option<PrimaryMouseEvent> {
        let pending = self.pending.swap(0, Ordering::AcqRel);
        if pending & PENDING_RELEASED != 0 {
            Some(PrimaryMouseEvent::Released)
        } else if pending & PENDING_PRESSED != 0 {
            Some(PrimaryMouseEvent::Pressed)
        } else {
            None
        }
    }

    fn notify_nonblocking(&self) {
        if !self.notification_pending.swap(true, Ordering::AcqRel) {
            // SAFETY: the receiver creates its queue before publishing this
            // mailbox. At most one notification is outstanding, and this API
            // does not wait for the receiver.
            if unsafe {
                PostThreadMessageW(
                    self.receiver_thread_id,
                    WM_PRIMARY_MOUSE_EVENT,
                    WPARAM(0),
                    LPARAM(0),
                )
            }
            .is_err()
            {
                self.notification_pending.store(false, Ordering::Release);
            }
        }
    }
}

/// Callback-side publisher for the bounded latest-gesture mailbox.
pub struct PrimaryMouseEventSink {
    mailbox: Arc<GestureMailbox>,
}

/// Blocking, event-driven receiver for primary mouse transitions.
///
/// This receiver is intentionally `!Send`: Windows thread messages must be
/// consumed on the thread that created the channel. Create and consume it on a
/// dedicated adapter thread, not the UI thread.
pub struct PrimaryMouseEventReceiver {
    mailbox: Arc<GestureMailbox>,
    _thread_bound: PhantomData<Rc<()>>,
}

/// Cloneable non-blocking stop handle for a Windows primary observer.
#[derive(Clone)]
pub struct PrimaryMouseObserverStop {
    mailbox: Arc<GestureMailbox>,
}

impl PrimaryMouseObserverStop {
    /// Suppresses callback publication and wakes the blocking receiver.
    pub fn stop(&self) {
        if !self.mailbox.stopped.swap(true, Ordering::AcqRel) {
            self.mailbox.pending.store(0, Ordering::Release);
            self.mailbox.notify_nonblocking();
        }
    }
}

/// Creates a bounded coalescing event path on the current consumer thread.
///
/// Saturated bursts retain a pending `Released` and discard stale `Pressed`
/// transitions; this preserves the latest completed gesture that triggers UIA.
pub fn primary_mouse_event_channel() -> (PrimaryMouseEventSink, PrimaryMouseEventReceiver) {
    let receiver_thread_id = unsafe { GetCurrentThreadId() };
    // SAFETY: PM_NOREMOVE creates the current thread's message queue without
    // consuming application messages.
    let mut queue_probe = MSG::default();
    unsafe {
        let _ = PeekMessageW(&mut queue_probe, None, 0, 0, PM_NOREMOVE);
    }
    let mailbox = Arc::new(GestureMailbox {
        pending: AtomicU8::new(0),
        notification_pending: AtomicBool::new(false),
        stopped: AtomicBool::new(false),
        receiver_thread_id,
    });
    (
        PrimaryMouseEventSink {
            mailbox: mailbox.clone(),
        },
        PrimaryMouseEventReceiver {
            mailbox,
            _thread_bound: PhantomData,
        },
    )
}

impl PrimaryMouseEventReceiver {
    /// Returns a non-blocking handle that wakes this receiver.
    pub fn stop_handle(&self) -> PrimaryMouseObserverStop {
        PrimaryMouseObserverStop {
            mailbox: self.mailbox.clone(),
        }
    }

    /// Waits for the next coalesced transition without polling.
    pub fn recv(&self) -> Result<PrimaryMouseEvent, AppError> {
        if unsafe { GetCurrentThreadId() } != self.mailbox.receiver_thread_id {
            return Err(internal(
                "primary mouse events must be received on their channel thread",
            ));
        }

        loop {
            if self.mailbox.stopped.load(Ordering::Acquire) {
                return Err(internal("primary mouse event observer is stopped"));
            }
            let mut message = MSG::default();
            // SAFETY: this receiver is bound to the queue-owning thread and
            // supplies valid writable message storage.
            let status = unsafe {
                GetMessageW(
                    &mut message,
                    None,
                    WM_PRIMARY_MOUSE_EVENT,
                    WM_PRIMARY_MOUSE_EVENT,
                )
            };
            if status.0 == -1 {
                return Err(internal("could not receive primary mouse event"));
            }
            if status.0 == 0 {
                return Err(internal("primary mouse event thread is shutting down"));
            }

            // Clear the coalescing gate before taking the pending bits. A
            // concurrent publisher either lands in this take or posts the next
            // wakeup, so no completion can be stranded by the race.
            self.mailbox
                .notification_pending
                .store(false, Ordering::Release);
            if self.mailbox.stopped.load(Ordering::Acquire) {
                return Err(internal("primary mouse event observer is stopped"));
            }
            if let Some(event) = self.mailbox.take_prioritized() {
                return Ok(event);
            }
        }
    }
}

/// Owns the dedicated Windows message-loop thread for a low-level mouse hook.
pub struct PrimaryMouseHook {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
    sink: *mut HookSink,
}

impl Drop for PrimaryMouseHook {
    fn drop(&mut self) {
        // SAFETY: `thread_id` belongs to the live hook thread. WM_QUIT carries no
        // pointers, and joining guarantees the hook callback has stopped before
        // this owner is destroyed.
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let installed = EVENT_SINK
            .compare_exchange(
                self.sink,
                ptr::null_mut(),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok();
        if installed {
            // SAFETY: this owner allocated `sink`, the hook thread has joined,
            // and compare_exchange proves no other owner can free the pointer.
            unsafe {
                drop(Box::from_raw(self.sink));
            }
        }
    }
}

/// Installs one unelevated `WH_MOUSE_LL` hook and returns its RAII owner.
///
/// The callback performs only atomic coalescing and a non-blocking wakeup;
/// selection/UIA work must be scheduled by the receiver after `Released`.
pub fn install_primary_mouse_hook(
    sink: PrimaryMouseEventSink,
) -> Result<PrimaryMouseHook, AppError> {
    let sink = Box::into_raw(Box::new(HookSink {
        mailbox: sink.mailbox,
    }));
    if EVENT_SINK
        .compare_exchange(ptr::null_mut(), sink, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        // SAFETY: compare_exchange failed, so the pointer was never published.
        unsafe {
            drop(Box::from_raw(sink));
        }
        return Err(internal("primary mouse hook is already installed"));
    }

    let (started_tx, started_rx) = mpsc::sync_channel(1);
    let thread = thread::Builder::new()
        .name("windows-primary-mouse-hook".into())
        .spawn(move || {
            // Calling GetCurrentThreadId before startup notification establishes
            // the queue owner used by PrimaryMouseHook::drop.
            let thread_id = unsafe { GetCurrentThreadId() };
            run_message_loop(thread_id, started_tx);
        })
        .map_err(|_| {
            release_sink(sink);
            internal("could not start the mouse hook thread")
        })?;

    let (thread_id, startup) = started_rx.recv().map_err(|_| {
        release_sink(sink);
        internal("mouse hook thread exited during startup")
    })?;

    match startup {
        Ok(()) => Ok(PrimaryMouseHook {
            thread_id,
            thread: Some(thread),
            sink,
        }),
        Err(error) => {
            let _ = thread.join();
            release_sink(sink);
            Err(error)
        }
    }
}

fn run_message_loop(thread_id: u32, started: mpsc::SyncSender<(u32, Result<(), AppError>)>) {
    // SAFETY: `low_level_mouse_proc` has the ABI and lifetime required by
    // SetWindowsHookExW. The module handle is the current executable module and
    // the hook remains installed until this same thread exits its message loop.
    let hook = unsafe {
        match GetModuleHandleW(None) {
            Ok(module) => SetWindowsHookExW(
                WH_MOUSE_LL,
                Some(low_level_mouse_proc),
                Some(HINSTANCE(module.0)),
                0,
            )
            .map_err(|_| internal("could not install the low-level mouse hook")),
            Err(_) => Err(internal("could not resolve the application module")),
        }
    };
    let hook = match hook {
        Ok(hook) => {
            // SAFETY: peeking with PM_NOREMOVE creates this thread's message
            // queue before the owner may post WM_QUIT during Drop.
            let mut queue_probe = MSG::default();
            unsafe {
                let _ = PeekMessageW(&mut queue_probe, None, 0, 0, PM_NOREMOVE);
            }
            let _ = started.send((thread_id, Ok(())));
            hook
        }
        Err(error) => {
            let _ = started.send((thread_id, Err(error)));
            return;
        }
    };

    let mut message = MSG::default();
    loop {
        // SAFETY: `message` is valid writable storage for the duration of each
        // call. The thread owns the queue and dispatches messages synchronously.
        let status = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if status.0 == -1 {
            break;
        }
        if status.0 == 0 {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    // SAFETY: `hook` was installed by this thread and is unhooked exactly once
    // after the message loop terminates.
    unsafe {
        let _ = UnhookWindowsHookEx(hook);
    }
}

unsafe extern "system" fn low_level_mouse_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 {
        let event = event_from_message(wparam.0 as u32);
        if let Some(event) = event {
            let sink = EVENT_SINK.load(Ordering::Acquire);
            if !sink.is_null() {
                // SAFETY: the published sink remains allocated until the hook
                // thread has joined. Recording is one atomic operation and the
                // wakeup is a coalesced non-blocking thread-message post.
                let sink = unsafe { &*sink };
                if sink.mailbox.record(event) {
                    sink.mailbox.notify_nonblocking();
                }
            }
        }
    }

    // SAFETY: forwarding the exact callback arguments is required by the hook
    // contract. This callback never dereferences `lparam`.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn event_from_message(message: u32) -> Option<PrimaryMouseEvent> {
    match message {
        WM_LBUTTONDOWN => Some(PrimaryMouseEvent::Pressed),
        WM_LBUTTONUP => Some(PrimaryMouseEvent::Released),
        _ => None,
    }
}

fn release_sink(sink: *mut HookSink) {
    if EVENT_SINK
        .compare_exchange(sink, ptr::null_mut(), Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        // SAFETY: compare_exchange removed the unique published pointer.
        unsafe {
            drop(Box::from_raw(sink));
        }
    }
}

fn internal(message: &'static str) -> AppError {
    AppError::new(AppErrorCode::Internal, message, false)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU8};

    use windows::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE};

    use std::sync::Arc;

    use super::{event_from_message, GestureMailbox, PrimaryMouseEvent, PrimaryMouseObserverStop};

    fn mailbox() -> GestureMailbox {
        GestureMailbox {
            pending: AtomicU8::new(0),
            notification_pending: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            receiver_thread_id: 1,
        }
    }

    #[test]
    fn primary_mouse_events_are_distinct_transitions() {
        assert_ne!(PrimaryMouseEvent::Pressed, PrimaryMouseEvent::Released);
    }

    #[test]
    fn maps_only_primary_button_transitions() {
        assert_eq!(
            event_from_message(WM_LBUTTONDOWN),
            Some(PrimaryMouseEvent::Pressed)
        );
        assert_eq!(
            event_from_message(WM_LBUTTONUP),
            Some(PrimaryMouseEvent::Released)
        );
        assert_eq!(event_from_message(WM_MOUSEMOVE), None);
    }

    #[test]
    fn saturation_prioritizes_release_over_stale_press() {
        let mailbox = mailbox();
        assert!(mailbox.record(PrimaryMouseEvent::Pressed));
        assert!(mailbox.record(PrimaryMouseEvent::Released));
        assert!(mailbox.record(PrimaryMouseEvent::Pressed));

        assert_eq!(
            mailbox.take_prioritized(),
            Some(PrimaryMouseEvent::Released)
        );
        assert_eq!(mailbox.take_prioritized(), None);
    }

    #[test]
    fn unsaturated_cycle_preserves_both_transitions() {
        let mailbox = mailbox();
        assert!(mailbox.record(PrimaryMouseEvent::Pressed));
        assert_eq!(mailbox.take_prioritized(), Some(PrimaryMouseEvent::Pressed));
        assert!(mailbox.record(PrimaryMouseEvent::Released));
        assert_eq!(
            mailbox.take_prioritized(),
            Some(PrimaryMouseEvent::Released)
        );
    }

    #[test]
    fn repeated_saturated_click_cycles_each_preserve_completion() {
        let mailbox = mailbox();
        for _ in 0..8 {
            assert!(mailbox.record(PrimaryMouseEvent::Pressed));
            assert!(mailbox.record(PrimaryMouseEvent::Released));
            assert_eq!(
                mailbox.take_prioritized(),
                Some(PrimaryMouseEvent::Released)
            );
        }
    }

    #[test]
    fn burst_of_click_cycles_coalesces_to_latest_completion() {
        let mailbox = mailbox();
        for _ in 0..8 {
            assert!(mailbox.record(PrimaryMouseEvent::Pressed));
            assert!(mailbox.record(PrimaryMouseEvent::Released));
        }

        assert_eq!(
            mailbox.take_prioritized(),
            Some(PrimaryMouseEvent::Released)
        );
        assert_eq!(mailbox.take_prioritized(), None);
    }

    #[test]
    fn stopping_is_idempotent_and_suppresses_callback_publication() {
        let mailbox = Arc::new(mailbox());
        let stop = PrimaryMouseObserverStop {
            mailbox: mailbox.clone(),
        };
        assert!(mailbox.record(PrimaryMouseEvent::Pressed));

        stop.stop();
        stop.stop();

        assert!(!mailbox.record(PrimaryMouseEvent::Released));
        assert_eq!(mailbox.take_prioritized(), None);
    }
}
