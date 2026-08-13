//! Focused-selection acquisition through macOS Accessibility.
//!
//! The FFI layer is deliberately private. Every returned Core Foundation object
//! is wrapped immediately so ownership cannot leak into the safe adapter.

use std::{
    borrow::Cow,
    ffi::{c_char, c_double, c_float, c_int, c_long, c_void, CStr, CString},
    mem,
    ptr::{self, NonNull},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;

use crate::{
    contracts::{AppError, AppErrorCode, PhysicalRect, SelectionSnapshot},
    platform::{SelectionAdapter, SelectionPolicy},
};

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type AXUIElementRef = *const c_void;
type AXValueRef = *const c_void;
type Boolean = u8;
type CFIndex = c_long;
type CFTypeId = usize;
type AXError = i32;
type Id = *mut c_void;
type Sel = *mut c_void;

const AX_SUCCESS: AXError = 0;
const UTF8_ENCODING: u32 = 0x0800_0100;
const AX_VALUE_CG_RECT: u32 = 3;
const AX_VALUE_CF_RANGE: u32 = 4;
const CF_NUMBER_CF_INDEX_TYPE: i32 = 14;
const MAX_ELEMENT_ANCESTORS: usize = 32;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CGPoint {
    x: c_double,
    y: c_double,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CGSize {
    width: c_double,
    height: c_double,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CFRange {
    location: CFIndex,
    length: CFIndex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidateFailure {
    NoSelection,
    Protected,
    Excluded,
}

struct ResolvedSelection {
    text: String,
    source_application_id: Option<String>,
    bounds_physical_px: Vec<PhysicalRect>,
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> Boolean;
    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> Boolean;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXUIElementCopyParameterizedAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        parameter: CFTypeRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXUIElementCopyElementAtPosition(
        application: AXUIElementRef,
        x: c_float,
        y: c_float,
        element: *mut AXUIElementRef,
    ) -> AXError;
    fn AXUIElementGetPid(element: AXUIElementRef, pid: *mut c_int) -> AXError;
    fn AXUIElementCreateApplication(pid: c_int) -> AXUIElementRef;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> AXError;
    fn AXValueGetType(value: AXValueRef) -> u32;
    fn AXValueGetValue(value: AXValueRef, value_type: u32, output: *mut c_void) -> Boolean;
    fn AXValueCreate(value_type: u32, value: *const c_void) -> AXValueRef;
}

type CGDirectDisplayId = u32;
type CGDisplayModeRef = *const c_void;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreate(source: CFTypeRef) -> CFTypeRef;
    fn CGEventGetLocation(event: CFTypeRef) -> CGPoint;
    fn CGGetActiveDisplayList(
        max_displays: u32,
        displays: *mut CGDirectDisplayId,
        count: *mut u32,
    ) -> i32;
    fn CGDisplayBounds(display: CGDirectDisplayId) -> CGRect;
    fn CGDisplayCopyDisplayMode(display: CGDirectDisplayId) -> CGDisplayModeRef;
    fn CGDisplayModeGetPixelWidth(mode: CGDisplayModeRef) -> usize;
    fn CGDisplayModeGetWidth(mode: CGDisplayModeRef) -> usize;
    fn CGDisplayModeRelease(mode: CGDisplayModeRef);
}

/// Upper bound on displays queried in one call; far above any real desktop.
const MAX_ACTIVE_DISPLAYS: u32 = 16;

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanTrue: CFTypeRef;
    fn CFRelease(value: CFTypeRef);
    fn CFGetTypeID(value: CFTypeRef) -> CFTypeId;
    fn CFStringGetTypeID() -> CFTypeId;
    fn CFBooleanGetTypeID() -> CFTypeId;
    fn CFBooleanGetValue(value: CFTypeRef) -> Boolean;
    fn CFNumberCreate(allocator: CFTypeRef, number_type: i32, value: *const c_void) -> CFTypeRef;
    fn CFNumberGetValue(number: CFTypeRef, number_type: i32, value: *mut c_void) -> Boolean;
    fn CFStringCreateWithCString(
        allocator: CFTypeRef,
        text: *const c_char,
        encoding: u32,
    ) -> CFStringRef;
    fn CFStringGetLength(value: CFStringRef) -> CFIndex;
    fn CFStringGetMaximumSizeForEncoding(length: CFIndex, encoding: u32) -> CFIndex;
    fn CFStringGetCString(
        value: CFStringRef,
        buffer: *mut c_char,
        capacity: CFIndex,
        encoding: u32,
    ) -> Boolean;
}

/// Accessibility authorization state used by settings UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessibilityPermission {
    Granted,
    Denied,
}

/// A display transform from AX global points into the application's physical
/// pixel topology. Convergence code should build these from the same monitor
/// inventory passed to placement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplayTransform {
    pub logical_bounds: PhysicalRect,
    pub physical_origin_x: f64,
    pub physical_origin_y: f64,
    pub scale_factor: f64,
}

/// Production macOS selection resolver.
///
/// Calls are synchronous native AX queries. Invoke the async trait method from
/// a blocking worker, never the UI thread.
/// Where the adapter gets its display topology from.
#[derive(Clone)]
enum DisplaySource {
    /// Read at every resolution, so an attached, detached or rescaled display
    /// takes effect immediately.
    Live,
    /// A fixed topology, used by tests and manual fixtures.
    Fixed(Arc<Vec<DisplayTransform>>),
}

impl DisplaySource {
    fn transforms(&self) -> Cow<'_, [DisplayTransform]> {
        match self {
            Self::Live => Cow::Owned(active_display_transforms()),
            Self::Fixed(fixed) => Cow::Borrowed(fixed.as_slice()),
        }
    }
}

#[derive(Clone)]
pub struct MacSelectionAdapter {
    next_id: Arc<AtomicU64>,
    displays: DisplaySource,
}

impl MacSelectionAdapter {
    /// Production adapter, which follows the live display topology.
    pub fn with_live_displays() -> Self {
        Self {
            next_id: Arc::new(AtomicU64::new(1)),
            displays: DisplaySource::Live,
        }
    }

    pub fn new(displays: Vec<DisplayTransform>) -> Self {
        Self {
            next_id: Arc::new(AtomicU64::new(1)),
            displays: DisplaySource::Fixed(Arc::new(displays)),
        }
    }

    pub fn permission_status() -> AccessibilityPermission {
        // SAFETY: AXIsProcessTrusted has no parameters and returns a Boolean.
        if unsafe { AXIsProcessTrusted() } != 0 {
            AccessibilityPermission::Granted
        } else {
            AccessibilityPermission::Denied
        }
    }

    /// Requests the standard Accessibility consent prompt. This does not imply
    /// access was granted; callers must query `permission_status` afterward.
    pub fn request_permission_prompt() -> AccessibilityPermission {
        let prompt_key = match CfString::new("AXTrustedCheckOptionPrompt") {
            Some(value) => value,
            None => return AccessibilityPermission::Denied,
        };
        let keys = [prompt_key.as_raw()];
        let values = [unsafe { kCFBooleanTrue }];
        let options = create_dictionary(&keys, &values);
        let trusted = options
            .as_ref()
            .is_some_and(|options| unsafe { AXIsProcessTrustedWithOptions(options.as_raw()) } != 0);
        if trusted {
            AccessibilityPermission::Granted
        } else {
            AccessibilityPermission::Denied
        }
    }

    fn resolve(&self, policy: &SelectionPolicy) -> Result<SelectionSnapshot, AppError> {
        if Self::permission_status() != AccessibilityPermission::Granted {
            return Err(adapter_error(
                AppErrorCode::PermissionDenied,
                "Accessibility permission is required",
            ));
        }

        // SAFETY: create rule returns an owned AXUIElementRef.
        let system = unsafe { OwnedCf::from_create(AXUIElementCreateSystemWide()) }
            .ok_or_else(internal_error)?;
        let displays = self.displays.transforms();
        let focused_selection = focused_element(system.as_raw())
            .map_err(|_| CandidateFailure::NoSelection)
            .and_then(|element| selection_from_lineage(element, policy, &displays));
        let resolved = prefer_selection_candidate(focused_selection, || {
            let element =
                element_at_pointer(system.as_raw()).map_err(|_| CandidateFailure::NoSelection)?;
            selection_from_lineage(element, policy, &displays)
        })
        .map_err(|failure| match failure {
            CandidateFailure::NoSelection => no_selection_error(),
            CandidateFailure::Protected | CandidateFailure::Excluded => unsupported_error(),
        })?;
        let anchor_physical_px = *resolved
            .bounds_physical_px
            .last()
            .ok_or_else(no_selection_error)?;

        Ok(SelectionSnapshot {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            text: resolved.text,
            source_application_id: resolved.source_application_id,
            bounds_physical_px: resolved.bounds_physical_px,
            anchor_physical_px,
            captured_at_epoch_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| internal_error())?
                .as_millis()
                .try_into()
                .map_err(|_| internal_error())?,
        })
    }

    /// Best-effort wake of the surface under the pointer. A surface that does
    /// not implement these attributes is simply left as it was.
    fn wake_source(&self) {
        if Self::permission_status() != AccessibilityPermission::Granted {
            return;
        }
        // SAFETY: create rule returns an owned AXUIElementRef.
        let Some(system) = (unsafe { OwnedCf::from_create(AXUIElementCreateSystemWide()) }) else {
            return;
        };
        let Ok(element) = element_at_pointer(system.as_raw()) else {
            return;
        };
        let mut pid: c_int = 0;
        // SAFETY: the element is live and the pid is written only on success.
        if unsafe { AXUIElementGetPid(element.as_raw(), &mut pid) } == 0 {
            enable_chromium_accessibility(pid);
        }
        // Reading a selection attribute is what makes a surface that builds its
        // tree lazily start building it.
        let _ = copy_attribute(element.as_raw(), "AXSelectedText");
    }
}

fn enable_chromium_accessibility(pid: c_int) {
    // SAFETY: create rule returns an owned AXUIElementRef for the process.
    let Some(application) = (unsafe { OwnedCf::from_create(AXUIElementCreateApplication(pid)) })
    else {
        return;
    };
    let Some(attribute) = CfString::new(CHROMIUM_ACCESSIBILITY_ATTRIBUTE) else {
        return;
    };
    // SAFETY: both references are live and the value is a constant CFBoolean.
    let _ = unsafe {
        AXUIElementSetAttributeValue(application.as_raw(), attribute.as_raw(), kCFBooleanTrue)
    };
}

fn selection_from_lineage(
    mut element: OwnedCf,
    policy: &SelectionPolicy,
    displays: &[DisplayTransform],
) -> Result<ResolvedSelection, CandidateFailure> {
    for _ in 0..MAX_ELEMENT_ANCESTORS {
        if is_protected(element.as_raw()) {
            return Err(CandidateFailure::Protected);
        }
        let source_application_id = application_id(element.as_raw());
        if policy
            .excluded_application_id
            .as_ref()
            .is_some_and(|excluded| source_application_id.as_ref() == Some(excluded))
        {
            return Err(CandidateFailure::Excluded);
        }
        if let Some((text, bounds_physical_px)) =
            selection_on_element(element.as_raw(), policy.max_code_points, displays)
        {
            return Ok(ResolvedSelection {
                text,
                source_application_id,
                bounds_physical_px,
            });
        }
        element = copy_attribute(element.as_raw(), "AXParent")
            .map_err(|_| CandidateFailure::NoSelection)?;
    }
    Err(CandidateFailure::NoSelection)
}

fn selection_on_element(
    element: AXUIElementRef,
    max_code_points: usize,
    displays: &[DisplayTransform],
) -> Option<(String, Vec<PhysicalRect>)> {
    let selected_text = copy_attribute(element, "AXSelectedText")
        .ok()
        .and_then(|value| cf_string_to_string(value.as_raw()));

    first_available_selection(
        || range_selection(element, selected_text.clone(), max_code_points, displays),
        || marker_selection(element, selected_text.clone(), max_code_points, displays),
    )
}

/// Accepts a strategy only when it produced eligible text *and* geometry. A
/// strategy that resolves text but no rectangles is incomplete, not a selection.
fn accept_selection(
    text: Option<String>,
    rectangles: Vec<PhysicalRect>,
    max_code_points: usize,
) -> Option<(String, Vec<PhysicalRect>)> {
    let text = text?;
    if !eligible_text(&text, max_code_points) || rectangles.is_empty() {
        return None;
    }
    Some((text, rectangles))
}

/// Tries geometry strategies in order. An incomplete strategy must fall through
/// rather than abort resolution for the element.
fn first_available_selection(
    primary: impl FnOnce() -> Option<(String, Vec<PhysicalRect>)>,
    fallback: impl FnOnce() -> Option<(String, Vec<PhysicalRect>)>,
) -> Option<(String, Vec<PhysicalRect>)> {
    primary().or_else(fallback)
}

/// Line-range geometry, used by native text controls.
fn range_selection(
    element: AXUIElementRef,
    selected_text: Option<String>,
    max_code_points: usize,
    displays: &[DisplayTransform],
) -> Option<(String, Vec<PhysicalRect>)> {
    let selected_range = copy_attribute(element, "AXSelectedTextRange")
        .ok()
        .and_then(|value| ax_range(value.as_raw()))?;
    let rectangles =
        selected_line_rectangles(element, selected_range, displays).unwrap_or_default();
    accept_selection(selected_text, rectangles, max_code_points)
}

/// Text-marker geometry, used by web and other rich document surfaces that
/// implement no `AXBoundsForRange`.
fn marker_selection(
    element: AXUIElementRef,
    selected_text: Option<String>,
    max_code_points: usize,
    displays: &[DisplayTransform],
) -> Option<(String, Vec<PhysicalRect>)> {
    let marker_range = copy_attribute(element, "AXSelectedTextMarkerRange").ok()?;
    let text = selected_text
        .filter(|text| eligible_text(text, max_code_points))
        .or_else(|| {
            copy_parameterized_attribute(
                element,
                "AXStringForTextMarkerRange",
                marker_range.as_raw(),
            )
            .ok()
            .and_then(|value| cf_string_to_string(value.as_raw()))
        });
    let rectangles =
        copy_parameterized_attribute(element, "AXBoundsForTextMarkerRange", marker_range.as_raw())
            .ok()
            .and_then(|value| ax_rect(value.as_raw()))
            .map(|bounds| normalize_rects(bounds, displays))
            .unwrap_or_default();
    accept_selection(text, rectangles, max_code_points)
}

fn eligible_text(text: &str, max_code_points: usize) -> bool {
    !text.trim().is_empty() && text.chars().count() <= max_code_points
}

#[async_trait]
impl SelectionAdapter for MacSelectionAdapter {
    async fn resolve_selection(
        &self,
        policy: &SelectionPolicy,
    ) -> Result<SelectionSnapshot, AppError> {
        let adapter = self.clone();
        let policy = policy.clone();
        tokio::task::spawn_blocking(move || adapter.resolve(&policy))
            .await
            .map_err(|_| internal_error())?
    }

    async fn prepare_source(&self) {
        let adapter = self.clone();
        let _ = tokio::task::spawn_blocking(move || adapter.wake_source()).await;
    }
}

/// Chromium exposes web content to the accessibility tree only after a client
/// sets this attribute on the application element. Other applications reject it
/// harmlessly.
const CHROMIUM_ACCESSIBILITY_ATTRIBUTE: &str = "AXManualAccessibility";

pub fn normalize_rect(
    logical: PhysicalRect,
    displays: &[DisplayTransform],
) -> Option<PhysicalRect> {
    if !valid_rect(logical) {
        return None;
    }
    let center_x = logical.x + logical.width / 2.0;
    let center_y = logical.y + logical.height / 2.0;
    let display = displays.iter().find(|display| {
        valid_rect(display.logical_bounds)
            && display.scale_factor.is_finite()
            && display.scale_factor > 0.0
            && center_x >= display.logical_bounds.x
            && center_x <= display.logical_bounds.x + display.logical_bounds.width
            && center_y >= display.logical_bounds.y
            && center_y <= display.logical_bounds.y + display.logical_bounds.height
    })?;
    Some(PhysicalRect {
        x: display.physical_origin_x
            + (logical.x - display.logical_bounds.x) * display.scale_factor,
        y: display.physical_origin_y
            + (logical.y - display.logical_bounds.y) * display.scale_factor,
        width: logical.width * display.scale_factor,
        height: logical.height * display.scale_factor,
    })
}

/// Reads the current display topology.
///
/// `CGDisplayBounds` is expressed in the same top-left logical space the
/// accessibility API reports geometry in, and the window layer positions
/// overlays at `logical * scale`, so both agree without conversion.
///
/// This is read per resolution rather than cached once: displays are attached,
/// detached and rescaled while the application runs, and a topology that has
/// gone stale silently discards every selection made on a display it does not
/// know about.
pub fn active_display_transforms() -> Vec<DisplayTransform> {
    let mut ids = [0 as CGDirectDisplayId; MAX_ACTIVE_DISPLAYS as usize];
    let mut count: u32 = 0;
    // SAFETY: the buffer holds MAX_ACTIVE_DISPLAYS entries and CoreGraphics
    // writes the number it filled into count.
    let result =
        unsafe { CGGetActiveDisplayList(MAX_ACTIVE_DISPLAYS, ids.as_mut_ptr(), &mut count) };
    if result != 0 {
        return Vec::new();
    }
    ids.iter()
        .take(count as usize)
        .filter_map(|id| display_transform(*id))
        .collect()
}

fn display_transform(id: CGDirectDisplayId) -> Option<DisplayTransform> {
    let scale_factor = display_scale_factor(id)?;
    // SAFETY: CGDisplayBounds accepts any identifier and returns an empty
    // rectangle for one that is no longer active.
    let bounds = unsafe { CGDisplayBounds(id) };
    let transform = DisplayTransform {
        logical_bounds: PhysicalRect {
            x: bounds.origin.x,
            y: bounds.origin.y,
            width: bounds.size.width,
            height: bounds.size.height,
        },
        physical_origin_x: bounds.origin.x * scale_factor,
        physical_origin_y: bounds.origin.y * scale_factor,
        scale_factor,
    };
    valid_display(&transform).then_some(transform)
}

fn display_scale_factor(id: CGDirectDisplayId) -> Option<f64> {
    // SAFETY: the copy rule returns an owned mode, released below.
    let mode = unsafe { CGDisplayCopyDisplayMode(id) };
    if mode.is_null() {
        return None;
    }
    // SAFETY: the mode stays live until it is released.
    let (pixel_width, width) = unsafe {
        (
            CGDisplayModeGetPixelWidth(mode),
            CGDisplayModeGetWidth(mode),
        )
    };
    // SAFETY: ownership returns to CoreGraphics here and the mode is not used again.
    unsafe { CGDisplayModeRelease(mode) };
    if width == 0 {
        return None;
    }
    let scale = pixel_width as f64 / width as f64;
    (scale.is_finite() && scale > 0.0).then_some(scale)
}

/// Splits a logical AX rectangle at display boundaries before scaling each
/// piece into the physical topology. This avoids applying one monitor's scale
/// to geometry that crosses onto another monitor.
pub fn normalize_rects(logical: PhysicalRect, displays: &[DisplayTransform]) -> Vec<PhysicalRect> {
    if !valid_rect(logical) {
        return Vec::new();
    }
    let mut pieces = displays
        .iter()
        .filter_map(|display| {
            if !valid_display(display) {
                return None;
            }
            let intersection = intersect_rect(logical, display.logical_bounds)?;
            Some((
                intersection.x,
                PhysicalRect {
                    x: display.physical_origin_x
                        + (intersection.x - display.logical_bounds.x) * display.scale_factor,
                    y: display.physical_origin_y
                        + (intersection.y - display.logical_bounds.y) * display.scale_factor,
                    width: intersection.width * display.scale_factor,
                    height: intersection.height * display.scale_factor,
                },
            ))
        })
        .collect::<Vec<_>>();
    pieces.sort_by(|left, right| left.0.total_cmp(&right.0));
    pieces.into_iter().map(|(_, rect)| rect).collect()
}

fn selected_line_rectangles(
    element: AXUIElementRef,
    selected: CFRange,
    displays: &[DisplayTransform],
) -> Option<Vec<PhysicalRect>> {
    if selected.location < 0 || selected.length <= 0 {
        return None;
    }
    let final_index = selected
        .location
        .checked_add(selected.length.checked_sub(1)?)?;
    let first_line = line_for_index(element, selected.location)?;
    let final_line = line_for_index(element, final_index)?;
    if first_line < 0 || final_line < first_line || final_line - first_line > 4_096 {
        return None;
    }

    let mut rectangles = Vec::new();
    for line in first_line..=final_line {
        let line_range = range_for_line(element, line)?;
        let intersection = intersect_range(selected, line_range)?;
        let range_value = create_ax_range(intersection)?;
        let bounds =
            copy_parameterized_attribute(element, "AXBoundsForRange", range_value.as_raw()).ok()?;
        let logical = ax_rect(bounds.as_raw())?;
        let pieces = normalize_rects(logical, displays);
        if pieces.is_empty() {
            return None;
        }
        rectangles.extend(pieces);
    }
    (!rectangles.is_empty()).then_some(rectangles)
}

fn line_for_index(element: AXUIElementRef, index: CFIndex) -> Option<CFIndex> {
    let parameter = create_cf_number(index)?;
    let line = copy_parameterized_attribute(element, "AXLineForIndex", parameter.as_raw()).ok()?;
    cf_number_to_index(line.as_raw())
}

fn range_for_line(element: AXUIElementRef, line: CFIndex) -> Option<CFRange> {
    let parameter = create_cf_number(line)?;
    let range = copy_parameterized_attribute(element, "AXRangeForLine", parameter.as_raw()).ok()?;
    ax_range(range.as_raw())
}

fn create_cf_number(value: CFIndex) -> Option<OwnedCf> {
    // SAFETY: the pointer addresses a CFIndex matching CF_NUMBER_CF_INDEX_TYPE.
    let raw = unsafe {
        CFNumberCreate(
            ptr::null(),
            CF_NUMBER_CF_INDEX_TYPE,
            (&raw const value).cast::<c_void>(),
        )
    };
    // SAFETY: CFNumberCreate follows the create rule.
    unsafe { OwnedCf::from_create(raw) }
}

fn cf_number_to_index(value: CFTypeRef) -> Option<CFIndex> {
    let mut output: CFIndex = 0;
    // SAFETY: output matches the requested CFIndex representation.
    if unsafe {
        CFNumberGetValue(
            value,
            CF_NUMBER_CF_INDEX_TYPE,
            (&mut output as *mut CFIndex).cast::<c_void>(),
        )
    } == 0
    {
        None
    } else {
        Some(output)
    }
}

fn create_ax_range(value: CFRange) -> Option<OwnedCf> {
    // SAFETY: the pointer addresses a CFRange matching AX_VALUE_CF_RANGE.
    let raw = unsafe { AXValueCreate(AX_VALUE_CF_RANGE, (&raw const value).cast::<c_void>()) };
    // SAFETY: AXValueCreate follows the create rule.
    unsafe { OwnedCf::from_create(raw) }
}

fn ax_range(value: CFTypeRef) -> Option<CFRange> {
    if unsafe { AXValueGetType(value) } != AX_VALUE_CF_RANGE {
        return None;
    }
    let mut range = CFRange {
        location: 0,
        length: 0,
    };
    // SAFETY: the checked AX type matches the CFRange output.
    if unsafe {
        AXValueGetValue(
            value,
            AX_VALUE_CF_RANGE,
            (&mut range as *mut CFRange).cast::<c_void>(),
        )
    } == 0
    {
        None
    } else {
        Some(range)
    }
}

fn intersect_range(left: CFRange, right: CFRange) -> Option<CFRange> {
    let start = left.location.max(right.location);
    let end = left
        .location
        .checked_add(left.length)?
        .min(right.location.checked_add(right.length)?);
    (end > start).then_some(CFRange {
        location: start,
        length: end - start,
    })
}

fn valid_display(display: &DisplayTransform) -> bool {
    valid_rect(display.logical_bounds)
        && display.physical_origin_x.is_finite()
        && display.physical_origin_y.is_finite()
        && display.scale_factor.is_finite()
        && display.scale_factor > 0.0
}

fn intersect_rect(left: PhysicalRect, right: PhysicalRect) -> Option<PhysicalRect> {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = (left.x + left.width).min(right.x + right.width);
    let bottom_edge = (left.y + left.height).min(right.y + right.height);
    (right_edge > x && bottom_edge > y).then_some(PhysicalRect {
        x,
        y,
        width: right_edge - x,
        height: bottom_edge - y,
    })
}

fn valid_rect(rect: PhysicalRect) -> bool {
    rect.x.is_finite()
        && rect.y.is_finite()
        && rect.width.is_finite()
        && rect.height.is_finite()
        && rect.width > 0.0
        && rect.height > 0.0
}

fn is_protected(element: AXUIElementRef) -> bool {
    let secure_subrole = copy_attribute(element, "AXSubrole")
        .ok()
        .and_then(|value| cf_string_to_string(value.as_raw()))
        .is_some_and(|subrole| subrole == "AXSecureTextField");
    let protected_content = copy_attribute(element, "AXProtectedContent")
        .ok()
        .is_some_and(|value| cf_boolean(value.as_raw()).unwrap_or(false));
    secure_subrole || protected_content
}

fn application_id(element: AXUIElementRef) -> Option<String> {
    let mut pid = 0;
    // SAFETY: `pid` is valid writable storage and `element` is live for this call.
    if unsafe { AXUIElementGetPid(element, &mut pid) } != AX_SUCCESS {
        return None;
    }
    bundle_identifier(pid).or_else(|| Some(format!("pid:{pid}")))
}

fn bundle_identifier(pid: c_int) -> Option<String> {
    let class_name = CString::new("NSRunningApplication").ok()?;
    let running_selector = CString::new("runningApplicationWithProcessIdentifier:").ok()?;
    let identifier_selector = CString::new("bundleIdentifier").ok()?;
    // SAFETY: names are valid NUL-terminated Objective-C runtime strings.
    let class = unsafe { objc_getClass(class_name.as_ptr()) };
    let running_selector = unsafe { sel_registerName(running_selector.as_ptr()) };
    let identifier_selector = unsafe { sel_registerName(identifier_selector.as_ptr()) };
    if class.is_null() || running_selector.is_null() || identifier_selector.is_null() {
        return None;
    }

    // SAFETY: the pool is balanced before returning and contains all
    // autoreleased AppKit objects created by these lookups.
    let pool = unsafe { objc_autoreleasePoolPush() };
    let result = (|| {
        type RunningApplication = unsafe extern "C" fn(Id, Sel, c_int) -> Id;
        type BundleIdentifier = unsafe extern "C" fn(Id, Sel) -> Id;
        // SAFETY: objc_msgSend is cast to each selector's exact documented ABI.
        let running_application: RunningApplication =
            unsafe { mem::transmute(objc_msgSend as *const ()) };
        let bundle_identifier: BundleIdentifier =
            unsafe { mem::transmute(objc_msgSend as *const ()) };
        let application = unsafe { running_application(class, running_selector, pid) };
        if application.is_null() {
            return None;
        }
        let identifier = unsafe { bundle_identifier(application, identifier_selector) };
        cf_string_to_string(identifier.cast_const())
    })();
    unsafe { objc_autoreleasePoolPop(pool) };
    result
}

fn ax_rect(value: CFTypeRef) -> Option<PhysicalRect> {
    // SAFETY: type is checked before writing a CGRect-sized output.
    if unsafe { AXValueGetType(value) } != AX_VALUE_CG_RECT {
        return None;
    }
    let mut rect = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize {
            width: 0.0,
            height: 0.0,
        },
    };
    // SAFETY: the requested AX type matches `rect`.
    if unsafe {
        AXValueGetValue(
            value,
            AX_VALUE_CG_RECT,
            (&mut rect as *mut CGRect).cast::<c_void>(),
        )
    } == 0
    {
        return None;
    }
    Some(PhysicalRect {
        x: rect.origin.x,
        y: rect.origin.y,
        width: rect.size.width,
        height: rect.size.height,
    })
}

fn copy_attribute(element: AXUIElementRef, name: &str) -> Result<OwnedCf, AXError> {
    let name = CfString::new(name).ok_or(-1)?;
    let mut value = ptr::null();
    // SAFETY: output is initialized by AX on success; both inputs remain live.
    let result = unsafe { AXUIElementCopyAttributeValue(element, name.as_raw(), &mut value) };
    if result == AX_SUCCESS {
        // SAFETY: Copy follows the create rule and returns ownership.
        unsafe { OwnedCf::from_create(value) }.ok_or(-1)
    } else {
        Err(result)
    }
}

fn focused_element(system: AXUIElementRef) -> Result<OwnedCf, AXError> {
    prefer_primary_or_fallback(copy_attribute(system, "AXFocusedUIElement"), || {
        let application = copy_attribute(system, "AXFocusedApplication")?;
        copy_attribute(application.as_raw(), "AXFocusedUIElement")
    })
}

fn element_at_pointer(system: AXUIElementRef) -> Result<OwnedCf, AXError> {
    // SAFETY: a null source asks CoreGraphics to create a generic event whose
    // location is the current global pointer position.
    let event = unsafe { OwnedCf::from_create(CGEventCreate(ptr::null())) }.ok_or(-1)?;
    // SAFETY: `event` is a live CGEvent.
    let location = unsafe { CGEventGetLocation(event.as_raw()) };
    if !location.x.is_finite() || !location.y.is_finite() {
        return Err(-1);
    }
    let mut element = ptr::null();
    // SAFETY: output is initialized by AX on success; the system element is live.
    let result = unsafe {
        AXUIElementCopyElementAtPosition(
            system,
            location.x as c_float,
            location.y as c_float,
            &mut element,
        )
    };
    if result == AX_SUCCESS {
        // SAFETY: Copy follows the create rule and returns ownership.
        unsafe { OwnedCf::from_create(element) }.ok_or(-1)
    } else {
        Err(result)
    }
}

fn prefer_selection_candidate<T>(
    primary: Result<T, CandidateFailure>,
    fallback: impl FnOnce() -> Result<T, CandidateFailure>,
) -> Result<T, CandidateFailure> {
    match primary {
        Err(CandidateFailure::NoSelection) => fallback(),
        result => result,
    }
}

fn prefer_primary_or_fallback<T, E>(
    primary: Result<T, E>,
    fallback: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    primary.or_else(|_| fallback())
}

fn copy_parameterized_attribute(
    element: AXUIElementRef,
    name: &str,
    parameter: CFTypeRef,
) -> Result<OwnedCf, AXError> {
    let name = CfString::new(name).ok_or(-1)?;
    let mut value = ptr::null();
    // SAFETY: output is initialized by AX on success and the range remains live.
    let result = unsafe {
        AXUIElementCopyParameterizedAttributeValue(element, name.as_raw(), parameter, &mut value)
    };
    if result == AX_SUCCESS {
        // SAFETY: Copy follows the create rule and returns ownership.
        unsafe { OwnedCf::from_create(value) }.ok_or(-1)
    } else {
        Err(result)
    }
}

fn cf_string_to_string(value: CFTypeRef) -> Option<String> {
    // SAFETY: type IDs may be queried for any non-null CF object.
    if value.is_null() || unsafe { CFGetTypeID(value) } != unsafe { CFStringGetTypeID() } {
        return None;
    }
    // SAFETY: the verified value is a CFString.
    let length = unsafe { CFStringGetLength(value) };
    // SAFETY: capacity query accepts non-negative CFString lengths.
    let capacity = unsafe { CFStringGetMaximumSizeForEncoding(length, UTF8_ENCODING) } + 1;
    let mut bytes = vec![0_u8; usize::try_from(capacity).ok()?];
    // SAFETY: buffer has exactly `capacity` bytes and the value is a CFString.
    if unsafe {
        CFStringGetCString(
            value,
            bytes.as_mut_ptr().cast::<c_char>(),
            capacity,
            UTF8_ENCODING,
        )
    } == 0
    {
        return None;
    }
    // SAFETY: successful CFStringGetCString always writes a trailing NUL.
    Some(
        unsafe { CStr::from_ptr(bytes.as_ptr().cast::<c_char>()) }
            .to_string_lossy()
            .into_owned(),
    )
}

fn cf_boolean(value: CFTypeRef) -> Option<bool> {
    // SAFETY: type IDs may be queried for any non-null CF object.
    if value.is_null() || unsafe { CFGetTypeID(value) } != unsafe { CFBooleanGetTypeID() } {
        return None;
    }
    // SAFETY: the type check established a CFBoolean.
    Some(unsafe { CFBooleanGetValue(value) } != 0)
}

struct OwnedCf(NonNull<c_void>);

impl OwnedCf {
    /// `value` must be +1 retained under a Core Foundation create/copy rule.
    unsafe fn from_create(value: CFTypeRef) -> Option<Self> {
        NonNull::new(value.cast_mut()).map(Self)
    }

    fn as_raw(&self) -> CFTypeRef {
        self.0.as_ptr()
    }
}

impl Drop for OwnedCf {
    fn drop(&mut self) {
        // SAFETY: OwnedCf is created only from +1 create/copy results and drops once.
        unsafe { CFRelease(self.as_raw()) };
    }
}

struct CfString(OwnedCf);

impl CfString {
    fn new(value: &str) -> Option<Self> {
        let value = CString::new(value).ok()?;
        // SAFETY: UTF-8 C string remains valid for the duration of the call.
        let raw = unsafe { CFStringCreateWithCString(ptr::null(), value.as_ptr(), UTF8_ENCODING) };
        // SAFETY: CFStringCreateWithCString returns a +1 object.
        unsafe { OwnedCf::from_create(raw) }.map(Self)
    }

    fn as_raw(&self) -> CFStringRef {
        self.0.as_raw()
    }
}

fn adapter_error(code: AppErrorCode, message: &'static str) -> AppError {
    AppError::new(code, message, false)
}

fn unsupported_error() -> AppError {
    adapter_error(
        AppErrorCode::UnsupportedControl,
        "Focused control does not expose an eligible selection",
    )
}

fn no_selection_error() -> AppError {
    adapter_error(
        AppErrorCode::NoSelection,
        "No eligible selection is available",
    )
}

fn internal_error() -> AppError {
    adapter_error(
        AppErrorCode::Internal,
        "The macOS selection adapter could not complete",
    )
}

type DictionaryCreateFn = unsafe extern "C" fn(
    CFTypeRef,
    *const CFTypeRef,
    *const CFTypeRef,
    CFIndex,
    CFTypeRef,
    CFTypeRef,
) -> CFTypeRef;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    #[link_name = "CFDictionaryCreate"]
    fn cf_dictionary_create(
        allocator: CFTypeRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        count: CFIndex,
        key_callbacks: CFTypeRef,
        value_callbacks: CFTypeRef,
    ) -> CFTypeRef;
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
}

fn create_dictionary(keys: &[CFTypeRef], values: &[CFTypeRef]) -> Option<OwnedCf> {
    if keys.len() != values.len() {
        return None;
    }
    let create: DictionaryCreateFn = cf_dictionary_create;
    // SAFETY: slices have matching lengths; callback structure addresses are static.
    let raw = unsafe {
        create(
            ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            keys.len().try_into().ok()?,
            (&raw const kCFTypeDictionaryKeyCallBacks).cast::<c_void>(),
            (&raw const kCFTypeDictionaryValueCallBacks).cast::<c_void>(),
        )
    };
    // SAFETY: CFDictionaryCreate returns a +1 object.
    unsafe { OwnedCf::from_create(raw) }
}

#[cfg(test)]
mod tests {
    use super::{
        accept_selection, first_available_selection, normalize_rect, normalize_rects,
        prefer_primary_or_fallback, prefer_selection_candidate, AccessibilityPermission,
        CandidateFailure, DisplayTransform, MacSelectionAdapter,
    };
    use crate::{
        contracts::{AppErrorCode, PhysicalRect},
        platform::{SelectionAdapter, SelectionPolicy},
    };

    #[test]
    fn normalizes_ax_points_into_selected_display_pixels() {
        let result = normalize_rect(
            PhysicalRect {
                x: 1500.0,
                y: 200.0,
                width: 100.0,
                height: 20.0,
            },
            &[DisplayTransform {
                logical_bounds: PhysicalRect {
                    x: 1440.0,
                    y: 0.0,
                    width: 1512.0,
                    height: 945.0,
                },
                physical_origin_x: 1920.0,
                physical_origin_y: 0.0,
                scale_factor: 2.0,
            }],
        );

        assert_eq!(
            result,
            Some(PhysicalRect {
                x: 2040.0,
                y: 400.0,
                width: 200.0,
                height: 40.0,
            })
        );
    }

    #[test]
    fn rejects_off_screen_or_non_finite_geometry() {
        let display = DisplayTransform {
            logical_bounds: PhysicalRect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
            physical_origin_x: 0.0,
            physical_origin_y: 0.0,
            scale_factor: 2.0,
        };
        assert_eq!(
            normalize_rect(
                PhysicalRect {
                    x: 150.0,
                    y: 10.0,
                    width: 10.0,
                    height: 10.0,
                },
                &[display],
            ),
            None
        );
        assert_eq!(
            normalize_rect(
                PhysicalRect {
                    x: f64::NAN,
                    y: 10.0,
                    width: 10.0,
                    height: 10.0,
                },
                &[display],
            ),
            None
        );
    }

    #[test]
    fn splits_and_normalizes_a_line_spanning_mixed_scale_displays() {
        let displays = [
            DisplayTransform {
                logical_bounds: PhysicalRect {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 100.0,
                },
                physical_origin_x: 0.0,
                physical_origin_y: 0.0,
                scale_factor: 1.0,
            },
            DisplayTransform {
                logical_bounds: PhysicalRect {
                    x: 100.0,
                    y: 0.0,
                    width: 100.0,
                    height: 100.0,
                },
                physical_origin_x: 100.0,
                physical_origin_y: 0.0,
                scale_factor: 2.0,
            },
        ];

        assert_eq!(
            normalize_rects(
                PhysicalRect {
                    x: 90.0,
                    y: 10.0,
                    width: 20.0,
                    height: 10.0,
                },
                &displays,
            ),
            vec![
                PhysicalRect {
                    x: 90.0,
                    y: 10.0,
                    width: 10.0,
                    height: 10.0,
                },
                PhysicalRect {
                    x: 100.0,
                    y: 20.0,
                    width: 20.0,
                    height: 20.0,
                },
            ]
        );
    }

    #[test]
    fn permission_probe_is_safe_without_prompting() {
        assert!(matches!(
            MacSelectionAdapter::permission_status(),
            AccessibilityPermission::Granted | AccessibilityPermission::Denied
        ));
    }

    #[test]
    fn focused_element_lookup_falls_back_to_the_focused_application() {
        assert_eq!(
            prefer_primary_or_fallback::<_, &str>(Err("no global value"), || Ok("text field")),
            Ok("text field")
        );
        assert_eq!(
            prefer_primary_or_fallback::<_, &str>(Ok("global field"), || {
                panic!("fallback must not run")
            }),
            Ok("global field")
        );
    }

    #[test]
    fn pointer_candidate_is_used_only_when_focus_has_no_selection() {
        assert_eq!(
            prefer_selection_candidate::<&str>(Err(CandidateFailure::NoSelection), || Ok(
                "document selection"
            ),),
            Ok("document selection")
        );
        assert_eq!(
            prefer_selection_candidate::<&str>(Err(CandidateFailure::Protected), || panic!(
                "protected content must not fall through"
            ),),
            Err(CandidateFailure::Protected)
        );
    }

    #[test]
    fn text_without_geometry_is_not_an_acceptable_selection() {
        let rectangle = PhysicalRect {
            x: 10.0,
            y: 20.0,
            width: 120.0,
            height: 18.0,
        };

        assert_eq!(
            accept_selection(Some("Bonjour tout le".into()), Vec::new(), 5_000),
            None
        );
        assert_eq!(
            accept_selection(Some("   ".into()), vec![rectangle], 5_000),
            None
        );
        assert_eq!(accept_selection(None, vec![rectangle], 5_000), None);
        assert_eq!(
            accept_selection(Some("Bonjour".into()), vec![rectangle], 3),
            None
        );
        assert_eq!(
            accept_selection(Some("Bonjour".into()), vec![rectangle], 5_000),
            Some(("Bonjour".into(), vec![rectangle]))
        );
    }

    /// Chromium web areas expose `AXSelectedText` and `AXSelectedTextRange` but
    /// implement no `AXBoundsForRange`, so the range strategy yields text with no
    /// geometry. The marker-range strategy must still be consulted.
    #[test]
    fn a_geometry_less_range_strategy_does_not_suppress_the_marker_strategy() {
        let rectangle = PhysicalRect {
            x: 10.0,
            y: 20.0,
            width: 120.0,
            height: 18.0,
        };

        let resolved = first_available_selection(
            || accept_selection(Some("Bonjour tout le".into()), Vec::new(), 5_000),
            || accept_selection(Some("Bonjour tout le".into()), vec![rectangle], 5_000),
        );

        assert_eq!(resolved, Some(("Bonjour tout le".into(), vec![rectangle])));
    }

    #[test]
    fn a_complete_range_strategy_short_circuits_the_marker_strategy() {
        let rectangle = PhysicalRect {
            x: 4.0,
            y: 8.0,
            width: 60.0,
            height: 16.0,
        };

        let resolved = first_available_selection(
            || accept_selection(Some("selected".into()), vec![rectangle], 5_000),
            || panic!("marker strategy must not run once the range strategy succeeds"),
        );

        assert_eq!(resolved, Some(("selected".into(), vec![rectangle])));
    }

    #[test]
    #[ignore = "manual macOS fixture: revoke Accessibility access for the test binary first"]
    fn manual_permission_denied_fixture_reports_denied_without_prompting() {
        assert_eq!(
            MacSelectionAdapter::permission_status(),
            AccessibilityPermission::Denied
        );
    }

    #[tokio::test]
    #[ignore = "manual macOS fixture: focus a secure text field before running"]
    async fn manual_secure_field_never_returns_selected_text() {
        let adapter = manual_adapter();
        let error = adapter
            .resolve_selection(&SelectionPolicy {
                max_code_points: 1_000,
                excluded_application_id: None,
            })
            .await
            .expect_err("secure selection must be suppressed");
        assert!(matches!(
            error.code,
            AppErrorCode::UnsupportedControl | AppErrorCode::NoSelection
        ));
    }

    #[tokio::test]
    #[ignore = "manual macOS fixture: select wrapped multiline text in an AX-enabled app"]
    async fn manual_focused_multiline_selection_returns_final_line_anchor() {
        let snapshot = manual_mixed_display_adapter()
            .resolve_selection(&SelectionPolicy {
                max_code_points: 10_000,
                excluded_application_id: None,
            })
            .await
            .expect("focused multiline AX selection");
        assert!(snapshot.bounds_physical_px.len() > 1);
        assert_eq!(
            snapshot.bounds_physical_px.last(),
            Some(&snapshot.anchor_physical_px)
        );
    }

    #[tokio::test]
    #[ignore = "manual macOS fixture: select text in an unfocused HTML reading surface"]
    async fn manual_pointer_surface_selection_resolves_without_focused_control() {
        let snapshot = manual_adapter()
            .resolve_selection(&SelectionPolicy {
                max_code_points: 10_000,
                excluded_application_id: None,
            })
            .await
            .expect("pointer-hosted application-surface selection");
        assert!(!snapshot.text.trim().is_empty());
        assert!(!snapshot.bounds_physical_px.is_empty());
    }

    #[tokio::test]
    #[ignore = "manual macOS fixture: select text crossing differently scaled displays"]
    async fn manual_cross_display_selection_has_finite_physical_fragments() {
        let snapshot = manual_adapter()
            .resolve_selection(&SelectionPolicy {
                max_code_points: 10_000,
                excluded_application_id: None,
            })
            .await
            .expect("cross-display AX selection");
        assert!(snapshot.bounds_physical_px.iter().all(|rect| {
            rect.x.is_finite() && rect.y.is_finite() && rect.width > 0.0 && rect.height > 0.0
        }));
    }

    /// The live topology must describe every attached display, place the main
    /// one at the origin, and agree with the window layer's `logical * scale`
    /// convention. A selection made on a display missing from this list resolves
    /// to no geometry at all, which is how an external screen stops working.
    #[test]
    fn live_topology_describes_every_attached_display() {
        let displays = super::active_display_transforms();
        if displays.is_empty() {
            // A headless build machine has no display to describe.
            return;
        }

        assert!(displays
            .iter()
            .any(|display| { display.logical_bounds.x == 0.0 && display.logical_bounds.y == 0.0 }));
        for display in &displays {
            assert!(display.scale_factor > 0.0);
            assert_eq!(
                display.physical_origin_x,
                display.logical_bounds.x * display.scale_factor
            );
            assert_eq!(
                display.physical_origin_y,
                display.logical_bounds.y * display.scale_factor
            );
        }
    }

    /// Guards the defect directly: geometry on a display the adapter does not
    /// know about produces nothing, so the topology may never be a stale cache.
    #[test]
    fn geometry_on_an_unknown_display_resolves_to_nothing() {
        let only_builtin = [DisplayTransform {
            logical_bounds: PhysicalRect {
                x: 0.0,
                y: 0.0,
                width: 1800.0,
                height: 1169.0,
            },
            physical_origin_x: 0.0,
            physical_origin_y: 0.0,
            scale_factor: 2.0,
        }];
        let on_a_second_display = PhysicalRect {
            x: -800.0,
            y: 400.0,
            width: 120.0,
            height: 20.0,
        };

        assert!(normalize_rects(on_a_second_display, &only_builtin).is_empty());
    }

    fn manual_adapter() -> MacSelectionAdapter {
        MacSelectionAdapter::new(vec![DisplayTransform {
            logical_bounds: PhysicalRect {
                x: -20_000.0,
                y: -20_000.0,
                width: 40_000.0,
                height: 40_000.0,
            },
            physical_origin_x: -20_000.0,
            physical_origin_y: -20_000.0,
            scale_factor: 1.0,
        }])
    }

    fn manual_mixed_display_adapter() -> MacSelectionAdapter {
        MacSelectionAdapter::new(vec![
            DisplayTransform {
                logical_bounds: PhysicalRect {
                    x: -20_000.0,
                    y: -20_000.0,
                    width: 20_000.0,
                    height: 40_000.0,
                },
                physical_origin_x: -20_000.0,
                physical_origin_y: -20_000.0,
                scale_factor: 1.0,
            },
            DisplayTransform {
                logical_bounds: PhysicalRect {
                    x: 0.0,
                    y: -20_000.0,
                    width: 20_000.0,
                    height: 40_000.0,
                },
                physical_origin_x: 0.0,
                physical_origin_y: -40_000.0,
                scale_factor: 2.0,
            },
        ])
    }
}
