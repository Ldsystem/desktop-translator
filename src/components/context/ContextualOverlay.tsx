import { useEffect } from "react";

import type { LanguageCode, TranslationRequest, UiLocale } from "../../contracts/ipc";
import type { OverlayState } from "../../state/overlayMachine";
import { errorCopy } from "../errorCopy";

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
  locale?: UiLocale;
  sourceLanguage: "auto" | LanguageCode;
  targetLanguage: LanguageCode;
  languages?: readonly LanguageOption[];
  speechAvailability?: Readonly<Record<string, boolean>>;
  onTranslate: (request: TranslationRequest) => void;
  onCorrectSource: (request: TranslationRequest) => void;
  onSpeak: (text: string, language: LanguageCode) => void;
  onDismiss: () => void;
}


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
  locale = "en",
  sourceLanguage,
  targetLanguage,
  languages = defaultLanguages,
  speechAvailability = {},
  onTranslate,
  onCorrectSource,
  onSpeak,
  onDismiss,
}: ContextualOverlayProps) {
  const zh = locale === "zh-CN";
  const errors = errorCopy(locale);
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
      <aside className="contextual-trigger" aria-label={zh ? "翻译控制" : "Translation Controls"}>
        <button
          className="translate-trigger"
          type="button"
          aria-label={zh ? "翻译所选文本" : "Translate Selected Text"}
          onClick={() => onTranslate(request())}
        >
          <span aria-hidden="true">A</span>
          <span className="translate-trigger__swap" aria-hidden="true">文</span>
        </button>
      </aside>
    );
  }

  return (
    <aside className="contextual-card" aria-label={zh ? "翻译" : "Translation"}>
      <div className="contextual-card__rail" aria-hidden="true" />
      <button className="icon-button dismiss-button" type="button" aria-label={zh ? "关闭" : "Dismiss"} onClick={onDismiss}>
        <CloseIcon />
      </button>

      {state.mode === "translating" && (
        <div className="loading-state" role="status" aria-live="polite">
          <span className="loading-mark" aria-hidden="true">
            <span />
            <span />
            <span />
          </span>
          <span>{zh ? "正在翻译…" : "Translating…"}</span>
        </div>
      )}

      {state.mode === "result-visible" && (
        <div className="translation-result" aria-live="polite">
          <section className="translation-block translation-block--source" aria-labelledby="source-heading">
            <div className="translation-block__header">
              <label id="source-heading" htmlFor="overlay-source-language">
                {zh ? "源文本" : "Source"}
              </label>
              <select
                id="overlay-source-language"
                name="overlay-source-language"
                value={state.result.effectiveSourceLanguage}
                onChange={(event) => onCorrectSource(request(event.currentTarget.value))}
                aria-label={zh ? "更正源语言" : "Correct Source Language"}
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
                aria-label={zh ? "朗读源文本" : "Speak Source Text"}
                disabled={speechAvailability[state.result.effectiveSourceLanguage] !== true}
                title={
                  speechAvailability[state.result.effectiveSourceLanguage] === false
                    ? (zh ? "没有已安装的语音支持此语言" : "No installed voice supports this language")
                    : speechAvailability[state.result.effectiveSourceLanguage] === undefined
                      ? (zh ? "正在检查已安装语音" : "Checking installed voice availability")
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
                aria-label={zh ? "朗读译文" : "Speak Translation"}
                disabled={speechAvailability[state.result.targetLanguage] !== true}
                title={
                  speechAvailability[state.result.targetLanguage] === false
                    ? (zh ? "没有已安装的语音支持此语言" : "No installed voice supports this language")
                    : speechAvailability[state.result.targetLanguage] === undefined
                      ? (zh ? "正在检查已安装语音" : "Checking installed voice availability")
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
            <h2>{errors[state.error.code].title}</h2>
            {!zh && <p>{state.error.message}</p>}
            <p className="error-state__guidance">{errors[state.error.code].guidance}</p>
          </div>
          {state.error.retryable && (
            <button className="button button--secondary" type="button" onClick={() => onTranslate(request())}>
              {zh ? "重试翻译" : "Retry Translation"}
            </button>
          )}
        </div>
      )}
    </aside>
  );
}
