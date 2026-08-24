import { describe, expect, it } from "vitest";

import fixtures from "./fixtures.json";
import type { StudyPracticeQuestion } from "./ipc";
import {
  isAppError,
  isSelectionSnapshot,
  isTranslationRequest,
  isTranslationResult,
  isStudyPracticeQuestion,
  isTextbookCatalogItem,
  isTextbookEntryPage,
  isInstalledTextbook,
  isUserSettings,
} from "./ipc";

describe("IPC contracts", () => {
  it("accepts every shared fixture", () => {
    expect(isSelectionSnapshot(fixtures.selection)).toBe(true);
    expect(isUserSettings(fixtures.settings)).toBe(true);
    expect(isTranslationRequest(fixtures.translationRequest)).toBe(true);
    expect(isTranslationResult(fixtures.translationResult)).toBe(true);
    expect(isStudyPracticeQuestion(fixtures.studyPracticeQuestion)).toBe(true);
    expect(
      JSON.parse(JSON.stringify(fixtures.studyPracticeQuestion)),
    ).toEqual(fixtures.studyPracticeQuestion);
    expect(isAppError(fixtures.error)).toBe(true);
    expect(fixtures.errors.every(isAppError)).toBe(true);
  });

  it("accepts safe textbook catalog, installed-book, and entry-page DTOs", () => {
    expect(
      isTextbookCatalogItem({
        id: "wikdict-en-zh-2026-06",
        title: "WikDict English - Chinese",
        sourceLanguage: "en",
        targetLanguage: "zh-CN",
        version: "2_2026-06",
        downloadUrl:
          "https://download.wikdict.com/dictionaries/sqlite/2_2026-06/en-zh.sqlite3",
        expectedBytes: 5169152,
        sha256: "16cf69dc8037a8d4dc6bde260142bf0181f9ff0a008d457f26452f1d80ca5ecd",
        license: "CC BY-SA 4.0",
        attribution: "WikDict, Wiktionary and DBnary contributors",
        sourceUrl: "https://www.wikdict.com/page/download",
      }),
    ).toBe(true);
    expect(
      isInstalledTextbook({
        id: "wikdict-en-zh-2026-06",
        title: "WikDict English - Chinese",
        sourceLanguage: "en",
        targetLanguage: "zh-CN",
        version: "2_2026-06",
        license: "CC BY-SA 4.0",
        attribution: "WikDict, Wiktionary and DBnary contributors",
        sourceUrl: "https://www.wikdict.com/page/download",
        entryCount: 100,
        installedAtEpochMs: 42,
        active: true,
      }),
    ).toBe(true);
    expect(
      isTextbookEntryPage({
        entries: [
          {
            id: 1,
            textbookId: "wikdict-en-zh-2026-06",
            sourceText: "ephemeral",
            translatedText: "蜉蝣",
            sourceLanguage: "en",
            targetLanguage: "zh-CN",
          },
        ],
        total: 1,
        offset: 0,
        limit: 50,
      }),
    ).toBe(true);
  });

  it("rejects unsafe textbook DTO fields and unbounded pages", () => {
    expect(
      isTextbookCatalogItem({
        id: "bad",
        title: "Bad",
        sourceLanguage: "en",
        targetLanguage: "zh-CN",
        version: "1",
        downloadUrl: "http://example.com/deck.sqlite3",
        expectedBytes: 10,
        sha256: "not-a-digest",
        license: "unknown",
        attribution: "unknown",
        sourceUrl: "https://example.com",
      }),
    ).toBe(false);
    expect(
      isTextbookEntryPage({ entries: [], total: 0, offset: 0, limit: 501 }),
    ).toBe(false);
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
    expect(
      isTranslationResult({ ...fixtures.translationResult, partOfSpeech: "oracle" }),
    ).toBe(false);
    expect(
      isStudyPracticeQuestion({
        ...fixtures.studyPracticeQuestion,
        choices: [{ text: "短暂的", partOfSpeech: "adjective" }],
      }),
    ).toBe(false);
    expect(
      isStudyPracticeQuestion({
        ...fixtures.studyPracticeQuestion,
        promptPartOfSpeech: "oracle",
      }),
    ).toBe(false);
  });
});

const fixturePromptCategory: StudyPracticeQuestion["promptPartOfSpeech"] =
  fixtures.studyPracticeQuestion.promptPartOfSpeech as "adjective";
void fixturePromptCategory;
// @ts-expect-error unknown prompt categories must be represented by absence
const invalidPromptCategory: StudyPracticeQuestion["promptPartOfSpeech"] = "oracle";
void invalidPromptCategory;
