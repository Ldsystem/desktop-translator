//! Focused-selection acquisition through Windows UI Automation TextPattern.

use std::{
    ffi::c_void,
    future::Future,
    pin::Pin,
    sync::{mpsc, Arc, Mutex},
    task::{Context, Poll, Waker},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use windows::{
    core::Error as WindowsError,
    Win32::{
        Foundation::E_ACCESSDENIED,
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
                COINIT_MULTITHREADED, SAFEARRAY,
            },
            Ole::{
                SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetDim, SafeArrayGetLBound,
                SafeArrayGetUBound, SafeArrayUnaccessData,
            },
            Threading::GetCurrentProcessId,
        },
        UI::Accessibility::{
            CUIAutomation, IUIAutomation, IUIAutomationTextPattern, IUIAutomationTextRange,
            UIA_TextPatternId, UIA_E_ELEMENTNOTAVAILABLE, UIA_E_NOTSUPPORTED,
        },
    },
};

use crate::{
    contracts::{AppError, AppErrorCode, PhysicalRect, SelectionSnapshot},
    placement::final_visible_line,
    platform::{SelectionAdapter, SelectionPolicy},
};

struct SelectionRequest {
    policy: SelectionPolicy,
    reply: ReplySender<Result<SelectionSnapshot, AppError>>,
}

/// UI Automation adapter backed by one dedicated COM MTA worker.
#[derive(Clone)]
pub struct WindowsSelectionAdapter {
    requests: mpsc::Sender<SelectionRequest>,
}

impl WindowsSelectionAdapter {
    /// Starts the UI Automation worker without elevating the process.
    pub fn new() -> Result<Self, AppError> {
        let (requests, receiver) = mpsc::channel();
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("windows-uia-selection".into())
            .spawn(move || run_uia_worker(receiver, started_tx))
            .map_err(|_| internal("could not start UI Automation worker"))?;
        started_rx
            .recv()
            .map_err(|_| internal("UI Automation worker exited during startup"))??;
        Ok(Self { requests })
    }
}

#[async_trait]
impl SelectionAdapter for WindowsSelectionAdapter {
    async fn resolve_selection(
        &self,
        policy: &SelectionPolicy,
    ) -> Result<SelectionSnapshot, AppError> {
        let (reply, response) = reply_channel(Err(internal(
            "UI Automation worker exited before completing selection",
        )));
        self.requests
            .send(SelectionRequest {
                policy: policy.clone(),
                reply,
            })
            .map_err(|_| internal("UI Automation worker is unavailable"))?;
        response.await
    }
}

fn run_uia_worker(
    requests: mpsc::Receiver<SelectionRequest>,
    started: mpsc::SyncSender<Result<(), AppError>>,
) {
    // SAFETY: COM is initialized and uninitialized on this same dedicated
    // thread. All UI Automation interfaces remain confined to this thread.
    let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if initialized.is_err() {
        let _ = started.send(Err(internal("could not initialize UI Automation COM")));
        return;
    }

    // SAFETY: CUIAutomation is an in-process COM class and the returned
    // interface remains thread-confined until before CoUninitialize.
    let automation =
        unsafe { CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER) };
    let automation = match automation {
        Ok(value) => {
            let _ = started.send(Ok(()));
            value
        }
        Err(_) => {
            let _ = started.send(Err(internal("could not create UI Automation client")));
            unsafe { CoUninitialize() };
            return;
        }
    };

    for request in requests {
        let result = resolve_focused_selection(&automation, &request.policy);
        request.reply.complete(result);
    }

    drop(automation);
    unsafe { CoUninitialize() };
}

fn resolve_focused_selection(
    automation: &IUIAutomation,
    policy: &SelectionPolicy,
) -> Result<SelectionSnapshot, AppError> {
    // SAFETY: the automation object is valid and thread-confined.
    let element = unsafe { automation.GetFocusedElement() }.map_err(map_uia_error)?;
    let process_id = unsafe { element.CurrentProcessId() }.map_err(map_uia_error)?;
    let source_application_id = process_id.to_string();

    if is_own_process(process_id, unsafe { GetCurrentProcessId() }) {
        return Err(no_selection("selection belongs to this application"));
    }
    if policy
        .excluded_application_id
        .as_deref()
        .is_some_and(|excluded| excluded == source_application_id)
    {
        return Err(no_selection("selection belongs to this application"));
    }
    if unsafe { element.CurrentIsPassword() }
        .map_err(map_uia_error)?
        .as_bool()
    {
        return Err(unsupported("protected controls are not readable"));
    }
    if unsafe { element.CurrentIsOffscreen() }
        .map_err(map_uia_error)?
        .as_bool()
    {
        return Err(no_selection("focused control is off-screen"));
    }

    // SAFETY: GetCurrentPatternAs performs the COM QueryInterface for the
    // requested TextPattern; unsupported/elevated providers are mapped below.
    let text_pattern: IUIAutomationTextPattern =
        unsafe { element.GetCurrentPatternAs(UIA_TextPatternId) }.map_err(map_uia_error)?;
    let selection = unsafe { text_pattern.GetSelection() }.map_err(map_uia_error)?;
    let selection_count = unsafe { selection.Length() }.map_err(map_uia_error)?;
    if selection_count <= 0 {
        return Err(no_selection("control exposes no selected text range"));
    }

    let mut text_parts = Vec::new();
    let mut rectangles = Vec::new();
    for index in 0..selection_count {
        let range = unsafe { selection.GetElement(index) }.map_err(map_uia_error)?;
        let text = range_text(&range)?;
        if !text.is_empty() {
            text_parts.push(text);
        }
        rectangles.extend(range_rectangles(&range)?);
    }

    let text = text_parts.join("\n");
    if text.trim().is_empty() {
        return Err(no_selection("selected text is empty"));
    }
    if text.chars().count() > policy.max_code_points {
        return Err(unsupported("selected text exceeds the configured limit"));
    }
    let anchor_physical_px = final_visible_line(&rectangles)
        .ok_or_else(|| no_selection("selection exposes no visible geometry"))?;

    Ok(SelectionSnapshot {
        id: next_selection_id(),
        text,
        source_application_id: Some(source_application_id),
        bounds_physical_px: rectangles,
        anchor_physical_px,
        captured_at_epoch_ms: epoch_millis(),
    })
}

fn is_own_process(uia_process_id: i32, current_process_id: u32) -> bool {
    u32::try_from(uia_process_id).ok() == Some(current_process_id)
}

fn range_text(range: &IUIAutomationTextRange) -> Result<String, AppError> {
    let text = unsafe { range.GetText(-1) }.map_err(map_uia_error)?;
    Ok(text.to_string())
}

fn range_rectangles(range: &IUIAutomationTextRange) -> Result<Vec<PhysicalRect>, AppError> {
    let array = unsafe { range.GetBoundingRectangles() }.map_err(map_uia_error)?;
    if array.is_null() {
        return Ok(Vec::new());
    }
    let values = copy_and_destroy_f64_safearray(array)?;
    Ok(rectangles_from_flat_values(&values))
}

fn copy_and_destroy_f64_safearray(array: *mut SAFEARRAY) -> Result<Vec<f64>, AppError> {
    let copy_result = copy_f64_safearray(array);
    // SAFETY: the array was returned with caller ownership. copy_f64_safearray
    // always balances a successful access before returning, so destruction is
    // attempted only after the lock has been released.
    let destroy_result = unsafe { SafeArrayDestroy(array) }
        .map_err(|_| internal("could not destroy UI Automation geometry"));
    match (copy_result, destroy_result) {
        (Ok(values), Ok(())) => Ok(values),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn copy_f64_safearray(array: *mut SAFEARRAY) -> Result<Vec<f64>, AppError> {
    if unsafe { SafeArrayGetDim(array) } != 1 {
        return Err(internal(
            "UI Automation geometry must be a one-dimensional array",
        ));
    }
    let bounds = (|| {
        let lower = unsafe { SafeArrayGetLBound(array, 1) }
            .map_err(|_| internal("invalid UI Automation geometry array"))?;
        let upper = unsafe { SafeArrayGetUBound(array, 1) }
            .map_err(|_| internal("invalid UI Automation geometry array"))?;
        if upper < lower {
            Ok(0)
        } else {
            usize::try_from(upper - lower + 1)
                .map_err(|_| internal("UI Automation geometry array is too large"))
        }
    })()?;
    if bounds == 0 {
        return Ok(Vec::new());
    }

    let mut raw: *mut c_void = std::ptr::null_mut();
    unsafe {
        SafeArrayAccessData(array, &mut raw)
            .map_err(|_| internal("could not access UI Automation geometry"))?;
    }

    let copy_result = if raw.is_null() {
        Err(internal("UI Automation returned invalid geometry"))
    } else {
        // SAFETY: UI Automation documents this SAFEARRAY as contiguous VT_R8
        // values. Bounds determine the exact readable count, copied while the
        // array is locked.
        Ok(unsafe { std::slice::from_raw_parts(raw.cast::<f64>(), bounds).to_vec() })
    };
    // SAFETY: this call exactly balances the successful SafeArrayAccessData,
    // including the null-data error path.
    let unaccess_result = unsafe { SafeArrayUnaccessData(array) }
        .map_err(|_| internal("could not release UI Automation geometry"));

    match (copy_result, unaccess_result) {
        (Ok(values), Ok(())) => Ok(values),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn rectangles_from_flat_values(values: &[f64]) -> Vec<PhysicalRect> {
    values
        .chunks_exact(4)
        .filter_map(|quad| {
            let rect = PhysicalRect {
                x: quad[0],
                y: quad[1],
                width: quad[2],
                height: quad[3],
            };
            (rect.x.is_finite()
                && rect.y.is_finite()
                && rect.width.is_finite()
                && rect.height.is_finite()
                && rect.width > 0.0
                && rect.height > 0.0)
                .then_some(rect)
        })
        .collect()
}

fn map_uia_error(error: WindowsError) -> AppError {
    map_uia_hresult(error.code())
}

fn map_uia_hresult(code: windows::core::HRESULT) -> AppError {
    if code == E_ACCESSDENIED || code == UIA_E_NOTSUPPORTED || code == UIA_E_ELEMENTNOTAVAILABLE {
        unsupported("control is protected, elevated, or does not expose TextPattern")
    } else {
        internal("UI Automation could not resolve the selection")
    }
}

fn next_selection_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    const JS_SAFE_INTEGER_MAX: u64 = 9_007_199_254_740_991;
    NEXT_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            Some(if current >= JS_SAFE_INTEGER_MAX {
                1
            } else {
                current + 1
            })
        })
        .unwrap_or(1)
}

fn epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
        .min(9_007_199_254_740_991)
}

fn no_selection(message: &'static str) -> AppError {
    AppError::new(AppErrorCode::NoSelection, message, false)
}

fn unsupported(message: &'static str) -> AppError {
    AppError::new(AppErrorCode::UnsupportedControl, message, false)
}

fn internal(message: &'static str) -> AppError {
    AppError::new(AppErrorCode::Internal, message, false)
}

struct ReplyState<T> {
    value: Option<T>,
    waker: Option<Waker>,
}

struct ReplySender<T> {
    state: Arc<Mutex<ReplyState<T>>>,
    fallback: Option<T>,
}
struct ReplyReceiver<T>(Arc<Mutex<ReplyState<T>>>);

fn reply_channel<T>(fallback: T) -> (ReplySender<T>, ReplyReceiver<T>) {
    let state = Arc::new(Mutex::new(ReplyState {
        value: None,
        waker: None,
    }));
    (
        ReplySender {
            state: state.clone(),
            fallback: Some(fallback),
        },
        ReplyReceiver(state),
    )
}

impl<T> ReplySender<T> {
    fn complete(mut self, value: T) {
        self.fallback = None;
        complete_reply(&self.state, value);
    }
}

impl<T> Drop for ReplySender<T> {
    fn drop(&mut self) {
        if let Some(fallback) = self.fallback.take() {
            complete_reply(&self.state, fallback);
        }
    }
}

fn complete_reply<T>(state: &Arc<Mutex<ReplyState<T>>>, value: T) {
    let mut state = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.value = Some(value);
    if let Some(waker) = state.waker.take() {
        waker.wake();
    }
}

impl<T> Future for ReplyReceiver<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(value) = state.value.take() {
            Poll::Ready(value)
        } else {
            state.waker = Some(context.waker().clone());
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{future::Future, pin::Pin, task::Context};

    use windows::Win32::Foundation::E_ACCESSDENIED;

    use crate::contracts::AppErrorCode;

    use super::{is_own_process, map_uia_hresult, rectangles_from_flat_values, reply_channel};

    #[test]
    fn converts_uia_quadruples_and_rejects_invalid_geometry() {
        let rectangles = rectangles_from_flat_values(&[
            10.0,
            20.0,
            30.0,
            40.0,
            1.0,
            2.0,
            0.0,
            3.0,
            f64::NAN,
            2.0,
            3.0,
            4.0,
        ]);

        assert_eq!(rectangles.len(), 1);
        assert_eq!(rectangles[0].x, 10.0);
        assert_eq!(rectangles[0].height, 40.0);
    }

    #[test]
    fn ignores_incomplete_trailing_uia_values() {
        let rectangles = rectangles_from_flat_values(&[1.0, 2.0, 3.0, 4.0, 99.0]);
        assert_eq!(rectangles.len(), 1);
    }

    #[test]
    fn maps_access_denied_to_stable_unsupported_control() {
        let error = map_uia_hresult(E_ACCESSDENIED);
        assert_eq!(error.code, AppErrorCode::UnsupportedControl);
        assert!(!error.retryable);
    }

    #[test]
    fn compares_signed_uia_process_ids_without_wrapping() {
        assert!(is_own_process(42, 42));
        assert!(!is_own_process(-1, u32::MAX));
        assert!(!is_own_process(41, 42));
    }

    #[test]
    fn dropped_worker_reply_completes_with_fallback() {
        let (reply, mut response) =
            reply_channel::<Result<(), _>>(Err(super::internal("worker exited")));
        drop(reply);
        let mut context = Context::from_waker(std::task::Waker::noop());

        let result = Pin::new(&mut response).poll(&mut context);
        assert!(matches!(
            result,
            std::task::Poll::Ready(Err(error)) if error.code == AppErrorCode::Internal
        ));
    }
}
