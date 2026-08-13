//! Native speech synthesis backed by AVSpeechSynthesizer.

use std::{
    ffi::{c_char, c_void, CString},
    mem,
    sync::Mutex,
    thread::{self, JoinHandle},
};

use async_trait::async_trait;
use crossbeam_channel::{bounded, select_biased, unbounded, Receiver, Sender};

use crate::{
    contracts::{AppError, AppErrorCode},
    platform::SpeechAdapter,
};

type Id = *mut c_void;
type Sel = *mut c_void;
type Class = *mut c_void;
type ObjcBool = i8;

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {}

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Class;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

enum SpeechCommand {
    Available {
        language: String,
        response: Sender<bool>,
    },
    Speak {
        text: String,
        language: String,
        response: Sender<Result<(), AppError>>,
    },
}

enum UrgentCommand {
    Stop {
        response: Sender<Result<(), AppError>>,
    },
    Shutdown,
}

/// Thread-confined AVSpeechSynthesizer command adapter.
///
/// All Objective-C objects remain on the dedicated speech worker. Public calls
/// exchange owned strings and results only, making the adapter Send + Sync
/// without claiming Objective-C object thread safety.
pub struct MacSpeechAdapter {
    commands: Sender<SpeechCommand>,
    urgent: Sender<UrgentCommand>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl MacSpeechAdapter {
    pub fn new() -> Result<Self, AppError> {
        let (commands, receiver) = bounded(16);
        let (urgent, urgent_receiver) = unbounded();
        let (ready_tx, ready_rx) = bounded(1);
        let worker = thread::Builder::new()
            .name("macos-native-speech".into())
            .spawn(move || speech_worker(receiver, urgent_receiver, ready_tx))
            .map_err(|_| speech_error("Native speech worker could not start"))?;
        ready_rx
            .recv()
            .map_err(|_| speech_error("Native speech worker stopped during startup"))??;
        Ok(Self {
            commands,
            urgent,
            worker: Mutex::new(Some(worker)),
        })
    }
}

#[async_trait]
impl SpeechAdapter for MacSpeechAdapter {
    async fn is_available(&self, language: &str) -> bool {
        if !valid_language(language) {
            return false;
        }
        let commands = self.commands.clone();
        let language = language.to_owned();
        tokio::task::spawn_blocking(move || {
            let (response_tx, response_rx) = bounded(1);
            if commands
                .send(SpeechCommand::Available {
                    language,
                    response: response_tx,
                })
                .is_err()
            {
                return false;
            }
            response_rx.recv().unwrap_or(false)
        })
        .await
        .unwrap_or(false)
    }

    async fn speak(&self, text: &str, language: &str) -> Result<(), AppError> {
        if text.trim().is_empty() || !valid_language(language) {
            return Err(AppError::new(
                AppErrorCode::InvalidLanguagePair,
                "Speech text and language must be non-empty",
                false,
            ));
        }
        let commands = self.commands.clone();
        let text = text.to_owned();
        let language = language.to_owned();
        tokio::task::spawn_blocking(move || {
            let (response_tx, response_rx) = bounded(1);
            commands
                .send(SpeechCommand::Speak {
                    text,
                    language,
                    response: response_tx,
                })
                .map_err(|_| speech_error("Native speech service is unavailable"))?;
            response_rx
                .recv()
                .map_err(|_| speech_error("Native speech service stopped unexpectedly"))?
        })
        .await
        .map_err(|_| speech_error("Native speech wait task failed"))?
    }

    async fn stop(&self) -> Result<(), AppError> {
        let urgent = self.urgent.clone();
        tokio::task::spawn_blocking(move || {
            let (response_tx, response_rx) = bounded(1);
            urgent
                .send(UrgentCommand::Stop {
                    response: response_tx,
                })
                .map_err(|_| speech_error("Native speech service is unavailable"))?;
            response_rx
                .recv()
                .map_err(|_| speech_error("Native speech service stopped unexpectedly"))?
        })
        .await
        .map_err(|_| speech_error("Native speech stop task failed"))?
    }
}

impl Drop for MacSpeechAdapter {
    fn drop(&mut self) {
        let _ = self.urgent.send(UrgentCommand::Shutdown);
        if let Ok(worker) = self.worker.get_mut() {
            if let Some(worker) = worker.take() {
                let _ = worker.join();
            }
        }
    }
}

fn speech_worker(
    receiver: Receiver<SpeechCommand>,
    urgent: Receiver<UrgentCommand>,
    ready: Sender<Result<(), AppError>>,
) {
    // SAFETY: the pool is balanced on this worker and wraps startup objects.
    let pool = unsafe { objc_autoreleasePoolPush() };
    let synthesizer = create_synthesizer();
    unsafe { objc_autoreleasePoolPop(pool) };
    let synthesizer = match synthesizer {
        Ok(value) => {
            let _ = ready.send(Ok(()));
            value
        }
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };

    'worker: loop {
        let command = select_biased! {
            recv(urgent) -> command => {
                match command {
                    Ok(command) => {
                        if handle_urgent(command, synthesizer, &receiver) {
                            break;
                        }
                        continue;
                    }
                    Err(_) => break,
                }
            }
            recv(receiver) -> command => {
                match command {
                    Ok(command) => command,
                    Err(_) => break,
                }
            }
        };
        if let Ok(urgent_command) = urgent.try_recv() {
            cancel_command(command);
            if handle_urgent(urgent_command, synthesizer, &receiver) {
                break 'worker;
            }
            continue;
        }
        // SAFETY: every command creates an independent balanced autorelease pool.
        let pool = unsafe { objc_autoreleasePoolPush() };
        match command {
            SpeechCommand::Available { language, response } => {
                let _ = response.send(native_voice(&language).is_some());
            }
            SpeechCommand::Speak {
                text,
                language,
                response,
            } => {
                let result = native_speak(synthesizer, &text, &language);
                let _ = response.send(result);
            }
        }
        // SAFETY: balances the command's autorelease pool on this thread.
        unsafe { objc_autoreleasePoolPop(pool) };
    }

    // SAFETY: synthesizer is the +1 result of alloc/init and remains on worker.
    unsafe {
        let _ = send_no_args::<()>(synthesizer, "release");
    }
}

fn handle_urgent(
    command: UrgentCommand,
    synthesizer: Id,
    pending: &Receiver<SpeechCommand>,
) -> bool {
    match command {
        UrgentCommand::Stop { response } => {
            // SAFETY: pool is balanced on the speech worker.
            let pool = unsafe { objc_autoreleasePoolPush() };
            let result = native_stop(synthesizer);
            cancel_pending(pending);
            let _ = response.send(result);
            unsafe { objc_autoreleasePoolPop(pool) };
            false
        }
        UrgentCommand::Shutdown => {
            cancel_pending(pending);
            true
        }
    }
}

fn cancel_pending(pending: &Receiver<SpeechCommand>) {
    while let Ok(command) = pending.try_recv() {
        cancel_command(command);
    }
}

fn cancel_command(command: SpeechCommand) {
    match command {
        SpeechCommand::Available { response, .. } => {
            let _ = response.send(false);
        }
        SpeechCommand::Speak { response, .. } => {
            let _ = response.send(Err(speech_error("Speech request was superseded by stop")));
        }
    }
}

fn create_synthesizer() -> Result<Id, AppError> {
    let class = class("AVSpeechSynthesizer")?;
    // SAFETY: class and selectors use the standard alloc/init ABI.
    let allocated = unsafe { send_no_args::<Id>(class, "alloc")? };
    let synthesizer = unsafe { send_no_args::<Id>(allocated, "init")? };
    if synthesizer.is_null() {
        Err(speech_error("AVSpeechSynthesizer could not initialize"))
    } else {
        Ok(synthesizer)
    }
}

fn native_voice(language: &str) -> Option<Id> {
    let class = class("AVSpeechSynthesisVoice").ok()?;
    let language = ns_string(language).ok()?;
    // SAFETY: voiceWithLanguage: accepts an NSString and returns autoreleased id.
    let voice = unsafe { send_object::<Id>(class, "voiceWithLanguage:", language).ok()? };
    (!voice.is_null()).then_some(voice)
}

fn native_speak(synthesizer: Id, text: &str, language: &str) -> Result<(), AppError> {
    let voice = native_voice(language)
        .ok_or_else(|| speech_error("No installed voice supports language"))?;
    let utterance_class = class("AVSpeechUtterance")?;
    let text = ns_string(text)?;
    // SAFETY: speechUtteranceWithString: accepts NSString and returns utterance.
    let utterance =
        unsafe { send_object::<Id>(utterance_class, "speechUtteranceWithString:", text)? };
    if utterance.is_null() {
        return Err(speech_error("AVSpeechUtterance could not initialize"));
    }
    // SAFETY: voice and utterance are live for this autorelease pool; the
    // synthesizer retains the utterance when enqueued.
    unsafe {
        send_object::<()>(utterance, "setVoice:", voice)?;
        send_object::<()>(synthesizer, "speakUtterance:", utterance)?;
    }
    Ok(())
}

fn native_stop(synthesizer: Id) -> Result<(), AppError> {
    // AVSpeechBoundary.immediate is zero.
    // SAFETY: selector ABI accepts an NSInteger boundary and returns BOOL.
    unsafe {
        let _ = send_usize::<ObjcBool>(synthesizer, "stopSpeakingAtBoundary:", 0)?;
    }
    Ok(())
}

fn valid_language(language: &str) -> bool {
    let value = language.trim();
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn class(name: &str) -> Result<Class, AppError> {
    let name = CString::new(name).map_err(|_| speech_error("Invalid Objective-C class name"))?;
    // SAFETY: class name is a valid NUL-terminated string.
    let class = unsafe { objc_getClass(name.as_ptr()) };
    if class.is_null() {
        Err(speech_error("Required AVFoundation class is unavailable"))
    } else {
        Ok(class)
    }
}

fn ns_string(value: &str) -> Result<Id, AppError> {
    let class = class("NSString")?;
    let value = CString::new(value).map_err(|_| speech_error("Speech text contains a NUL byte"))?;
    let selector = selector("stringWithUTF8String:")?;
    type Send = unsafe extern "C" fn(Id, Sel, *const c_char) -> Id;
    // SAFETY: objc_msgSend is cast to the exact NSString factory ABI.
    let send: Send = unsafe { mem::transmute(objc_msgSend as *const ()) };
    let string = unsafe { send(class, selector, value.as_ptr()) };
    if string.is_null() {
        Err(speech_error("NSString could not encode speech input"))
    } else {
        Ok(string)
    }
}

fn selector(name: &str) -> Result<Sel, AppError> {
    let name = CString::new(name).map_err(|_| speech_error("Invalid Objective-C selector"))?;
    // SAFETY: selector name is a valid NUL-terminated string.
    let selector = unsafe { sel_registerName(name.as_ptr()) };
    if selector.is_null() {
        Err(speech_error("Objective-C selector is unavailable"))
    } else {
        Ok(selector)
    }
}

unsafe fn send_no_args<R>(object: Id, name: &str) -> Result<R, AppError> {
    let selector = selector(name)?;
    type Send<R> = unsafe extern "C" fn(Id, Sel) -> R;
    // SAFETY: objc_msgSend is cast to the exact no-argument selector ABI.
    let send: Send<R> = unsafe { mem::transmute(objc_msgSend as *const ()) };
    Ok(unsafe { send(object, selector) })
}

unsafe fn send_object<R>(object: Id, name: &str, argument: Id) -> Result<R, AppError> {
    let selector = selector(name)?;
    type Send<R> = unsafe extern "C" fn(Id, Sel, Id) -> R;
    // SAFETY: objc_msgSend is cast to the exact object-argument selector ABI.
    let send: Send<R> = unsafe { mem::transmute(objc_msgSend as *const ()) };
    Ok(unsafe { send(object, selector, argument) })
}

unsafe fn send_usize<R>(object: Id, name: &str, argument: usize) -> Result<R, AppError> {
    let selector = selector(name)?;
    type Send<R> = unsafe extern "C" fn(Id, Sel, usize) -> R;
    // SAFETY: objc_msgSend is cast to the exact integer-argument selector ABI.
    let send: Send<R> = unsafe { mem::transmute(objc_msgSend as *const ()) };
    Ok(unsafe { send(object, selector, argument) })
}

fn speech_error(message: &'static str) -> AppError {
    AppError::new(AppErrorCode::Internal, message, false)
}

#[cfg(test)]
mod tests {
    use crossbeam_channel::bounded;

    use crate::platform::SpeechAdapter;

    use super::{cancel_pending, valid_language, MacSpeechAdapter, SpeechCommand};

    #[test]
    fn accepts_bcp47_style_language_identifiers() {
        assert!(valid_language("en-US"));
        assert!(valid_language("zh-Hant-TW"));
    }

    #[test]
    fn rejects_empty_or_injection_shaped_language_identifiers() {
        assert!(!valid_language(""));
        assert!(!valid_language("en US"));
        assert!(!valid_language("en\0US"));
    }

    #[test]
    fn stop_cancellation_drains_queued_speech_without_waiting_for_it() {
        let (commands, pending) = bounded(2);
        let (first_response, first_result) = bounded(1);
        let (second_response, second_result) = bounded(1);
        commands
            .send(SpeechCommand::Speak {
                text: "first".into(),
                language: "en-US".into(),
                response: first_response,
            })
            .expect("first queued");
        commands
            .send(SpeechCommand::Speak {
                text: "second".into(),
                language: "en-US".into(),
                response: second_response,
            })
            .expect("second queued");

        cancel_pending(&pending);

        assert!(first_result.recv().expect("first response").is_err());
        assert!(second_result.recv().expect("second response").is_err());
    }

    #[tokio::test]
    #[ignore = "manual macOS fixture: requires an installed en-US voice and audible output"]
    async fn manual_avspeech_voice_speaks_and_stops() {
        let speech = MacSpeechAdapter::new().expect("AVSpeechSynthesizer");
        assert!(speech.is_available("en-US").await);
        speech
            .speak("Desktop translator native speech fixture.", "en-US")
            .await
            .expect("native speech begins");
        speech.stop().await.expect("native speech stops");
    }
}
