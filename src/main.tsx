import { StrictMode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import App from "./app/App";
import { createPronouncer } from "./app/pronunciation";
import { getWindowMode } from "./app/windowMode";
import type { QuickTranslateStatus } from "./components/quick/QuickTranslatePanel";
import type { StudyApi } from "./components/vocabulary/VocabularyWindow";
import type {
  AppError,
  InstalledTextbook,
  PracticePreferences,
  PracticeOutcome,
  PracticeQuestion,
  RelatedWord,
  RelatedVocabulary,
  SelectionSnapshot,
  StudyPracticeOutcome,
  StudyPracticeQuestion,
  TextbookCatalogItem,
  TextbookEntryPage,
  TextbookPromotionResult,
  TranslationRequest,
  TranslationResult,
  UserSettings,
  VocabularyEntry,
  VocabularyProvenance,
  VocabularyRevision,
} from "./contracts/ipc";
import {
  initialOverlayState,
  reduceOverlayState,
  type OverlayState,
} from "./state/overlayMachine";

type CredentialStatus = "missing" | "ready" | "invalid" | "testing";
type PermissionStatus = "unknown" | "granted" | "denied";

interface SelectionEvent {
  requestId: number;
  selection: SelectionSnapshot;
}

const fallbackSettings: UserSettings = {
  schemaVersion: 2,
  enabled: true,
  sourceLanguage: "auto",
  targetLanguage: "en",
  startAtLogin: false,
  theme: "system",
  maxSelectionCodePoints: 5_000,
  uiLocale: "en",
  translationProvider: "google",
  microsoftCloud: "global",
};

function runningInTauri() {
  return "__TAURI_INTERNALS__" in window;
}

function normalizeAppError(error: unknown): AppError {
  if (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    "message" in error &&
    "retryable" in error
  ) {
    return error as AppError;
  }
  return {
    code: "internal",
    message: "The native service could not complete the request.",
    retryable: false,
  };
}

export function createStudyApi(refreshPersonal: () => void): StudyApi {
  return {
    listCatalog: () => invoke<TextbookCatalogItem[]>("list_textbook_catalog"),
    listDownloaded: () => invoke<InstalledTextbook[]>("list_downloaded_textbooks"),
    downloadTextbook: (textbookId) => invoke<InstalledTextbook>("download_textbook", { textbookId }),
    setActiveTextbook: (textbookId) => invoke<void>("set_active_textbook", { textbookId: textbookId ?? null }),
    removeTextbook: (textbookId) => invoke<void>("remove_downloaded_textbook", { textbookId }),
    listTextbookEntries: (textbookId, search, offset, limit) => invoke<TextbookEntryPage>("list_textbook_entries", { textbookId, search: search || null, offset, limit }),
    addTextbookEntry: (textbookEntryId) => invoke<TextbookPromotionResult>("add_textbook_entry_to_personal", { textbookEntryId }),
    listVocabularyProvenance: (entryId) => invoke<VocabularyProvenance[]>("list_vocabulary_provenance", { entryId }),
    listRelated: (entryId, seed?: number) => invoke<RelatedWord[]>("get_related_vocabulary", { entryId, seed }),
    deleteVocabularyEntry: (entryId) => invoke<void>("delete_vocabulary_entry", { entryId }),
    correctVocabularySourceLanguage: (entryId, sourceLanguage) => invoke<VocabularyEntry>("correct_vocabulary_source_language", { entryId, effectiveSourceLanguage: sourceLanguage }),
    getPracticePreferences: () => invoke<PracticePreferences>("get_practice_preferences"),
    savePracticePreferences: (preferences) => invoke<void>("save_practice_preferences", { preferences }),
    getPracticeQuestion: () => invoke<StudyPracticeQuestion | null>("get_practice_question"),
    submitPracticeAnswer: (entryId, direction, selectedAnswer) => invoke<StudyPracticeOutcome>("submit_practice_answer", { entryId, direction, selectedAnswer }),
    refreshPersonal,
  };
}

export function Bootstrap() {
  const mode = getWindowMode();
  const [settings, setSettings] = useState<UserSettings>();
  const [overlayState, setOverlayState] =
    useState<OverlayState>(initialOverlayState);
  const [credentialStatus, setCredentialStatus] =
    useState<CredentialStatus>("missing");
  const [permissionStatus, setPermissionStatus] =
    useState<PermissionStatus>("unknown");
  const [speechAvailability, setSpeechAvailability] = useState<
    Record<string, boolean>
  >({});
  const [quickStatus, setQuickStatus] = useState<QuickTranslateStatus>({
    mode: "idle",
  });
  const [vocabularyEntries, setVocabularyEntries] = useState<VocabularyEntry[]>([]);
  const [vocabularyLoading, setVocabularyLoading] = useState(mode === "study");
  const [vocabularyError, setVocabularyError] = useState<string>();
  const [vocabularyRevision, setVocabularyRevision] = useState(0);
  const vocabularySearch = useRef("");
  const vocabularyRequest = useRef(0);
  const [relatedVocabulary, setRelatedVocabulary] = useState<RelatedVocabulary[]>([]);
  const [practiceQuestion, setPracticeQuestion] = useState<PracticeQuestion | null>();
  const [practiceOutcome, setPracticeOutcome] = useState<PracticeOutcome>();
  const speaking = useRef(false);
  const vocabularyPronouncer = useRef(createPronouncer({
    stop: () => invoke("stop_speech"),
    speak: (text, language) => invoke("speak_text", { text, language }),
    onError: () => setVocabularyError("Pronunciation could not be played with the installed voice."),
  }));

  useEffect(() => {
    if (!runningInTauri()) {
      setSettings(fallbackSettings);
      return;
    }

    let disposed = false;
    const settingsSubscription = listen<UserSettings>("settings-changed", ({ payload }) => {
      if (!disposed) setSettings(payload);
    });
    const subscription =
      mode === "overlay"
        ? listen<SelectionEvent>("selection-resolved", ({ payload }) => {
            setOverlayState((state) => {
              const resolving = reduceOverlayState(state, {
                type: "pointer-up",
                requestId: payload.requestId,
              });
              return reduceOverlayState(resolving, {
                type: "selection-resolved",
                requestId: payload.requestId,
                selection: payload.selection,
              });
            });
          }).then((unlisten) => {
            if (!disposed) {
              void invoke("overlay_ready");
            }
            return unlisten;
          })
        : undefined;

    Promise.all([
      invoke<UserSettings>("get_settings"),
      invoke<CredentialStatus>("get_credential_status"),
      invoke<PermissionStatus>("sync_permission"),
    ])
      .then(([loadedSettings, loadedCredentialStatus, loadedPermissionStatus]) => {
        if (!disposed) {
          setSettings(loadedSettings);
          setCredentialStatus(loadedCredentialStatus);
          setPermissionStatus(loadedPermissionStatus);
        }
      })
      .catch(() => {
        if (!disposed) {
          setSettings(fallbackSettings);
        }
      });

    const permissionPoll =
      mode === "settings"
        ? window.setInterval(() => {
            void invoke<PermissionStatus>("sync_permission")
              .then((status) => {
                if (!disposed) {
                  setPermissionStatus(status);
                }
              })
              .catch(() => undefined);
          }, 2000)
        : undefined;

    return () => {
      disposed = true;
      if (permissionPoll !== undefined) {
        window.clearInterval(permissionPoll);
      }
      void subscription?.then((unlisten) => unlisten());
      void settingsSubscription.then((unlisten) => unlisten());
    };
  }, [mode]);

  const loadVocabulary = useCallback((search?: string) => {
    if (search !== undefined) vocabularySearch.current = search;
    const request = ++vocabularyRequest.current;
    if (!runningInTauri()) {
      setVocabularyLoading(false);
      return;
    }
    setVocabularyLoading(true);
    setVocabularyError(undefined);
    void invoke<VocabularyEntry[]>("list_vocabulary", { search: vocabularySearch.current || null })
      .then((entries) => { if (request === vocabularyRequest.current) setVocabularyEntries(entries); })
      .catch(() => { if (request === vocabularyRequest.current) setVocabularyError("Your local wordbook could not be opened."); })
      .finally(() => { if (request === vocabularyRequest.current) setVocabularyLoading(false); });
  }, []);

  const studyApi = useMemo(() => createStudyApi(() => loadVocabulary()), [loadVocabulary]);

  useEffect(() => {
    if (mode === "study") loadVocabulary();
  }, [mode]);

  useEffect(() => {
    if (!runningInTauri() || mode !== "study") return;
    let disposed = false;
    const refresh = () => { setVocabularyRevision((current) => current + 1); loadVocabulary(); };
    const subscription = listen<VocabularyRevision>("vocabulary-revision", ({ payload }) => {
      if (disposed) return;
      setVocabularyRevision(payload.revision);
      loadVocabulary();
    });
    const onFocus = () => refresh();
    const onVisibility = () => { if (document.visibilityState === "visible") refresh(); };
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      disposed = true;
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onVisibility);
      void subscription.then((unlisten) => unlisten());
    };
  }, [mode, loadVocabulary]);

  useEffect(() => {
    if (!runningInTauri() || mode !== "study" || vocabularyEntries.length === 0) {
      return;
    }

    let disposed = false;
    const languages = [...new Set(vocabularyEntries.flatMap((entry) => [entry.effectiveSourceLanguage, entry.targetLanguage]))];
    void Promise.all(
      languages.map(async (language) => [
        language,
        await invoke<boolean>("get_speech_availability", { language }).catch(() => false),
      ] as const),
    ).then((entries) => {
      if (!disposed) {
        setSpeechAvailability((current) => ({ ...current, ...Object.fromEntries(entries) }));
      }
    });

    return () => { disposed = true; };
  }, [mode, vocabularyEntries]);

  useEffect(() => {
    if (
      !runningInTauri() ||
      mode !== "overlay" ||
      overlayState.mode !== "result-visible"
    ) {
      return;
    }

    let disposed = false;
    const languages = [
      overlayState.result.effectiveSourceLanguage,
      overlayState.result.targetLanguage,
    ];
    void Promise.all(
      [...new Set(languages)].map(async (language) => [
        language,
        await invoke<boolean>("get_speech_availability", { language }).catch(
          () => false,
        ),
      ] as const),
    ).then((entries) => {
      if (!disposed) {
        setSpeechAvailability((current) => ({
          ...current,
          ...Object.fromEntries(entries),
        }));
      }
    });

    return () => {
      disposed = true;
    };
  }, [mode, overlayState]);

  useEffect(() => {
    if (!runningInTauri() || mode !== "quick") {
      return;
    }

    // Each reopen of the tray panel starts from a clean result area.
    const subscription = listen("quick-translate-opened", () =>
      setQuickStatus({ mode: "idle" }),
    );

    return () => {
      void subscription.then((unlisten) => unlisten());
    };
  }, [mode]);

  useEffect(() => {
    if (!runningInTauri() || mode !== "quick" || quickStatus.mode !== "result") {
      return;
    }

    let disposed = false;
    const language = quickStatus.result.targetLanguage;
    void invoke<boolean>("get_speech_availability", { language })
      .catch(() => false)
      .then((available) => {
        if (!disposed) {
          setSpeechAvailability((current) => ({ ...current, [language]: available }));
        }
      });

    return () => {
      disposed = true;
    };
  }, [mode, quickStatus]);

  const quickTranslate = (request: TranslationRequest) => {
    setQuickStatus({ mode: "translating" });
    void invoke<TranslationResult>("translate_input", { request })
      .then((result) => setQuickStatus({ mode: "result", result }))
      .catch((error: unknown) =>
        setQuickStatus({ mode: "error", error: normalizeAppError(error) }),
      );
  };

  const translate = (request: TranslationRequest) => {
    setOverlayState((state) => reduceOverlayState(state, { type: "translate" }));
    void invoke<TranslationResult>("translate_selection", { request })
      .then((result) => {
        setOverlayState((state) =>
          reduceOverlayState(state, { type: "translation-resolved", result }),
        );
      })
      .catch((error: unknown) => {
        setOverlayState((state) =>
          reduceOverlayState(state, {
            type: "translation-failed",
            selectionId: request.selectionId,
            error: normalizeAppError(error),
          }),
        );
      });
  };

  if (!settings) {
    return null;
  }

  return (
    <App
      key={mode}
      mode={mode}
      overlayState={overlayState}
      initialSettings={settings}
      credentialStatus={credentialStatus}
      permissionStatus={permissionStatus}
      speechAvailability={speechAvailability}
      quickStatus={quickStatus}
      vocabularyEntries={vocabularyEntries}
      vocabularyLoading={vocabularyLoading}
      vocabularyError={vocabularyError}
      vocabularyRevision={vocabularyRevision}
      relatedVocabulary={relatedVocabulary}
      practiceQuestion={practiceQuestion}
      practiceOutcome={practiceOutcome}
      studyApi={studyApi}
      onQuickTranslate={quickTranslate}
      onVocabularySearch={loadVocabulary}
      onSelectVocabulary={() => undefined}
      onStartPractice={() => {
        setPracticeQuestion(undefined);
        setPracticeOutcome(undefined);
        setVocabularyError(undefined);
        void invoke<PracticeQuestion | null>("get_practice_question")
          .then(setPracticeQuestion)
          .catch(() => {
            setPracticeQuestion(null);
            setVocabularyError("A practice question could not be prepared.");
          });
      }}
      onSubmitPracticeAnswer={(entryId, selectedTranslation) => {
        setVocabularyError(undefined);
        void invoke<PracticeOutcome>("submit_practice_answer", { entryId, selectedTranslation })
          .then((outcome) => {
            setPracticeOutcome(outcome);
            setVocabularyEntries((entries) => entries.map((entry) => entry.id === outcome.entry.id ? outcome.entry : entry));
          })
          .catch(() => setVocabularyError("Your answer could not be saved."));
      }}
      onPronounceVocabulary={(text, language) => {
        setVocabularyError(undefined);
        vocabularyPronouncer.current(text, language);
      }}
      onTranslate={translate}
      onCorrectSource={translate}
      onSpeak={(text, language) => {
        if (speaking.current) {
          speaking.current = false;
          void invoke("stop_speech");
        } else {
          speaking.current = true;
          void invoke("speak_text", { text, language }).catch(() => {
            speaking.current = false;
          });
        }
      }}
      onDismiss={() => {
        speaking.current = false;
        void invoke("stop_speech");
        setOverlayState((state) =>
          reduceOverlayState(state, { type: "dismiss" }),
        );
        void invoke("dismiss_overlay");
      }}
      onSaveSettings={(nextSettings) => {
        void invoke("save_settings", { settings: nextSettings })
          .then(() => {
            setSettings(nextSettings);
            setOverlayState((state) =>
              reduceOverlayState(state, {
                type: nextSettings.enabled ? "enable" : "disable",
              }),
            );
            if (!nextSettings.enabled) {
              speaking.current = false;
            }
            void invoke<CredentialStatus>("get_credential_status", { provider: nextSettings.translationProvider })
              .then(setCredentialStatus)
              .catch(() => setCredentialStatus("missing"));
          })
          .catch(async () => {
            const status = await invoke<PermissionStatus>("get_permission_status").catch(
              () => "unknown" as const,
            );
            setPermissionStatus(status);
          });
      }}
      onSaveCredential={(provider, field) => {
        void invoke<boolean>("prompt_and_save_credential", { provider, field }).then((saved) => {
          if (saved) {
            void invoke<CredentialStatus>("get_credential_status", { provider })
              .then(setCredentialStatus)
              .catch(() => setCredentialStatus("missing"));
          }
        });
      }}
      onProviderChange={(provider) => {
        void invoke<CredentialStatus>("get_credential_status", { provider })
          .then(setCredentialStatus)
          .catch(() => setCredentialStatus("missing"));
      }}
      onTestCredential={(provider) => {
        setCredentialStatus("testing");
        void invoke("test_credential", { provider })
          .then(() => setCredentialStatus("ready"))
          .catch(() => setCredentialStatus("invalid"));
      }}
      onRemoveCredential={(provider) => {
        void invoke("remove_credential", { provider }).then(() =>
          setCredentialStatus("missing"),
        );
      }}
      onOpenSystemSettings={() => void invoke("open_accessibility_settings")}
      onQuit={() => {
        speaking.current = false;
        void invoke("quit_application");
      }}
    />
  );
}

export function mount(root: HTMLElement) {
  createRoot(root).render(
    <StrictMode>
      <Bootstrap />
    </StrictMode>,
  );
}

const root = document.getElementById("root");

if (root) {
  mount(root);
}
