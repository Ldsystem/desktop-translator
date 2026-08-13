import { describe, expect, it } from "vitest";

import fixtures from "./fixtures.json";
import {
  isAppError,
  isSelectionSnapshot,
  isTranslationRequest,
  isTranslationResult,
  isUserSettings,
} from "./ipc";

describe("IPC contracts", () => {
  it("accepts every shared fixture", () => {
    expect(isSelectionSnapshot(fixtures.selection)).toBe(true);
    expect(isUserSettings(fixtures.settings)).toBe(true);
    expect(isTranslationRequest(fixtures.translationRequest)).toBe(true);
    expect(isTranslationResult(fixtures.translationResult)).toBe(true);
    expect(isAppError(fixtures.error)).toBe(true);
    expect(fixtures.errors.every(isAppError)).toBe(true);
  });

  it("rejects malformed or unsafe values", () => {
    expect(isSelectionSnapshot({ ...fixtures.selection, text: "" })).toBe(false);
    expect(isUserSettings({ ...fixtures.settings, maxSelectionCodePoints: 0 })).toBe(false);
    expect(isTranslationRequest({ ...fixtures.translationRequest, selectionId: -1 })).toBe(false);
    expect(
      isTranslationRequest({
        ...fixtures.translationRequest,
        selectionId: Number.MAX_SAFE_INTEGER + 1,
      }),
    ).toBe(false);
    expect(
      isTranslationResult({
        ...fixtures.translationResult,
        detectedSourceLanguage: "",
      }),
    ).toBe(false);
    expect(isAppError({ ...fixtures.error, code: "raw-provider-error" })).toBe(false);
  });
});
