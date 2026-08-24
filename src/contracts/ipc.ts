/** BCP-47-style language identifier accepted by the translation provider. */
export type LanguageCode = string;
/** User-selectable application color theme. */
export type Theme = "system" | "light" | "dark";

/** Rectangle expressed in global physical screen pixels. */
export interface PhysicalRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Immutable, monotonic snapshot of one eligible native text selection. */
export interface SelectionSnapshot {
  id: number;
  text: string;
  sourceApplicationId?: string;
  boundsPhysicalPx: PhysicalRect[];
  anchorPhysicalPx: PhysicalRect;
  capturedAtEpochMs: number;
}

/** Schema-versioned, non-secret user preferences persisted by the core. */
export interface UserSettings {
  schemaVersion: 1;
  enabled: boolean;
  sourceLanguage: "auto" | LanguageCode;
  targetLanguage: LanguageCode;
  startAtLogin: boolean;
  theme: Theme;
  maxSelectionCodePoints: number;
}

/** Validated translation command payload sent from the overlay to the core. */
export interface TranslationRequest {
  selectionId: number;
  text: string;
  sourceLanguage: "auto" | LanguageCode;
  targetLanguage: LanguageCode;
}

/** Provider-independent translation response correlated to one selection. */
export interface TranslationResult {
  selectionId: number;
  translatedText: string;
  detectedSourceLanguage?: LanguageCode;
  effectiveSourceLanguage: LanguageCode;
  targetLanguage: LanguageCode;
  partOfSpeech?: string;
}

export interface VocabularyEntry {
  id: number;
  sourceText: string;
  translatedText: string;
  requestedSourceLanguage: LanguageCode;
  effectiveSourceLanguage: LanguageCode;
  targetLanguage: LanguageCode;
  partOfSpeech?: string;
  lookupCount: number;
  recallScore: number;
  effectiveRecall: number;
  familiarityLevel: number;
  reviewCount: number;
  correctCount: number;
  wrongCount: number;
  correctStreak: number;
  wrongStreak: number;
  lastSeenEpochMs: number;
  lastReviewedEpochMs?: number;
}

export type VocabularyRevisionKind =
  | "added"
  | "updated"
  | "deleted"
  | "language-corrected"
  | "practice-reviewed"
  | "activated";

export interface VocabularyRevision {
  revision: number;
  kind: VocabularyRevisionKind;
  entryId?: number;
}

export interface RelatedVocabulary {
  entry: VocabularyEntry;
  reason: "root" | "meaning";
}

export interface PracticeQuestion {
  entryId: number;
  sourceText: string;
  effectiveSourceLanguage: LanguageCode;
  targetLanguage: LanguageCode;
  choices: string[];
}

export interface PracticeOutcome {
  correct: boolean;
  correctTranslation: string;
  entry: VocabularyEntry;
}

/** One pinned, app-curated textbook artifact offered for native installation. */
export interface TextbookCatalogItem {
  id: string;
  title: string;
  description?: string;
  scope?: string;
  script?: string;
  estimatedEntryCount?: number;
  sourceLanguage: LanguageCode;
  targetLanguage: LanguageCode;
  version: string;
  downloadUrl: string;
  expectedBytes: number;
  sha256: string;
  license: string;
  attribution: string;
  sourceUrl: string;
}

/** Installed textbook metadata safe to render without exposing local paths. */
export interface InstalledTextbook {
  id: string;
  title: string;
  sourceLanguage: LanguageCode;
  targetLanguage: LanguageCode;
  version: string;
  license: string;
  attribution: string;
  sourceUrl: string;
  entryCount: number;
  installedAtEpochMs: number;
  active: boolean;
}

/** One normalized entry imported from a validated textbook artifact. */
export interface TextbookEntry {
  id: number;
  textbookId: string;
  sourceText: string;
  translatedText: string;
  phoneticSymbols?: string;
  partOfSpeech?: string;
  sourceLanguage: LanguageCode;
  targetLanguage: LanguageCode;
}

/** Bounded result page for textbook browsing and search. */
export interface TextbookEntryPage {
  entries: TextbookEntry[];
  total: number;
  offset: number;
  limit: number;
}

export interface TextbookPromotionResult {
  vocabularyEntryId: number;
  inserted: boolean;
}

/** Attribution retained when a downloaded textbook entry joins the personal wordbook. */
export interface VocabularyProvenance {
  textbookId: string;
  textbookTitle: string;
  textbookVersion: string;
  license: string;
  attribution: string;
  sourceUrl: string;
  sourceText: string;
  translatedText: string;
  promotedAtEpochMs: number;
}

export interface RelatedWord {
  kind: "personal" | "textbook";
  vocabularyEntryId?: number;
  textbookEntryId?: number;
  textbookId?: string;
  sourceText: string;
  translatedText: string;
  sourceLanguage: LanguageCode;
  targetLanguage: LanguageCode;
  partOfSpeech?: string;
  reason: "root" | "meaning";
  promoted: boolean;
  origins: RelatedOrigin[];
}

export interface RelatedOrigin {
  kind: "personal" | "textbook";
  textbookId?: string;
  textbookTitle?: string;
}

export type PracticeDirection =
  | "random"
  | "source-to-target"
  | "target-to-source";

export interface PracticePreferences {
  direction: PracticeDirection;
}

export interface StudyPracticeQuestion {
  entryId: number;
  direction: Exclude<PracticeDirection, "random">;
  prompt: string;
  promptLanguage: LanguageCode;
  answerLanguage: LanguageCode;
  promptPartOfSpeech?: string;
  choices: StudyPracticeChoice[];
}

export interface StudyPracticeChoice {
  text: string;
  partOfSpeech?: string;
}

export interface StudyPracticeOutcome {
  correct: boolean;
  correctAnswer: string;
  direction: Exclude<PracticeDirection, "random">;
  entry: VocabularyEntry;
}

/** Stable error codes safe to expose across IPC without provider details. */
export const appErrorCodes = [
  "permission-denied",
  "unsupported-control",
  "no-selection",
  "missing-credential",
  "invalid-credential",
  "api-restricted",
  "billing-required",
  "quota-exceeded",
  "offline",
  "timeout",
  "service-unavailable",
  "invalid-language-pair",
  "internal",
] as const;

export type AppErrorCode = (typeof appErrorCodes)[number];

/** Stable user-facing error envelope returned by native commands. */
export interface AppError {
  code: AppErrorCode;
  message: string;
  retryable: boolean;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNonNegativeInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && Number(value) >= 0;
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function isPhysicalRect(value: unknown): value is PhysicalRect {
  return (
    isRecord(value) &&
    [value.x, value.y, value.width, value.height].every(
      (part) => typeof part === "number" && Number.isFinite(part),
    ) &&
    Number(value.width) > 0 &&
    Number(value.height) > 0
  );
}

function isHttpsUrl(value: unknown): value is string {
  if (!isNonEmptyString(value)) return false;
  try {
    return new URL(value).protocol === "https:";
  } catch {
    return false;
  }
}

function isSha256(value: unknown): value is string {
  return typeof value === "string" && /^[a-f0-9]{64}$/.test(value);
}

/** Validates the native invalidation signal used by all open study windows. */
export function isVocabularyRevision(value: unknown): value is VocabularyRevision {
  return (
    isRecord(value) &&
    Number.isSafeInteger(value.revision) &&
    Number(value.revision) > 0 &&
    ["added", "updated", "deleted", "language-corrected", "practice-reviewed", "activated"].includes(
      String(value.kind),
    ) &&
    (value.entryId === undefined || Number.isSafeInteger(value.entryId))
  );
}

/** Validates an unknown value as a selection snapshot at the IPC boundary. */
export function isSelectionSnapshot(value: unknown): value is SelectionSnapshot {
  return (
    isRecord(value) &&
    isNonNegativeInteger(value.id) &&
    isNonEmptyString(value.text) &&
    Array.isArray(value.boundsPhysicalPx) &&
    value.boundsPhysicalPx.length > 0 &&
    value.boundsPhysicalPx.every(isPhysicalRect) &&
    isPhysicalRect(value.anchorPhysicalPx) &&
    isNonNegativeInteger(value.capturedAtEpochMs) &&
    (value.sourceApplicationId === undefined || typeof value.sourceApplicationId === "string")
  );
}

/** Validates persisted or IPC-provided settings. */
export function isUserSettings(value: unknown): value is UserSettings {
  return (
    isRecord(value) &&
    value.schemaVersion === 1 &&
    typeof value.enabled === "boolean" &&
    isNonEmptyString(value.sourceLanguage) &&
    isNonEmptyString(value.targetLanguage) &&
    typeof value.startAtLogin === "boolean" &&
    (value.theme === "system" || value.theme === "light" || value.theme === "dark") &&
    Number.isSafeInteger(value.maxSelectionCodePoints) &&
    Number(value.maxSelectionCodePoints) > 0
  );
}

/** Validates a translation command before invoking native code. */
export function isTranslationRequest(value: unknown): value is TranslationRequest {
  return (
    isRecord(value) &&
    isNonNegativeInteger(value.selectionId) &&
    isNonEmptyString(value.text) &&
    isNonEmptyString(value.sourceLanguage) &&
    isNonEmptyString(value.targetLanguage)
  );
}

/** Validates a translation response before rendering it. */
export function isTranslationResult(value: unknown): value is TranslationResult {
  return (
    isRecord(value) &&
    isNonNegativeInteger(value.selectionId) &&
    isNonEmptyString(value.translatedText) &&
    isNonEmptyString(value.effectiveSourceLanguage) &&
    isNonEmptyString(value.targetLanguage) &&
    (value.detectedSourceLanguage === undefined ||
      isNonEmptyString(value.detectedSourceLanguage)) &&
    (value.partOfSpeech === undefined || isNonEmptyString(value.partOfSpeech))
  );
}

/** Validates pinned textbook metadata before it is shown by the renderer. */
export function isTextbookCatalogItem(value: unknown): value is TextbookCatalogItem {
  return (
    isRecord(value) &&
    [
      value.id,
      value.title,
      value.sourceLanguage,
      value.targetLanguage,
      value.version,
      value.license,
      value.attribution,
    ].every(isNonEmptyString) &&
    isHttpsUrl(value.downloadUrl) &&
    isHttpsUrl(value.sourceUrl) &&
    isNonNegativeInteger(value.expectedBytes) &&
    Number(value.expectedBytes) > 0 &&
    (value.description === undefined || isNonEmptyString(value.description)) &&
    (value.scope === undefined || isNonEmptyString(value.scope)) &&
    (value.script === undefined || isNonEmptyString(value.script)) &&
    (value.estimatedEntryCount === undefined ||
      (isNonNegativeInteger(value.estimatedEntryCount) &&
        Number(value.estimatedEntryCount) > 0)) &&
    isSha256(value.sha256)
  );
}

/** Validates installed textbook metadata returned by native storage. */
export function isInstalledTextbook(value: unknown): value is InstalledTextbook {
  return (
    isRecord(value) &&
    [
      value.id,
      value.title,
      value.sourceLanguage,
      value.targetLanguage,
      value.version,
      value.license,
      value.attribution,
    ].every(isNonEmptyString) &&
    isHttpsUrl(value.sourceUrl) &&
    isNonNegativeInteger(value.entryCount) &&
    isNonNegativeInteger(value.installedAtEpochMs) &&
    typeof value.active === "boolean"
  );
}

/** Validates one normalized textbook entry returned by native storage. */
export function isTextbookEntry(value: unknown): value is TextbookEntry {
  return (
    isRecord(value) &&
    isNonNegativeInteger(value.id) &&
    [
      value.textbookId,
      value.sourceText,
      value.translatedText,
      value.sourceLanguage,
      value.targetLanguage,
    ].every(isNonEmptyString) &&
    (value.phoneticSymbols === undefined || isNonEmptyString(value.phoneticSymbols)) &&
    (value.partOfSpeech === undefined || isNonEmptyString(value.partOfSpeech))
  );
}

/** Validates bounded textbook browse/search output. */
export function isTextbookEntryPage(value: unknown): value is TextbookEntryPage {
  return (
    isRecord(value) &&
    Array.isArray(value.entries) &&
    value.entries.every(isTextbookEntry) &&
    isNonNegativeInteger(value.total) &&
    isNonNegativeInteger(value.offset) &&
    Number.isSafeInteger(value.limit) &&
    Number(value.limit) > 0 &&
    Number(value.limit) <= 500 &&
    value.entries.length <= Number(value.limit)
  );
}

/** Validates the stable provider-independent error envelope. */
export function isAppError(value: unknown): value is AppError {
  return (
    isRecord(value) &&
    typeof value.code === "string" &&
    appErrorCodes.includes(value.code as AppErrorCode) &&
    isNonEmptyString(value.message) &&
    typeof value.retryable === "boolean"
  );
}
