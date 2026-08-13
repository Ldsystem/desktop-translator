//! Native secure credential entry that never places the API key in WebView state.

use crate::contracts::{AppError, AppErrorCode};

/// Prompts for an API key using the platform's native secure credential control.
#[cfg(target_os = "macos")]
pub fn prompt_secure_text(title: &str, message: &str) -> Result<Option<String>, AppError> {
    macos::prompt(title, message)
}

/// Prompts for an API key using Windows Credential UI without persistence.
#[cfg(target_os = "windows")]
pub fn prompt_secure_text(title: &str, message: &str) -> Result<Option<String>, AppError> {
    windows::prompt(title, message)
}

fn prompt_error() -> AppError {
    AppError::new(
        AppErrorCode::Internal,
        "Secure credential entry is unavailable.",
        false,
    )
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
    struct CredUiInfo {
        size: u32,
        parent: *mut c_void,
        message_text: *const u16,
        caption_text: *const u16,
        banner: *mut c_void,
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

    pub fn prompt(title: &str, message: &str) -> Result<Option<String>, AppError> {
        let title = wide(title);
        let message = wide(message);
        let target = wide("Desktop Translator Google Cloud Translation");
        let mut username = [0_u16; MAX_USERNAME];
        let mut password = [0_u16; MAX_PASSWORD];
        let mut save = 0;
        let info = CredUiInfo {
            size: mem::size_of::<CredUiInfo>() as u32,
            parent: ptr::null_mut(),
            message_text: message.as_ptr(),
            caption_text: title.as_ptr(),
            banner: ptr::null_mut(),
        };
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

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }
}
