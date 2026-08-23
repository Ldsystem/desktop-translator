import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type {
  InstalledTextbook,
  PracticeQuestion,
  TextbookCatalogItem,
  VocabularyEntry,
} from "../../contracts/ipc";
import { VocabularyWindow, type StudyApi } from "./VocabularyWindow";

const entry: VocabularyEntry = {
  id: 1,
  sourceText: "hello",
  translatedText: "hola",
  requestedSourceLanguage: "auto",
  effectiveSourceLanguage: "en",
  targetLanguage: "es",
  lookupCount: 4,
  recallScore: 47,
  effectiveRecall: 43,
  familiarityLevel: 2,
  reviewCount: 1,
  correctCount: 1,
  wrongCount: 0,
  correctStreak: 1,
  wrongStreak: 0,
  lastSeenEpochMs: 1,
  lastReviewedEpochMs: 1,
};

const catalogItem: TextbookCatalogItem = {
  id: "wikdict-en-zh",
  title: "Everyday English · Chinese",
  sourceLanguage: "en",
  targetLanguage: "zh-CN",
  version: "2026.08",
  downloadUrl: "https://example.com/dictionary.sqlite",
  expectedBytes: 1024,
  sha256: "a".repeat(64),
  license: "CC BY-SA 4.0",
  attribution: "WikDict",
  sourceUrl: "https://www.wikdict.com/",
};

const installedBook: InstalledTextbook = {
  id: catalogItem.id,
  title: catalogItem.title,
  sourceLanguage: "en",
  targetLanguage: "zh-CN",
  version: catalogItem.version,
  license: catalogItem.license,
  attribution: catalogItem.attribution,
  sourceUrl: catalogItem.sourceUrl,
  entryCount: 8000,
  installedAtEpochMs: 1,
  active: true,
};

function makeStudyApi(overrides: Partial<StudyApi> = {}): StudyApi {
  return {
    listCatalog: vi.fn().mockResolvedValue([catalogItem]),
    listDownloaded: vi.fn().mockResolvedValue([]),
    downloadTextbook: vi.fn().mockResolvedValue(installedBook),
    setActiveTextbook: vi.fn().mockResolvedValue(undefined),
    removeTextbook: vi.fn().mockResolvedValue(undefined),
    listTextbookEntries: vi.fn().mockResolvedValue({ entries: [], total: 0, offset: 0, limit: 40 }),
    addTextbookEntry: vi.fn().mockResolvedValue({ vocabularyEntryId: 9, inserted: true }),
    listRelated: vi.fn().mockResolvedValue([]),
    getPracticePreferences: vi.fn().mockResolvedValue({ direction: "random" }),
    savePracticePreferences: vi.fn().mockResolvedValue(undefined),
    getPracticeQuestion: vi.fn().mockResolvedValue(null),
    submitPracticeAnswer: vi.fn(),
    refreshPersonal: vi.fn(),
    ...overrides,
  };
}

async function flushEffects() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("VocabularyWindow", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  it("renders the library with independent lookup and recall signals", () => {
    act(() => root.render(<VocabularyWindow entries={[entry]} loading={false} error={undefined} related={[]} question={undefined} outcome={undefined} onSearch={vi.fn()} onSelectEntry={vi.fn()} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} />));
    expect(container.textContent).toContain("hello");
    expect(container.textContent).toContain("4 lookups");
    const ruler = container.querySelector(".recall-ruler");
    expect(ruler?.hasAttribute("aria-label")).toBe(false);
    expect(ruler?.querySelector(".sr-only")?.textContent).toBe("Recall 43 out of 100");
    expect(ruler?.querySelector(".recall-ruler__value")?.textContent).toBe("43");
    expect(ruler?.querySelector(".recall-ruler__label")).toBeNull();
  });

  it("keeps pronunciation separate from opening the card", () => {
    const pronounce = vi.fn();
    const open = vi.fn();
    act(() => root.render(<VocabularyWindow entries={[entry]} loading={false} error={undefined} related={[]} question={undefined} outcome={undefined} speechAvailability={{ en: true }} onPronounce={pronounce} onSearch={vi.fn()} onSelectEntry={open} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} />));

    const speaker = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.getAttribute("aria-label") === "Pronounce hello");
    expect(speaker?.disabled).toBe(false);
    expect(container.querySelector("button button")).toBeNull();

    act(() => speaker?.click());
    expect(pronounce).toHaveBeenCalledWith("hello", "en");
    expect(open).not.toHaveBeenCalled();

    const cardAction = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.getAttribute("aria-label") === "Open related words for hello");
    act(() => cardAction?.click());
    expect(open).toHaveBeenCalledWith(1);
  });

  it("explains unavailable pronunciation states", () => {
    const props = { entries: [entry], loading: false, error: undefined, related: [], question: undefined, outcome: undefined, onPronounce: vi.fn(), onSearch: vi.fn(), onSelectEntry: vi.fn(), onStartPractice: vi.fn(), onSubmitAnswer: vi.fn() };
    act(() => root.render(<VocabularyWindow {...props} speechAvailability={{}} />));
    let speaker = container.querySelector<HTMLButtonElement>('[aria-label="Pronounce hello"]');
    expect(speaker?.disabled).toBe(true);
    expect(speaker?.title).toBe("Checking installed voice availability");
    expect(speaker?.parentElement?.tabIndex).toBe(0);
    expect(speaker?.parentElement?.getAttribute("aria-label")).toBe("Checking installed voice availability for hello");

    act(() => root.render(<VocabularyWindow {...props} speechAvailability={{ en: false }} />));
    speaker = container.querySelector<HTMLButtonElement>('[aria-label="Pronounce hello"]');
    expect(speaker?.disabled).toBe(true);
    expect(speaker?.title).toBe("No installed voice supports this language");
    expect(speaker?.parentElement?.getAttribute("aria-label")).toBe("No installed voice supports this language for hello");
  });

  it("does not reveal correctness until an answer is submitted", () => {
    const question: PracticeQuestion = { entryId: 1, sourceText: "hello", effectiveSourceLanguage: "en", targetLanguage: "es", choices: ["hola", "mundo"] };
    const submit = vi.fn();
    act(() => root.render(<VocabularyWindow entries={[entry]} loading={false} error={undefined} related={[]} question={question} outcome={undefined} onSearch={vi.fn()} onSelectEntry={vi.fn()} onStartPractice={vi.fn()} onSubmitAnswer={submit} />));
    expect(container.textContent).not.toContain("Correct");
    const choice = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "hola");
    act(() => choice?.click());
    const check = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Check answer");
    act(() => check?.click());
    expect(submit).toHaveBeenCalledWith(1, "hola");
  });

  it("keeps practice actions in the same dock when feedback appears", () => {
    const question: PracticeQuestion = { entryId: 1, sourceText: "hello", effectiveSourceLanguage: "en", targetLanguage: "es", choices: ["hola", "mundo"] };
    const props = { entries: [entry], loading: false, error: undefined, related: [], question, onSearch: vi.fn(), onSelectEntry: vi.fn(), onStartPractice: vi.fn(), onSubmitAnswer: vi.fn() };
    act(() => root.render(<VocabularyWindow {...props} outcome={undefined} />));
    const actions = container.querySelector(".practice-actions");
    expect(actions).not.toBeNull();
    expect(actions?.textContent).toContain("Check answer");
    const choice = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "hola");
    act(() => choice?.click());
    const check = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Check answer");
    act(() => check?.focus());

    act(() => root.render(<VocabularyWindow {...props} outcome={{ correct: true, correctTranslation: "hola", entry }} />));
    expect(container.querySelector(".practice-actions")).toBe(actions);
    expect(actions?.textContent).toContain("Correct");
    expect(actions?.textContent).toContain("Next word");
    const feedback = actions?.querySelector(".practice-feedback");
    const next = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Next word");
    expect(feedback?.contains(next ?? null)).toBe(false);
    expect(next?.parentElement).toBe(actions);
    expect([...container.querySelectorAll<HTMLButtonElement>(".practice-choice")].every((button) => button.disabled)).toBe(true);
    expect(document.activeElement?.textContent).toBe("Next word");
  });

  it("gives empty and failure states a clear next action", () => {
    act(() => root.render(<VocabularyWindow entries={[]} loading={false} error="Library unavailable" related={[]} question={undefined} outcome={undefined} onSearch={vi.fn()} onSelectEntry={vi.fn()} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} />));
    expect(container.textContent).toContain("Library unavailable");
    expect(container.textContent).toContain("Translate a word");
  });

  it("offers a fourth Textbooks destination with stable Discover and Downloaded pages", async () => {
    const api = makeStudyApi();
    act(() => root.render(<VocabularyWindow entries={[entry]} loading={false} related={[]} question={undefined} onSearch={vi.fn()} onSelectEntry={vi.fn()} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} studyApi={api} />));

    const textbooks = [...container.querySelectorAll<HTMLButtonElement>(".study-nav")].find((button) => button.textContent === "Textbooks");
    act(() => textbooks?.click());
    await flushEffects();

    expect(container.textContent).toContain("Textbook shelf");
    expect(container.textContent).toContain("Everyday English · Chinese");
    expect(container.textContent).toContain("Download");
    const downloaded = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Downloaded");
    act(() => downloaded?.click());
    expect(container.textContent).toContain("No downloaded textbooks yet");
  });

  it("browses an installed textbook and keeps an idempotent Add state on the entry", async () => {
    const addTextbookEntry = vi.fn().mockResolvedValue({ vocabularyEntryId: 9, inserted: true });
    const api = makeStudyApi({
      listDownloaded: vi.fn().mockResolvedValue([installedBook]),
      listTextbookEntries: vi.fn().mockResolvedValue({
        entries: [{ id: 81, textbookId: installedBook.id, sourceText: "ephemeral", translatedText: "短暂的", phoneticSymbols: "/ɪˈfemərəl/", sourceLanguage: "en", targetLanguage: "zh-CN" }],
        total: 1,
        offset: 0,
        limit: 40,
      }),
      addTextbookEntry,
    });
    act(() => root.render(<VocabularyWindow entries={[entry]} loading={false} related={[]} question={undefined} onSearch={vi.fn()} onSelectEntry={vi.fn()} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} studyApi={api} />));
    act(() => [...container.querySelectorAll<HTMLButtonElement>(".study-nav")].find((button) => button.textContent === "Textbooks")?.click());
    await flushEffects();
    act(() => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Downloaded")?.click());
    act(() => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Browse words")?.click());
    await flushEffects();

    expect(container.textContent).toContain("/ɪˈfemərəl/");
    const add = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Add to my wordbook");
    await act(async () => add?.click());
    expect(addTextbookEntry).toHaveBeenCalledWith(81);
    expect(container.textContent).toContain("Added");
    expect(add?.getAttribute("aria-disabled")).toBe("true");
  });

  it("switches related-word corpus and promotes textbook results", async () => {
    const listRelated = vi.fn().mockResolvedValue([{ kind: "textbook", textbookEntryId: 81, textbookId: installedBook.id, sourceText: "helpful", translatedText: "有帮助的", sourceLanguage: "en", targetLanguage: "zh-CN", reason: "root", promoted: false }]);
    const addTextbookEntry = vi.fn().mockResolvedValue({ vocabularyEntryId: 10, inserted: true });
    const api = makeStudyApi({ listDownloaded: vi.fn().mockResolvedValue([installedBook]), listRelated, addTextbookEntry });
    act(() => root.render(<VocabularyWindow entries={[entry]} loading={false} related={[]} question={undefined} onSearch={vi.fn()} onSelectEntry={vi.fn()} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} studyApi={api} />));
    act(() => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.getAttribute("aria-label") === "Open related words for hello")?.click());
    await flushEffects();
    const textbookSource = container.querySelector<HTMLInputElement>('input[value="textbook"]');
    act(() => textbookSource?.click());
    await flushEffects();

    expect(listRelated).toHaveBeenLastCalledWith(1, { kind: "textbook", textbookId: installedBook.id });
    expect(container.textContent).toContain("helpful");
    await act(async () => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Add")?.click());
    expect(addTextbookEntry).toHaveBeenCalledWith(81);
    expect(container.textContent).toContain("Added");
  });

  it("persists a practice direction and renders direction-neutral prompt fields", async () => {
    const savePracticePreferences = vi.fn().mockResolvedValue(undefined);
    const getPracticeQuestion = vi.fn().mockResolvedValue({ entryId: 1, direction: "target-to-source", prompt: "hola", promptLanguage: "es", answerLanguage: "en", choices: ["hello", "world"] });
    const api = makeStudyApi({ savePracticePreferences, getPracticeQuestion });
    act(() => root.render(<VocabularyWindow entries={[entry]} loading={false} related={[]} question={undefined} onSearch={vi.fn()} onSelectEntry={vi.fn()} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} studyApi={api} />));
    act(() => [...container.querySelectorAll<HTMLButtonElement>(".study-nav")].find((button) => button.textContent === "Practice")?.click());
    await flushEffects();
    const reverse = container.querySelector<HTMLInputElement>('input[value="target-to-source"]');
    await act(async () => reverse?.click());
    await flushEffects();

    expect(savePracticePreferences).toHaveBeenCalledWith({ direction: "target-to-source" });
    expect(container.textContent).toContain("hola");
    expect(container.textContent).toContain("ES → EN");
    expect(container.textContent).toContain("Choose the answer");
  });
});
