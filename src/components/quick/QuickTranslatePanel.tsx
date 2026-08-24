import { useEffect, useRef, useState } from "react";

import type {
  AppError,
  LanguageCode,
  TranslationRequest,
  TranslationResult,
  UiLocale,
} from "../../contracts/ipc";
import { defaultLanguages, type LanguageOption } from "../context/ContextualOverlay";
import { errorCopy } from "../errorCopy";

/** Lifecycle of one operator-initiated translation. */
export type QuickTranslateStatus =
  | { mode: "idle" }
  | { mode: "translating" }
  | { mode: "result"; result: TranslationResult }
  | { mode: "error"; error: AppError };

interface QuickTranslatePanelProps {
  status: QuickTranslateStatus;
  locale?: UiLocale;
  sourceLanguage: "auto" | LanguageCode;
  targetLanguage: LanguageCode;
  languages?: readonly LanguageOption[];
  speechAvailability?: Readonly<Record<string, boolean>>;
  onTranslate: (request: TranslationRequest) => void;
  onSpeak: (text: string, language: LanguageCode) => void;
}

function SpeakerIcon() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
      <path d="M4 9v6h4l5 4V5L8 9H4Zm12.5-.7a5 5 0 0 1 0 7.4M19 5a9 9 0 0 1 0 14" />
    </svg>
  );
}

export function QuickTranslatePanel({
  status,
  locale = "en",
  sourceLanguage,
  targetLanguage,
  languages = defaultLanguages,
  speechAvailability = {},
  onTranslate,
  onSpeak,
}: QuickTranslatePanelProps) {
  const zh = locale === "zh-CN";
  const errors = errorCopy(locale);
  const [text, setText] = useState("");
  const [source, setSource] = useState<"auto" | LanguageCode>(sourceLanguage);
  const [target, setTarget] = useState<LanguageCode>(targetLanguage);
  const input = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    input.current?.focus();
  }, []);

  const translating = status.mode === "translating";

  const translate = () => {
    if (text.trim().length === 0 || translating) {
      return;
    }
    // Typed text owns no native selection, so the identifier is always zero.
    onTranslate({
      selectionId: 0,
      text: text.trim(),
      sourceLanguage: source,
      targetLanguage: target,
    });
  };

  return (
    <section className="quick-panel" aria-label={zh ? "快速翻译" : "Quick Translation"}>
      <form
        className="quick-panel__form"
        onSubmit={(event) => {
          event.preventDefault();
          translate();
        }}
      >
        <div className="quick-panel__languages">
          <label className="field">
            <span>{zh ? "源语言" : "From"}</span>
            <select
              value={source}
              onChange={(event) => setSource(event.target.value as "auto" | LanguageCode)}
            >
              <option value="auto">{zh ? "自动检测" : "Detect language"}</option>
              {languages.map((language) => (
                <option key={language.code} value={language.code}>
                  {language.label}
                </option>
              ))}
            </select>
          </label>
          <span className="language-arrow" aria-hidden="true">
            →
          </span>
          <label className="field">
            <span>{zh ? "目标语言" : "To"}</span>
            <select
              value={target}
              onChange={(event) => setTarget(event.target.value as LanguageCode)}
            >
              {languages.map((language) => (
                <option key={language.code} value={language.code}>
                  {language.label}
                </option>
              ))}
            </select>
          </label>
        </div>

        <textarea
          ref={input}
          className="quick-panel__input"
          value={text}
          rows={3}
          placeholder={zh ? "输入或粘贴要翻译的文本" : "Type or paste text to translate"}
          aria-label={zh ? "待翻译文本" : "Text to Translate"}
          onChange={(event) => setText(event.target.value)}
          onKeyDown={(event) => {
            // Enter translates; Shift+Enter keeps the newline for longer text.
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              translate();
            }
          }}
        />

        <button
          className="button button--primary quick-panel__action"
          type="submit"
          disabled={translating || text.trim().length === 0}
        >
          {translating ? (zh ? "正在翻译…" : "Translating…") : (zh ? "翻译" : "Translate")}
        </button>
      </form>

      {status.mode === "result" && (
        <div className="quick-panel__result" role="status" aria-live="polite">
          <div className="translation-block__header">
            <h2>{status.result.targetLanguage}</h2>
            <button
              className="icon-button"
              type="button"
              aria-label={zh ? "朗读译文" : "Speak Translation"}
              disabled={speechAvailability[status.result.targetLanguage] === false}
              onClick={() =>
                onSpeak(status.result.translatedText, status.result.targetLanguage)
              }
            >
              <SpeakerIcon />
            </button>
          </div>
          <p>{status.result.translatedText}</p>
        </div>
      )}

      {status.mode === "error" && (
        <div className="quick-panel__error" role="alert">
          <strong>{errors[status.error.code].title}</strong>
          <span>{errors[status.error.code].guidance}</span>
        </div>
      )}
    </section>
  );
}
