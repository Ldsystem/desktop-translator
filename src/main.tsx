import { StrictMode, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import App from "./app/App";
import { getWindowMode } from "./app/windowMode";
import type {
  AppError,
  SelectionSnapshot,
  TranslationRequest,
  TranslationResult,
  UserSettings,
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
  schemaVersion: 1,
  enabled: true,
  sourceLanguage: "auto",
  targetLanguage: "en",
  startAtLogin: false,
  theme: "system",
  maxSelectionCodePoints: 5_000,
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
  const speaking = useRef(false);

  useEffect(() => {
    if (!runningInTauri()) {
      setSettings(fallbackSettings);
      return;
    }

    let disposed = false;
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
      invoke<PermissionStatus>("get_permission_status"),
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

    return () => {
      disposed = true;
      void subscription?.then((unlisten) => unlisten());
    };
  }, [mode]);

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
          })
          .catch(async () => {
            const status = await invoke<PermissionStatus>("get_permission_status").catch(
              () => "unknown" as const,
            );
            setPermissionStatus(status);
          });
      }}
      onSaveCredential={() => {
        void invoke<boolean>("prompt_and_save_credential").then((saved) => {
          if (saved) {
            setCredentialStatus("ready");
          }
        });
      }}
      onTestCredential={() => {
        setCredentialStatus("testing");
        void invoke("test_credential")
          .then(() => setCredentialStatus("ready"))
          .catch(() => setCredentialStatus("invalid"));
      }}
      onRemoveCredential={() => {
        void invoke("remove_credential").then(() =>
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
