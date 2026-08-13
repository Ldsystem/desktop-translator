import { useState } from "react";

import type { LanguageCode, TranslationRequest, UserSettings } from "../contracts/ipc";
import { ContextualOverlay } from "../components/context/ContextualOverlay";
import { SettingsPanel } from "../components/settings/SettingsPanel";
import {
  initialOverlayState,
  type OverlayState,
} from "../state/overlayMachine";
import "../styles/tokens.css";
import "../styles/app.css";
import { getWindowMode, type WindowMode } from "./windowMode";

const defaultSettings: UserSettings = {
  schemaVersion: 1,
  enabled: true,
  sourceLanguage: "auto",
  targetLanguage: "en",
  startAtLogin: false,
  theme: "system",
  maxSelectionCodePoints: 5_000,
};

const ignore = () => undefined;

export interface AppProps {
  mode?: WindowMode;
  overlayState?: OverlayState;
  initialSettings?: UserSettings;
  credentialStatus?: "missing" | "ready" | "invalid" | "testing";
  permissionStatus?: "unknown" | "granted" | "denied";
  speechAvailability?: Readonly<Record<string, boolean>>;
  onTranslate?: (request: TranslationRequest) => void;
  onCorrectSource?: (request: TranslationRequest) => void;
  onSpeak?: (text: string, language: LanguageCode) => void;
  onDismiss?: () => void;
  onSaveSettings?: (settings: UserSettings) => void;
  onSaveCredential?: () => void;
  onTestCredential?: () => void;
  onRemoveCredential?: () => void;
  onOpenSystemSettings?: () => void;
  onQuit?: () => void;
}

export default function App({
  mode = getWindowMode(),
  overlayState = initialOverlayState,
  initialSettings = defaultSettings,
  credentialStatus = "missing",
  permissionStatus = "unknown",
  speechAvailability = {},
  onTranslate = ignore,
  onCorrectSource = ignore,
  onSpeak = ignore,
  onDismiss = ignore,
  onSaveSettings = ignore,
  onSaveCredential = ignore,
  onTestCredential = ignore,
  onRemoveCredential = ignore,
  onOpenSystemSettings = ignore,
  onQuit = ignore,
}: AppProps) {
  const [settings, setSettings] = useState(initialSettings);

  if (mode === "overlay") {
    return (
      <div className="app-surface app-surface--overlay" data-theme={settings.theme}>
        <ContextualOverlay
          state={overlayState}
          sourceLanguage={settings.sourceLanguage}
          targetLanguage={settings.targetLanguage}
          speechAvailability={speechAvailability}
          onTranslate={onTranslate}
          onCorrectSource={onCorrectSource}
          onSpeak={onSpeak}
          onDismiss={onDismiss}
        />
      </div>
    );
  }

  return (
    <div className="app-surface app-surface--settings" data-theme={settings.theme}>
      <SettingsPanel
        settings={settings}
        credentialStatus={credentialStatus}
        permissionStatus={permissionStatus}
        onSave={(nextSettings) => {
          setSettings(nextSettings);
          onSaveSettings(nextSettings);
        }}
        onSaveCredential={onSaveCredential}
        onTestCredential={onTestCredential}
        onRemoveCredential={onRemoveCredential}
        onOpenSystemSettings={onOpenSystemSettings}
        onQuit={onQuit}
      />
    </div>
  );
}
