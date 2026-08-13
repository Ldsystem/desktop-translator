import { useEffect } from "react";

import type {
  AppErrorCode,
  LanguageCode,
  TranslationRequest,
} from "../../contracts/ipc";
import type { OverlayState } from "../../state/overlayMachine";

export interface LanguageOption {
  code: LanguageCode;
  label: string;
}

export const defaultLanguages: readonly LanguageOption[] = [
  { code: "en", label: "English" },
  { code: "zh-CN", label: "Chinese (Simplified)" },
  { code: "ja", label: "Japanese" },
  { code: "ko", label: "Korean" },
  { code: "fr", label: "French" },
  { code: "de", label: "German" },
  { code: "es", label: "Spanish" },
];

interface ContextualOverlayProps {
  state: OverlayState;
  sourceLanguage: "auto" | LanguageCode;
  targetLanguage: LanguageCode;
  languages?: readonly LanguageOption[];
  speechAvailability?: Readonly<Record<string, boolean>>;
  onTranslate: (request: TranslationRequest) => void;
  onCorrectSource: (request: TranslationRequest) => void;
  onSpeak: (text: string, language: LanguageCode) => void;
  onDismiss: () => void;
}

const errorCopy: Record<AppErrorCode, { title: string; guidance: string }> = {
  "permission-denied": {
    title: "Accessibility Permission Required",
    guidance: "Allow selection access in System Settings.",
  },
  "unsupported-control": {
    title: "Text Control Not Supported",
    guidance: "Select text in a standard editable or document control.",
  },
  "no-selection": {
    title: "No Text Selected",
    guidance: "Select text, then try again.",
  },
  "missing-credential": {
    title: "API Key Required",
    guidance: "Save an API key in Settings.",
  },
  "invalid-credential": {
    title: "API Key Not Accepted",
    guidance: "Replace the API key in Settings.",
  },
  "api-restricted": {
    title: "API Access Restricted",
    guidance: "Allow translation access for this API key.",
  },
  "billing-required": {
    title: "Billing Required",
    guidance: "Enable billing for the translation service.",
  },
  "quota-exceeded": {
    title: "Translation Quota Reached",
    guidance: "Check your service quota, then retry.",
  },
  offline: {
    title: "You’re Offline",
    guidance: "Reconnect to the internet, then retry.",
  },
  timeout: {
    title: "Translation Timed Out",
    guidance: "Check your connection, then retry.",
  },
  "service-unavailable": {
    title: "Translation Service Unavailable",
    guidance: "Wait a moment, then retry.",
  },
  "invalid-language-pair": {
    title: "Language Pair Not Supported",
    guidance: "Choose a different source or target language.",
  },
  internal: {
    title: "Translation Failed",
    guidance: "Dismiss this result and try again.",
  },
};

function SpeakerIcon() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
      <path d="M4 9v6h4l5 4V5L8 9H4Zm12.5-.7a5 5 0 0 1 0 7.4M19 5a9 9 0 0 1 0 14" />
    </svg>
  );
}

function CloseIcon() {
  return (
    <svg viewBox="0 0 24 24" width="17" height="17" aria-hidden="true">
      <path d="m7 7 10 10M17 7 7 17" />
    </svg>
  );
}

export function ContextualOverlay({
  state,
  sourceLanguage,
  targetLanguage,
  languages = defaultLanguages,
  speechAvailability = {},
  onTranslate,
  onCorrectSource,
  onSpeak,
  onDismiss,
}: ContextualOverlayProps) {
  useEffect(() => {
    const handleEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && state.mode !== "idle" && state.mode !== "disabled") {
        onDismiss();
      }
    };
    document.addEventListener("keydown", handleEscape);
    return () => document.removeEventListener("keydown", handleEscape);
  }, [onDismiss, state.mode]);

  if (
    state.mode === "disabled" ||
    state.mode === "idle" ||
    state.mode === "pointer-down" ||
    state.mode === "resolving-selection"
  ) {
    return null;
  }

  const request = (correctedSource = sourceLanguage): TranslationRequest => ({
    selectionId: state.selection.id,
    text: state.selection.text,
    sourceLanguage: correctedSource,
    targetLanguage,
  });

  if (state.mode === "button-visible") {
    return (
      <aside className="contextual-trigger" aria-label="Translation Controls">
        <button
          className="translate-trigger"
          type="button"
          aria-label="Translate Selected Text"
          onClick={() => onTranslate(request())}
        >
          <span aria-hidden="true">A</span>
          <span className="translate-trigger__swap" aria-hidden="true">文</span>
        </button>
      </aside>
    );
  }

  return (
    <aside className="contextual-card" aria-label="Translation">
      <div className="contextual-card__rail" aria-hidden="true" />
      <button className="icon-button dismiss-button" type="button" aria-label="Dismiss" onClick={onDismiss}>
        <CloseIcon />
      </button>

      {state.mode === "translating" && (
        <div className="loading-state" role="status" aria-live="polite">
          <span className="loading-mark" aria-hidden="true">
            <span />
            <span />
            <span />
          </span>
          <span>Translating…</span>
        </div>
      )}

      {state.mode === "result-visible" && (
        <div className="translation-result" aria-live="polite">
          <section className="translation-block translation-block--source" aria-labelledby="source-heading">
            <div className="translation-block__header">
              <label id="source-heading" htmlFor="overlay-source-language">
                Source
              </label>
              <select
                id="overlay-source-language"
                name="overlay-source-language"
                value={state.result.effectiveSourceLanguage}
                onChange={(event) => onCorrectSource(request(event.currentTarget.value))}
                aria-label="Correct Source Language"
              >
                {languages.map((language) => (
                  <option key={language.code} value={language.code}>
                    {language.label}
                  </option>
                ))}
              </select>
              <button
                className="icon-button"
                type="button"
                aria-label="Speak Source Text"
                disabled={speechAvailability[state.result.effectiveSourceLanguage] !== true}
                title={
                  speechAvailability[state.result.effectiveSourceLanguage] === false
                    ? "No installed voice supports this language"
                    : speechAvailability[state.result.effectiveSourceLanguage] === undefined
                      ? "Checking installed voice availability"
                      : undefined
                }
                onClick={() => onSpeak(state.selection.text, state.result.effectiveSourceLanguage)}
              >
                <SpeakerIcon />
              </button>
            </div>
            <p lang={state.result.effectiveSourceLanguage}>{state.selection.text}</p>
          </section>

          <section className="translation-block translation-block--target" aria-labelledby="target-heading">
            <div className="translation-block__header">
              <h2 id="target-heading">
                {languages.find((language) => language.code === state.result.targetLanguage)?.label ??
                  state.result.targetLanguage}
              </h2>
              <button
                className="icon-button"
                type="button"
                aria-label="Speak Translation"
                disabled={speechAvailability[state.result.targetLanguage] !== true}
                title={
                  speechAvailability[state.result.targetLanguage] === false
                    ? "No installed voice supports this language"
                    : speechAvailability[state.result.targetLanguage] === undefined
                      ? "Checking installed voice availability"
                      : undefined
                }
                onClick={() => onSpeak(state.result.translatedText, state.result.targetLanguage)}
              >
                <SpeakerIcon />
              </button>
            </div>
            <p lang={state.result.targetLanguage}>{state.result.translatedText}</p>
          </section>
        </div>
      )}

      {state.mode === "error-visible" && (
        <div className="error-state" role="alert">
          <span className="error-state__mark" aria-hidden="true">!</span>
          <div>
            <h2>{errorCopy[state.error.code].title}</h2>
            <p>{state.error.message}</p>
            <p className="error-state__guidance">{errorCopy[state.error.code].guidance}</p>
          </div>
          {state.error.retryable && (
            <button className="button button--secondary" type="button" onClick={() => onTranslate(request())}>
              Retry Translation
            </button>
          )}
        </div>
      )}
    </aside>
  );
}
