//! Event-driven primary mouse observation using a listen-only Quartz event tap.

use std::{
    ffi::c_void,
    ptr,
    sync::{
        atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc,
    },
    thread::{self, JoinHandle},
};

use crate::placement::PhysicalPoint;

type CGEventTapProxy = *mut c_void;
type CGEventRef = *mut c_void;
type CFMachPortRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CFRunLoopRef = *mut c_void;
type CFStringRef = *const c_void;
type CFAllocatorRef = *const c_void;
type CGEventType = u32;
type CGEventMask = u64;

const LEFT_MOUSE_DOWN: CGEventType = 1;
const LEFT_MOUSE_UP: CGEventType = 2;
const SESSION_EVENT_TAP: u32 = 1;
const HEAD_INSERT_EVENT_TAP: u32 = 0;
const LISTEN_ONLY_EVENT_TAP: u32 = 1;
const TAP_DISABLED_BY_TIMEOUT: CGEventType = u32::MAX - 1;
const TAP_DISABLED_BY_USER_INPUT: CGEventType = u32::MAX;
const EVENT_QUEUE_CAPACITY: usize = 32;
const MOUSE_EVENT_CLICK_STATE: u32 = 1;
const SELECTION_DRAG_MIN_DISTANCE: f64 = 3.0;

type EventTapCallback =
    unsafe extern "C" fn(CGEventTapProxy, CGEventType, CGEventRef, *mut c_void) -> CGEventRef;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        location: u32,
        placement: u32,
        options: u32,
        events_of_interest: CGEventMask,
        callback: EventTapCallback,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: u8);
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopDefaultMode: CFStringRef;
    fn CFRelease(value: *const c_void);
    fn CFMachPortCreateRunLoopSource(
        allocator: CFAllocatorRef,
        port: CFMachPortRef,
        order: isize,
    ) -> CFRunLoopSourceRef;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(run_loop: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRun();
    fn CFRunLoopStop(run_loop: CFRunLoopRef);
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PrimaryMouseEvent {
    Pressed {
        position: PhysicalPoint,
        click_count: u8,
    },
    Released {
        position: PhysicalPoint,
        click_count: u8,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PrimaryGestureState {
    pressed: Option<(PhysicalPoint, u8)>,
}

impl PrimaryGestureState {
    /// Returns true only for a drag selection or a multi-click selection.
    pub fn observe(&mut self, event: PrimaryMouseEvent) -> bool {
        match event {
            PrimaryMouseEvent::Pressed {
                position,
                click_count,
            } => {
                self.pressed = Some((position, click_count));
                false
            }
            PrimaryMouseEvent::Released {
                position,
                click_count,
            } => self
                .pressed
                .take()
                .is_some_and(|(pressed_at, pressed_click_count)| {
                    pressed_click_count.max(click_count) >= 2
                        || pointer_distance(pressed_at, position) >= SELECTION_DRAG_MIN_DISTANCE
                }),
        }
    }
}

fn pointer_distance(start: PhysicalPoint, end: PhysicalPoint) -> f64 {
    (end.x - start.x).hypot(end.y - start.y)
}

/// Owns the event-tap thread and its blocking receiver.
///
/// Dropping the observer stops the run loop and joins the worker, so the
/// callback context cannot outlive its channel.
pub struct PrimaryMouseObserver {
    events: Receiver<PrimaryMouseEvent>,
    dropped_events: Arc<AtomicU64>,
    stop: Arc<ObserverStopState>,
    worker: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct ObserverStopState {
    stopped: AtomicBool,
    run_loop: AtomicPtr<c_void>,
}

impl ObserverStopState {
    fn stop(&self) -> bool {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return false;
        }
        let run_loop = self.run_loop.load(Ordering::Acquire);
        if !run_loop.is_null() {
            // SAFETY: the event-tap worker owns this run loop until it exits;
            // stopping is thread-safe and the worker is joined before release.
            unsafe { CFRunLoopStop(run_loop) };
        }
        true
    }

    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
}

/// Cloneable non-blocking stop handle for a macOS primary observer.
#[derive(Clone)]
pub struct PrimaryMouseObserverStop {
    state: Arc<ObserverStopState>,
}

impl PrimaryMouseObserverStop {
    /// Stops event delivery and wakes the observer's blocking receiver.
    pub fn stop(&self) {
        self.state.stop();
    }
}

impl PrimaryMouseObserver {
    pub fn start() -> Result<Self, &'static str> {
        let (event_tx, event_rx) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let dropped_events = Arc::new(AtomicU64::new(0));
        let callback_dropped_events = Arc::clone(&dropped_events);
        let stop = Arc::new(ObserverStopState::default());
        let worker_stop = stop.clone();
        let worker = thread::Builder::new()
            .name("macos-primary-event-tap".into())
            .spawn(move || {
                event_tap_thread(event_tx, callback_dropped_events, worker_stop, ready_tx)
            })
            .map_err(|_| "could not start mouse event observer")?;

        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(message)) => {
                let _ = worker.join();
                return Err(message);
            }
            Err(_) => {
                let _ = worker.join();
                return Err("mouse event observer stopped during startup");
            }
        };

        Ok(Self {
            events: event_rx,
            dropped_events,
            stop,
            worker: Some(worker),
        })
    }

    /// Blocks until the next event arrives. No idle polling is performed.
    pub fn recv(&self) -> Result<PrimaryMouseEvent, mpsc::RecvError> {
        self.events.recv()
    }

    pub fn receiver(&self) -> &Receiver<PrimaryMouseEvent> {
        &self.events
    }

    /// Returns a non-blocking handle that wakes this observer.
    pub fn stop_handle(&self) -> PrimaryMouseObserverStop {
        PrimaryMouseObserverStop {
            state: self.stop.clone(),
        }
    }

    /// Number of events discarded to keep the Quartz callback nonblocking.
    pub fn dropped_event_count(&self) -> u64 {
        self.dropped_events.load(Ordering::Relaxed)
    }
}

impl Drop for PrimaryMouseObserver {
    fn drop(&mut self) {
        self.stop.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn event_tap_thread(
    events: SyncSender<PrimaryMouseEvent>,
    dropped_events: Arc<AtomicU64>,
    stop: Arc<ObserverStopState>,
    ready: mpsc::SyncSender<Result<(), &'static str>>,
) {
    let context = Box::new(EventTapContext {
        events,
        dropped_events,
        tap: AtomicPtr::new(ptr::null_mut()),
        stop: stop.clone(),
    });
    let context_ptr = Box::into_raw(context);
    let event_mask = (1_u64 << LEFT_MOUSE_DOWN) | (1_u64 << LEFT_MOUSE_UP);
    // SAFETY: callback and context remain valid until CFRunLoopRun returns.
    let tap = unsafe {
        CGEventTapCreate(
            SESSION_EVENT_TAP,
            HEAD_INSERT_EVENT_TAP,
            LISTEN_ONLY_EVENT_TAP,
            event_mask,
            event_tap_callback,
            context_ptr.cast::<c_void>(),
        )
    };
    if tap.is_null() {
        // SAFETY: CGEventTapCreate did not retain the context on failure.
        drop(unsafe { Box::from_raw(context_ptr) });
        let _ = ready.send(Err(
            "Quartz event tap unavailable; Accessibility permission may be missing",
        ));
        return;
    }
    // SAFETY: the context remains allocated through the event-tap lifetime.
    unsafe { (*context_ptr).tap.store(tap, Ordering::Release) };

    // SAFETY: tap is a live CFMachPort and the source follows the create rule.
    let source = unsafe { CFMachPortCreateRunLoopSource(ptr::null(), tap, 0) };
    if source.is_null() {
        // SAFETY: tap is +1 from CGEventTapCreate and context is still ours.
        unsafe { CFRelease(tap.cast_const()) };
        drop(unsafe { Box::from_raw(context_ptr) });
        let _ = ready.send(Err("could not create the event-tap run-loop source"));
        return;
    }

    // SAFETY: current run loop is valid for this thread through CFRunLoopRun.
    let run_loop = unsafe { CFRunLoopGetCurrent() };
    stop.run_loop.store(run_loop, Ordering::Release);
    unsafe { CFRunLoopAddSource(run_loop, source, kCFRunLoopDefaultMode) };
    if ready.send(Ok(())).is_ok() && !stop.is_stopped() {
        // SAFETY: source has been installed and Drop stops this run loop.
        unsafe { CFRunLoopRun() };
    }

    // SAFETY: source/tap are +1 create-rule objects; the run loop has stopped,
    // so Quartz can no longer invoke the callback before context destruction.
    unsafe {
        CFRelease(source.cast_const());
        CFRelease(tap.cast_const());
        drop(Box::from_raw(context_ptr));
    }
    stop.run_loop.store(ptr::null_mut(), Ordering::Release);
}

unsafe extern "C" fn event_tap_callback(
    _proxy: CGEventTapProxy,
    event_type: CGEventType,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef {
    // SAFETY: user_info points to EventTapContext for the installed tap lifetime.
    let context = unsafe { &*user_info.cast::<EventTapContext>() };
    if matches!(
        event_type,
        TAP_DISABLED_BY_TIMEOUT | TAP_DISABLED_BY_USER_INPUT
    ) {
        if context.stop.is_stopped() {
            return event;
        }
        let tap = context.tap.load(Ordering::Acquire);
        if !tap.is_null() {
            // SAFETY: tap remains live until after the run loop stops.
            unsafe { CGEventTapEnable(tap, 1) };
        }
        return event;
    }
    // SAFETY: Quartz supplies a live event for mouse callbacks.
    let Some(position) = (unsafe { super::selection::event_location_logical(event.cast_const()) })
    else {
        return event;
    };
    // SAFETY: Quartz supplies a live mouse event and click state is an integer field.
    let click_count = unsafe { CGEventGetIntegerValueField(event, MOUSE_EVENT_CLICK_STATE) }
        .clamp(0, i64::from(u8::MAX)) as u8;
    let observed = match event_type {
        LEFT_MOUSE_DOWN => Some(PrimaryMouseEvent::Pressed {
            position,
            click_count,
        }),
        LEFT_MOUSE_UP => Some(PrimaryMouseEvent::Released {
            position,
            click_count,
        }),
        _ => None,
    };
    if let Some(observed) = observed {
        deliver_event(context, observed);
    }
    event
}

struct EventTapContext {
    events: SyncSender<PrimaryMouseEvent>,
    dropped_events: Arc<AtomicU64>,
    tap: AtomicPtr<c_void>,
    stop: Arc<ObserverStopState>,
}

fn deliver_event(context: &EventTapContext, event: PrimaryMouseEvent) {
    if context.stop.is_stopped() {
        return;
    }
    match context.events.try_send(event) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            context.dropped_events.fetch_add(1, Ordering::Relaxed);
        }
        Err(TrySendError::Disconnected(_)) => {}
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicPtr, AtomicU64},
        mpsc, Arc,
    };

    use super::{
        deliver_event, EventTapContext, ObserverStopState, PrimaryGestureState, PrimaryMouseEvent,
    };

    fn point(x: f64, y: f64) -> crate::placement::PhysicalPoint {
        crate::placement::PhysicalPoint { x, y }
    }

    #[test]
    fn completes_only_a_matching_primary_press_and_release() {
        let mut state = PrimaryGestureState::default();
        assert!(!state.observe(PrimaryMouseEvent::Released {
            position: point(10.0, 10.0),
            click_count: 1,
        }));
        assert!(!state.observe(PrimaryMouseEvent::Pressed {
            position: point(10.0, 10.0),
            click_count: 1,
        }));
        assert!(state.observe(PrimaryMouseEvent::Released {
            position: point(20.0, 20.0),
            click_count: 1,
        }));
        assert!(!state.observe(PrimaryMouseEvent::Released {
            position: point(20.0, 20.0),
            click_count: 1,
        }));
    }

    #[test]
    fn an_unmoved_single_click_is_not_a_selection_gesture() {
        let mut state = PrimaryGestureState::default();
        assert!(!state.observe(PrimaryMouseEvent::Pressed {
            position: point(10.0, 10.0),
            click_count: 1,
        }));
        assert!(!state.observe(PrimaryMouseEvent::Released {
            position: point(10.0, 10.0),
            click_count: 1,
        }));
    }

    #[test]
    fn multi_click_selection_starts_with_the_second_click() {
        let mut state = PrimaryGestureState::default();
        for click_count in 1..=3 {
            assert!(!state.observe(PrimaryMouseEvent::Pressed {
                position: point(1.0, 1.0),
                click_count,
            }));
            assert_eq!(
                state.observe(PrimaryMouseEvent::Released {
                    position: point(1.0, 1.0),
                    click_count,
                }),
                click_count >= 2
            );
        }
    }

    #[test]
    fn callback_delivery_is_bounded_and_nonblocking() {
        let (events, receiver) = mpsc::sync_channel(1);
        let dropped_events = Arc::new(AtomicU64::new(0));
        let context = EventTapContext {
            events,
            dropped_events: Arc::clone(&dropped_events),
            tap: AtomicPtr::default(),
            stop: Arc::new(ObserverStopState::default()),
        };

        let pressed = PrimaryMouseEvent::Pressed {
            position: point(2200.0, 500.0),
            click_count: 1,
        };
        deliver_event(&context, pressed);
        deliver_event(
            &context,
            PrimaryMouseEvent::Released {
                position: point(2200.0, 500.0),
                click_count: 1,
            },
        );

        assert_eq!(receiver.try_recv(), Ok(pressed));
        assert_eq!(dropped_events.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[test]
    fn stopping_is_idempotent_and_suppresses_callback_delivery() {
        let (events, receiver) = mpsc::sync_channel(1);
        let stop = Arc::new(ObserverStopState::default());
        let context = EventTapContext {
            events,
            dropped_events: Arc::new(AtomicU64::new(0)),
            tap: AtomicPtr::default(),
            stop: stop.clone(),
        };

        let pressed = PrimaryMouseEvent::Pressed {
            position: point(2200.0, 500.0),
            click_count: 1,
        };
        deliver_event(&context, pressed);
        assert!(stop.stop());
        assert!(!stop.stop());
        deliver_event(
            &context,
            PrimaryMouseEvent::Released {
                position: point(2200.0, 500.0),
                click_count: 1,
            },
        );

        assert_eq!(receiver.try_recv(), Ok(pressed));
        assert!(receiver.try_recv().is_err());
    }
}
