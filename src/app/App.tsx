import { useState } from "react";

import type {
  LanguageCode,
  PracticeOutcome,
  PracticeQuestion,
  RelatedVocabulary,
  TranslationRequest,
  UserSettings,
  VocabularyEntry,
} from "../contracts/ipc";
import { ContextualOverlay } from "../components/context/ContextualOverlay";
import {
  QuickTranslatePanel,
  type QuickTranslateStatus,
} from "../components/quick/QuickTranslatePanel";
import { SettingsPanel } from "../components/settings/SettingsPanel";
import { VocabularyWindow } from "../components/vocabulary/VocabularyWindow";
import type { StudyApi } from "../components/vocabulary/VocabularyWindow";
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
  quickStatus?: QuickTranslateStatus;
  vocabularyEntries?: readonly VocabularyEntry[];
  vocabularyLoading?: boolean;
  vocabularyError?: string;
  relatedVocabulary?: readonly RelatedVocabulary[];
  practiceQuestion?: PracticeQuestion | null;
  practiceOutcome?: PracticeOutcome;
  studyApi?: StudyApi;
  onQuickTranslate?: (request: TranslationRequest) => void;
  onVocabularySearch?: (search: string) => void;
  onSelectVocabulary?: (entryId: number) => void;
  onStartPractice?: () => void;
  onSubmitPracticeAnswer?: (entryId: number, selectedTranslation: string) => void;
  onPronounceVocabulary?: (text: string, language: LanguageCode) => void;
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
  quickStatus = { mode: "idle" },
  vocabularyEntries = [],
  vocabularyLoading = false,
  vocabularyError,
  relatedVocabulary = [],
  practiceQuestion,
  practiceOutcome,
  studyApi,
  onQuickTranslate = ignore,
  onVocabularySearch = ignore,
  onSelectVocabulary = ignore,
  onStartPractice = ignore,
  onSubmitPracticeAnswer = ignore,
  onPronounceVocabulary = ignore,
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
    const surfaceClass =
      overlayState.mode === "button-visible"
        ? "app-surface app-surface--overlay app-surface--trigger"
        : "app-surface app-surface--overlay";
    return (
      <div className={surfaceClass} data-theme={settings.theme}>
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

  if (mode === "quick") {
    return (
      <div className="app-surface app-surface--quick" data-theme={settings.theme}>
        <QuickTranslatePanel
          status={quickStatus}
          sourceLanguage={settings.sourceLanguage}
          targetLanguage={settings.targetLanguage}
          speechAvailability={speechAvailability}
          onTranslate={onQuickTranslate}
          onSpeak={onSpeak}
        />
      </div>
    );
  }

  if (mode === "study") {
    return (
      <div className="app-surface app-surface--study" data-theme={settings.theme}>
        <VocabularyWindow
          entries={vocabularyEntries}
          loading={vocabularyLoading}
          error={vocabularyError}
          related={relatedVocabulary}
          question={practiceQuestion}
          outcome={practiceOutcome}
          studyApi={studyApi}
          speechAvailability={speechAvailability}
          onPronounce={onPronounceVocabulary}
          onSearch={onVocabularySearch}
          onSelectEntry={onSelectVocabulary}
          onStartPractice={onStartPractice}
          onSubmitAnswer={onSubmitPracticeAnswer}
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
