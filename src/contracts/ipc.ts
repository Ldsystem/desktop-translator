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
      isNonEmptyString(value.detectedSourceLanguage))
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
