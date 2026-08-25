//! Native Windows SAPI speech on a dedicated COM worker.

use std::{
    future::Future,
    pin::Pin,
    sync::{mpsc, Arc, Mutex},
    task::{Context, Poll, Waker},
    thread,
};

use async_trait::async_trait;
use windows::{
    core::PCWSTR,
    Win32::{
        Globalization::{LocaleNameToLCID, ResolveLocaleName, LOCALE_ALLOW_NEUTRAL_NAMES},
        Media::Speech::{
            ISpObjectToken, ISpObjectTokenCategory, ISpVoice, SpObjectTokenCategory, SpVoice,
            SPCAT_VOICES, SPEAKFLAGS, SPF_ASYNC, SPF_PURGEBEFORESPEAK,
        },
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_MULTITHREADED,
        },
    },
};

use crate::{
    contracts::{AppError, AppErrorCode},
    platform::SpeechAdapter,
};

/// Win32 `LOCALE_NAME_MAX_LENGTH`, including the terminating NUL.
/// The `windows` 0.62 crate no longer exports this Globalization constant.
const LOCALE_NAME_MAX_LENGTH: usize = 85;

enum SpeechCommand {
    IsAvailable {
        language: String,
        reply: ReplySender<bool>,
    },
    Speak {
        text: String,
        language: String,
        reply: ReplySender<Result<(), AppError>>,
    },
    Stop {
        reply: ReplySender<Result<(), AppError>>,
    },
}

/// Native speech adapter backed by the installed Windows SAPI voices.
#[derive(Clone)]
pub struct WindowsSpeechAdapter {
    commands: mpsc::Sender<SpeechCommand>,
}

impl WindowsSpeechAdapter {
    /// Starts the thread-confined SAPI voice used for all speech operations.
    pub fn new() -> Result<Self, AppError> {
        let (commands, receiver) = mpsc::channel();
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("windows-native-speech".into())
            .spawn(move || run_speech_worker(receiver, started_tx))
            .map_err(|_| internal("could not start Windows speech worker"))?;
        started_rx
            .recv()
            .map_err(|_| internal("Windows speech worker exited during startup"))??;
        Ok(Self { commands })
    }
}

#[async_trait]
impl SpeechAdapter for WindowsSpeechAdapter {
    async fn is_available(&self, language: &str) -> bool {
        let (reply, response) = reply_channel(false);
        if self
            .commands
            .send(SpeechCommand::IsAvailable {
                language: language.to_owned(),
                reply,
            })
            .is_err()
        {
            return false;
        }
        response.await
    }

    async fn speak(&self, text: &str, language: &str) -> Result<(), AppError> {
        if text.trim().is_empty() {
            return Err(AppError::new(
                AppErrorCode::NoSelection,
                "speech text is empty",
                false,
            ));
        }
        let (reply, response) = reply_channel(Err(internal(
            "Windows speech worker exited before speaking",
        )));
        self.commands
            .send(SpeechCommand::Speak {
                text: text.to_owned(),
                language: language.to_owned(),
                reply,
            })
            .map_err(|_| internal("Windows speech worker is unavailable"))?;
        response.await
    }

    async fn stop(&self) -> Result<(), AppError> {
        let (reply, response) = reply_channel(Err(internal(
            "Windows speech worker exited before stopping speech",
        )));
        self.commands
            .send(SpeechCommand::Stop { reply })
            .map_err(|_| internal("Windows speech worker is unavailable"))?;
        response.await
    }
}

fn run_speech_worker(
    commands: mpsc::Receiver<SpeechCommand>,
    started: mpsc::SyncSender<Result<(), AppError>>,
) {
    // SAFETY: COM is initialized and uninitialized on this same worker. SAPI
    // interfaces and voice tokens never leave this thread.
    if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_err() {
        let _ = started.send(Err(internal("could not initialize Windows speech COM")));
        return;
    }
    let voice = unsafe {
        CoCreateInstance::<_, ISpVoice>(&SpVoice, None, CLSCTX_INPROC_SERVER)
            .map_err(|_| internal("could not create the Windows SAPI voice"))
    };
    let voice = match voice {
        Ok(voice) => voice,
        Err(error) => {
            let _ = started.send(Err(error));
            unsafe { CoUninitialize() };
            return;
        }
    };
    let category = unsafe {
        CoCreateInstance::<_, ISpObjectTokenCategory>(
            &SpObjectTokenCategory,
            None,
            CLSCTX_INPROC_SERVER,
        )
        .and_then(|category| {
            category.SetId(SPCAT_VOICES, false)?;
            Ok(category)
        })
        .map_err(|_| internal("could not enumerate installed Windows voices"))
    };
    let category = match category {
        Ok(category) => {
            let _ = started.send(Ok(()));
            category
        }
        Err(error) => {
            let _ = started.send(Err(error));
            drop(voice);
            unsafe { CoUninitialize() };
            return;
        }
    };

    for command in commands {
        match command {
            SpeechCommand::IsAvailable { language, reply } => {
                reply.complete(find_voice(&category, &language).is_some());
            }
            SpeechCommand::Speak {
                text,
                language,
                reply,
            } => {
                let result = speak_with_voice(&voice, &category, &text, &language);
                reply.complete(result);
            }
            SpeechCommand::Stop { reply } => {
                // SAFETY: a null text pointer with PURGEBEFORESPEAK is SAPI's
                // documented cancellation operation for the current queue.
                let result =
                    unsafe { voice.Speak(PCWSTR::null(), speak_flags(SPF_PURGEBEFORESPEAK), None) }
                        .map(|_| ())
                        .map_err(|_| internal("could not stop Windows speech"));
                reply.complete(result);
            }
        }
    }

    drop(category);
    drop(voice);
    unsafe { CoUninitialize() };
}

fn speak_with_voice(
    voice: &ISpVoice,
    category: &ISpObjectTokenCategory,
    text: &str,
    language: &str,
) -> Result<(), AppError> {
    let token = find_voice(category, language).ok_or_else(|| {
        AppError::new(
            AppErrorCode::UnsupportedControl,
            "no installed Windows voice supports this language",
            false,
        )
    })?;
    let text = wide(text);

    // SAFETY: `token` and `voice` are valid thread-confined COM interfaces.
    // `text` is NUL-terminated and remains alive for the synchronous enqueue.
    unsafe {
        voice
            .SetVoice(&token)
            .and_then(|_| {
                voice.Speak(
                    PCWSTR(text.as_ptr()),
                    speak_flags(SPF_ASYNC) | speak_flags(SPF_PURGEBEFORESPEAK),
                    None,
                )
            })
            .map(|_| ())
            .map_err(|_| internal("Windows could not begin native speech"))
    }
}

fn find_voice(category: &ISpObjectTokenCategory, language: &str) -> Option<ISpObjectToken> {
    let attribute = language_attribute(language)?;
    let attribute = wide(&attribute);
    // SAFETY: both strings are NUL-terminated for the duration of GetVoices;
    // the returned enumerator and tokens remain on this COM worker.
    let voices = unsafe {
        category
            .EnumTokens(PCWSTR(attribute.as_ptr()), PCWSTR::null())
            .ok()?
    };
    let mut count = 0;
    unsafe { voices.GetCount(&mut count).ok()? };
    if count == 0 {
        None
    } else {
        unsafe { voices.Item(0).ok() }
    }
}

fn language_attribute(language: &str) -> Option<String> {
    let normalized = language.trim().replace('_', "-");
    if normalized.is_empty() {
        return None;
    }
    let wide_language = wide(&normalized);
    let mut resolved = [0u16; LOCALE_NAME_MAX_LENGTH];
    // SAFETY: the source is NUL-terminated and `resolved` supplies the exact
    // writable capacity passed to ResolveLocaleName.
    let resolved_len =
        unsafe { ResolveLocaleName(PCWSTR(wide_language.as_ptr()), Some(&mut resolved)) };
    let locale_name = if resolved_len > 0 {
        PCWSTR(resolved.as_ptr())
    } else {
        PCWSTR(wide_language.as_ptr())
    };
    // SAFETY: `locale_name` points to one of the live NUL-terminated buffers.
    let locale = unsafe { LocaleNameToLCID(locale_name, LOCALE_ALLOW_NEUTRAL_NAMES) };
    if locale == 0 {
        None
    } else {
        Some(voice_language_attribute(locale))
    }
}

fn voice_language_attribute(locale: u32) -> String {
    format!("Language={:04x}", locale & 0xffff)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn speak_flags(flag: SPEAKFLAGS) -> u32 {
    flag.0 as u32
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

    use windows::Win32::Media::Speech::{SPF_ASYNC, SPF_PURGEBEFORESPEAK};

    use crate::contracts::AppErrorCode;

    use super::{language_attribute, reply_channel, speak_flags, voice_language_attribute, wide};

    #[test]
    fn wide_strings_are_nul_terminated() {
        assert_eq!(wide("hello").last(), Some(&0));
    }

    #[test]
    fn empty_language_is_never_available() {
        assert_eq!(language_attribute("  "), None);
    }

    #[test]
    fn sapi_language_attribute_uses_low_word_hex_langid() {
        assert_eq!(voice_language_attribute(0x0000_0409), "Language=0409");
        assert_eq!(voice_language_attribute(0x1234_0804), "Language=0804");
    }

    #[test]
    fn speak_flags_encode_as_u32_without_bitor_on_speakflags() {
        assert_eq!(
            speak_flags(SPF_ASYNC) | speak_flags(SPF_PURGEBEFORESPEAK),
            3
        );
    }

    #[test]
    fn dropped_sapi_reply_completes_with_stable_error() {
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
