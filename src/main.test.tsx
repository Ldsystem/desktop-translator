import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import { createStudyApi, mount } from "./main";

describe("application bootstrap", () => {
  beforeEach(() => invoke.mockReset());

  it("mounts into the provided host", () => {
    const host = document.createElement("div");

    expect(() => mount(host)).not.toThrow();
  });

  it("maps the study renderer to the exact native command contracts", async () => {
    invoke.mockResolvedValue(undefined);
    const api = createStudyApi(vi.fn());

    await api.listCatalog();
    await api.listDownloaded();
    await api.downloadTextbook("wikdict-en-zh");
    await api.setActiveTextbook(undefined);
    await api.listTextbookEntries("wikdict-en-zh", "hello", 40, 40);
    await api.addTextbookEntry(81);
    await api.listRelated(1, 7);
    await api.deleteVocabularyEntry(1);
    await api.correctVocabularySourceLanguage(1, "en");
    await api.savePracticePreferences({ direction: "target-to-source" });
    await api.submitPracticeAnswer(1, "target-to-source", "hello");

    expect(invoke.mock.calls).toEqual([
      ["list_textbook_catalog"],
      ["list_downloaded_textbooks"],
      ["download_textbook", { textbookId: "wikdict-en-zh" }],
      ["set_active_textbook", { textbookId: null }],
      ["list_textbook_entries", { textbookId: "wikdict-en-zh", search: "hello", offset: 40, limit: 40 }],
      ["add_textbook_entry_to_personal", { textbookEntryId: 81 }],
      ["get_related_vocabulary", { entryId: 1, seed: 7 }],
      ["delete_vocabulary_entry", { entryId: 1 }],
      ["correct_vocabulary_source_language", { entryId: 1, sourceLanguage: "en" }],
      ["save_practice_preferences", { preferences: { direction: "target-to-source" } }],
      ["submit_practice_answer", { entryId: 1, direction: "target-to-source", selectedAnswer: "hello" }],
    ]);
  });
});
