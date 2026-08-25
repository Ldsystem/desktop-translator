//! Native secure credential entry that never places the API key in WebView state.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::contracts::{AppError, AppErrorCode};

static PROMPT_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// Prompts for an API key using the platform's native secure credential control.
#[cfg(target_os = "macos")]
pub fn prompt_secure_text(title: &str, message: &str) -> Result<Option<String>, AppError> {
    let _guard = acquire_prompt_guard()?;
    macos::prompt(title, message)
}

/// Prompts for an API key using Windows Credential UI without persistence.
#[cfg(target_os = "windows")]
pub fn prompt_secure_text(title: &str, message: &str) -> Result<Option<String>, AppError> {
    prompt_secure_text_with_parent(title, message, std::ptr::null_mut())
}

/// Owns the credential dialog with the invoking window so it cannot fall behind.
#[cfg(target_os = "windows")]
pub fn prompt_secure_text_for_window(
    window: &tauri::WebviewWindow,
    title: &str,
    message: &str,
) -> Result<Option<String>, AppError> {
    use std::sync::mpsc;

    let parent = window
        .hwnd()
        .ok()
        .map(|handle| handle.0 as isize)
        .unwrap_or(0);
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();

    let title = title.to_owned();
    let message = message.to_owned();
    let (sender, receiver) = mpsc::sync_channel(1);
    window
        .run_on_main_thread(move || {
            let parent = parent as *mut std::ffi::c_void;
            let result = prompt_secure_text_with_parent(&title, &message, parent);
            let _ = sender.send(result);
        })
        .map_err(|_| prompt_error())?;
    receiver.recv().map_err(|_| prompt_error())?
}

#[cfg(target_os = "windows")]
fn prompt_secure_text_with_parent(
    title: &str,
    message: &str,
    parent: *mut std::ffi::c_void,
) -> Result<Option<String>, AppError> {
    let _guard = acquire_prompt_guard()?;
    windows::prompt(title, message, parent)
}

fn prompt_error() -> AppError {
    AppError::new(
        AppErrorCode::Internal,
        "Secure credential entry is unavailable.",
        false,
    )
}

fn try_begin_prompt() -> bool {
    PROMPT_IN_PROGRESS
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

fn end_prompt() {
    PROMPT_IN_PROGRESS.store(false, Ordering::Release);
}

struct PromptGuard;

impl Drop for PromptGuard {
    fn drop(&mut self) {
        end_prompt();
    }
}

fn acquire_prompt_guard() -> Result<PromptGuard, AppError> {
    if try_begin_prompt() {
        Ok(PromptGuard)
    } else {
        Err(AppError::new(
            AppErrorCode::Internal,
            "A credential prompt is already open.",
            false,
        ))
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{c_char, c_void, CStr, CString};

    use super::{prompt_error, AppError};

    type Id = *mut c_void;
    type Sel = *mut c_void;

    const FIRST_BUTTON_RETURN: isize = 1000;

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_msgSend();
        fn objc_autoreleasePoolPush() -> *mut c_void;
        fn objc_autoreleasePoolPop(pool: *mut c_void);
    }

    pub fn prompt(title: &str, message: &str) -> Result<Option<String>, AppError> {
        // SAFETY: all AppKit objects remain inside one balanced autorelease pool on
        // the Tauri command's AppKit main thread; no Objective-C pointer escapes.
        unsafe {
            let pool = objc_autoreleasePoolPush();
            let result = prompt_in_pool(title, message);
            objc_autoreleasePoolPop(pool);
            result
        }
    }

    unsafe fn prompt_in_pool(title: &str, message: &str) -> Result<Option<String>, AppError> {
        let alert_class = class("NSAlert")?;
        let secure_field_class = class("NSSecureTextField")?;
        let alert = send_id(send_id(alert_class, "alloc")?, "init")?;
        let title = ns_string(title)?;
        let message = ns_string(message)?;
        let save = ns_string("Save")?;
        let cancel = ns_string("Cancel")?;
        let empty = ns_string("")?;
        send_void_id(alert, "setMessageText:", title)?;
        send_void_id(alert, "setInformativeText:", message)?;
        let _ = send_id_id(alert, "addButtonWithTitle:", save)?;
        let _ = send_id_id(alert, "addButtonWithTitle:", cancel)?;
        let field = send_id_id(secure_field_class, "textFieldWithString:", empty)?;
        send_void_id(alert, "setAccessoryView:", field)?;
        let response = send_isize(alert, "runModal")?;
        if response != FIRST_BUTTON_RETURN {
            return Ok(None);
        }
        let value = send_id(field, "stringValue")?;
        let bytes = send_c_string(value, "UTF8String")?;
        Ok(Some(
            CStr::from_ptr(bytes)
                .to_str()
                .map_err(|_| prompt_error())?
                .to_owned(),
        ))
    }

    unsafe fn class(name: &str) -> Result<Id, AppError> {
        let name = CString::new(name).map_err(|_| prompt_error())?;
        let value = unsafe { objc_getClass(name.as_ptr()) };
        (!value.is_null()).then_some(value).ok_or_else(prompt_error)
    }

    unsafe fn selector(name: &str) -> Result<Sel, AppError> {
        let name = CString::new(name).map_err(|_| prompt_error())?;
        let value = unsafe { sel_registerName(name.as_ptr()) };
        (!value.is_null()).then_some(value).ok_or_else(prompt_error)
    }

    unsafe fn send_id(receiver: Id, name: &str) -> Result<Id, AppError> {
        let function: unsafe extern "C" fn(Id, Sel) -> Id =
            unsafe { std::mem::transmute(objc_msgSend as *const ()) };
        let value = unsafe { function(receiver, selector(name)?) };
        (!value.is_null()).then_some(value).ok_or_else(prompt_error)
    }

    unsafe fn send_id_id(receiver: Id, name: &str, argument: Id) -> Result<Id, AppError> {
        let function: unsafe extern "C" fn(Id, Sel, Id) -> Id =
            unsafe { std::mem::transmute(objc_msgSend as *const ()) };
        let value = unsafe { function(receiver, selector(name)?, argument) };
        (!value.is_null()).then_some(value).ok_or_else(prompt_error)
    }

    unsafe fn send_void_id(receiver: Id, name: &str, argument: Id) -> Result<(), AppError> {
        let function: unsafe extern "C" fn(Id, Sel, Id) =
            unsafe { std::mem::transmute(objc_msgSend as *const ()) };
        unsafe { function(receiver, selector(name)?, argument) };
        Ok(())
    }

    unsafe fn send_isize(receiver: Id, name: &str) -> Result<isize, AppError> {
        let function: unsafe extern "C" fn(Id, Sel) -> isize =
            unsafe { std::mem::transmute(objc_msgSend as *const ()) };
        Ok(unsafe { function(receiver, selector(name)?) })
    }

    unsafe fn send_c_string(receiver: Id, name: &str) -> Result<*const c_char, AppError> {
        let function: unsafe extern "C" fn(Id, Sel) -> *const c_char =
            unsafe { std::mem::transmute(objc_msgSend as *const ()) };
        let value = unsafe { function(receiver, selector(name)?) };
        if value.is_null() {
            Err(prompt_error())
        } else {
            Ok(value)
        }
    }

    unsafe fn ns_string(value: &str) -> Result<Id, AppError> {
        let class = class("NSString")?;
        let bytes = CString::new(value).map_err(|_| prompt_error())?;
        let selector = selector("stringWithUTF8String:")?;
        let function: unsafe extern "C" fn(Id, Sel, *const c_char) -> Id =
            unsafe { std::mem::transmute(objc_msgSend as *const ()) };
        let value = unsafe { function(class, selector, bytes.as_ptr()) };
        (!value.is_null()).then_some(value).ok_or_else(prompt_error)
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use std::{ffi::c_void, mem, ptr};

    use super::{prompt_error, AppError};

    const ERROR_CANCELLED: u32 = 1223;
    const CREDUI_FLAGS_DO_NOT_PERSIST: u32 = 0x0000_0002;
    const CREDUI_FLAGS_GENERIC_CREDENTIALS: u32 = 0x0004_0000;
    const MAX_USERNAME: usize = 1;
    const MAX_PASSWORD: usize = 512;

    #[repr(C)]
    pub(super) struct CredUiInfo {
        pub(super) size: u32,
        pub(super) parent: *mut c_void,
        message_text: *const u16,
        caption_text: *const u16,
        banner: *mut c_void,
    }

    pub(super) fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub(super) fn cred_ui_info(title: &[u16], message: &[u16], parent: *mut c_void) -> CredUiInfo {
        CredUiInfo {
            size: mem::size_of::<CredUiInfo>() as u32,
            parent,
            message_text: message.as_ptr(),
            caption_text: title.as_ptr(),
            banner: ptr::null_mut(),
        }
    }

    #[link(name = "Credui")]
    unsafe extern "system" {
        fn CredUIPromptForCredentialsW(
            info: *const CredUiInfo,
            target_name: *const u16,
            reserved: *mut c_void,
            authentication_error: u32,
            user_name: *mut u16,
            user_name_max_chars: u32,
            password: *mut u16,
            password_max_chars: u32,
            save: *mut i32,
            flags: u32,
        ) -> u32;
    }

    pub fn prompt(
        title: &str,
        message: &str,
        parent: *mut c_void,
    ) -> Result<Option<String>, AppError> {
        let title = wide(title);
        let message = wide(message);
        let target = wide("Desktop Translator Google Cloud Translation");
        let mut username = [0_u16; MAX_USERNAME];
        let mut password = [0_u16; MAX_PASSWORD];
        let mut save = 0;
        let info = cred_ui_info(&title, &message, parent);
        if !parent.is_null() {
            let hwnd = ::windows::Win32::Foundation::HWND(parent);
            // SAFETY: `parent` is the invoking Tauri window HWND and remains
            // valid for this synchronous CredUI call on the UI thread.
            unsafe {
                let _ = ::windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd);
                let _ = ::windows::Win32::UI::WindowsAndMessaging::BringWindowToTop(hwnd);
            }
        }
        // SAFETY: all pointers refer to live, correctly sized buffers for the call.
        let status = unsafe {
            CredUIPromptForCredentialsW(
                &info,
                target.as_ptr(),
                ptr::null_mut(),
                0,
                username.as_mut_ptr(),
                username.len() as u32,
                password.as_mut_ptr(),
                password.len() as u32,
                &mut save,
                CREDUI_FLAGS_DO_NOT_PERSIST | CREDUI_FLAGS_GENERIC_CREDENTIALS,
            )
        };
        if status == ERROR_CANCELLED {
            return Ok(None);
        }
        if status != 0 {
            return Err(prompt_error());
        }
        let length = password
            .iter()
            .position(|character| *character == 0)
            .unwrap_or(password.len());
        let value = String::from_utf16(&password[..length]).map_err(|_| prompt_error())?;
        password.fill(0);
        Ok(Some(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "windows")]
    #[test]
    fn cred_ui_info_uses_the_invoking_window_as_owner() {
        let title = windows::wide("Google Cloud Translation API Key");
        let message =
            windows::wide("The key is stored directly in the operating-system credential vault.");
        let parent = 0x00BEEF_usize as *mut std::ffi::c_void;
        let info = windows::cred_ui_info(&title, &message, parent);
        assert_eq!(info.parent, parent);
        assert_eq!(info.size, std::mem::size_of::<windows::CredUiInfo>() as u32);
        assert!(!info.parent.is_null());
    }

    #[test]
    fn overlapping_credential_prompts_are_rejected() {
        assert!(try_begin_prompt());
        assert!(!try_begin_prompt());
        end_prompt();
        assert!(try_begin_prompt());
        end_prompt();
    }
}
