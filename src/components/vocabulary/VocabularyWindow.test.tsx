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
  description: "High-frequency English for daily communication.",
  scope: "Everyday · NGSL",
  script: "Simplified Chinese",
  estimatedEntryCount: 2809,
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

const catalogChoices: TextbookCatalogItem[] = [
  catalogItem,
  { ...catalogItem, id: "academic", title: "Academic English", scope: "Academic · NAWL", estimatedEntryCount: 957 },
  { ...catalogItem, id: "toeic", title: "TOEIC English", scope: "TOEIC · TSL", estimatedEntryCount: 1250 },
  { ...catalogItem, id: "business", title: "Business English", scope: "Business · BSL", estimatedEntryCount: 1744 },
  { ...catalogItem, id: "general", title: "General English Dictionary", scope: "General reference", estimatedEntryCount: 30518 },
];

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
    deleteVocabularyEntry: vi.fn().mockResolvedValue(undefined),
    correctVocabularySourceLanguage: vi.fn().mockResolvedValue(entry),
    listVocabularyProvenance: vi.fn().mockResolvedValue([]),
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

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
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

  it("keeps the frame fixed and gives the active content one explicit scroll owner", () => {
    act(() => root.render(<VocabularyWindow entries={[entry]} loading={false} related={[]} question={undefined} onSearch={vi.fn()} onSelectEntry={vi.fn()} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} />));

    const frame = container.querySelector(".study-window");
    const rail = frame?.querySelector(":scope > .study-rail");
    const content = frame?.querySelector(":scope > .study-content");
    const scroller = content?.querySelector(":scope > .study-scroll-region");
    expect(rail).not.toBeNull();
    expect(scroller?.querySelector(".study-header")).not.toBeNull();
    expect(scroller?.getAttribute("data-scroll-owner")).toBe("study-content");
  });

  it("keeps two word-oriented pronunciation controls separate from opening the card", () => {
    const pronounce = vi.fn();
    const open = vi.fn();
    act(() => root.render(<VocabularyWindow entries={[entry]} loading={false} error={undefined} related={[]} question={undefined} outcome={undefined} speechAvailability={{ en: true, es: true }} onPronounce={pronounce} onSearch={vi.fn()} onSelectEntry={open} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} />));

    const speaker = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.getAttribute("aria-label") === "Pronounce hello");
    expect(speaker?.disabled).toBe(false);
    expect(speaker?.querySelector("svg")?.getAttribute("width")).toBe("16");
    expect(speaker?.querySelector("svg")?.getAttribute("height")).toBe("16");
    expect(container.querySelector("button button")).toBeNull();

    act(() => speaker?.click());
    expect(pronounce).toHaveBeenCalledWith("hello", "en");
    expect(open).not.toHaveBeenCalled();

    const translationSpeaker = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.getAttribute("aria-label") === "Pronounce hola");
    act(() => translationSpeaker?.click());
    expect(pronounce).toHaveBeenLastCalledWith("hola", "es");

    const cardAction = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.getAttribute("aria-label") === "Open related words for hello");
    const manageAction = container.querySelector<HTMLButtonElement>('[aria-label="Manage hello"]');
    expect(manageAction?.title).toBe("Manage word");
    expect(manageAction?.textContent).toBe("");
    expect(cardAction?.title).toBe("Find related words");
    expect(cardAction?.textContent).toBe("");
    act(() => cardAction?.click());
    expect(open).toHaveBeenCalledWith(1);
  });

  it("shows retained textbook provenance and its source for an opened personal word", async () => {
    const api = makeStudyApi({
      listVocabularyProvenance: vi.fn().mockResolvedValue([{
        textbookId: "wikdict-en-zh",
        textbookTitle: "WikDict English - Chinese",
        textbookVersion: "2_2026-06",
        license: "CC BY-SA 4.0",
        attribution: "WikDict, Wiktionary and DBnary contributors",
        sourceUrl: "https://www.wikdict.com/page/download",
        sourceText: "hello",
        translatedText: "\u4f60\u597d",
        promotedAtEpochMs: 1,
      }]),
    });
    act(() => root.render(<VocabularyWindow entries={[entry]} loading={false} related={[]} question={undefined} onSearch={vi.fn()} onSelectEntry={vi.fn()} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} studyApi={api} />));

    const cardAction = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.getAttribute("aria-label") === "Open related words for hello");
    act(() => cardAction?.click());
    await flushEffects();

    expect(api.listVocabularyProvenance).toHaveBeenCalledWith(1);
    const disclosure = container.querySelector<HTMLDetailsElement>(".word-provenance");
    expect(disclosure?.open).toBe(false);
    expect(disclosure?.querySelector("summary")?.textContent).toContain("Textbook source details");
    act(() => disclosure?.querySelector("summary")?.click());
    expect(disclosure?.open).toBe(true);
    expect(container.textContent).toContain("WikDict English - Chinese");
    expect(container.textContent).toContain("2_2026-06");
    expect(container.textContent).toContain("CC BY-SA 4.0");
    expect(container.textContent).toContain("WikDict, Wiktionary and DBnary contributors");
    expect(container.querySelector<HTMLAnchorElement>('a[href="https://www.wikdict.com/page/download"]')?.textContent).toBe("View source");
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

  it("corrects or confirms deletion by the selected immutable word id", async () => {
    const correctVocabularySourceLanguage = vi.fn().mockResolvedValue({ ...entry, effectiveSourceLanguage: "fr" });
    const deleteVocabularyEntry = vi.fn().mockResolvedValue(undefined);
    const api = makeStudyApi({ correctVocabularySourceLanguage, deleteVocabularyEntry });
    act(() => root.render(<VocabularyWindow entries={[entry]} loading={false} related={[]} question={undefined} onSearch={vi.fn()} onSelectEntry={vi.fn()} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} studyApi={api} />));

    act(() => container.querySelector<HTMLButtonElement>('[aria-label="Manage hello"]')?.click());
    const language = container.querySelector<HTMLSelectElement>(".word-manage select")!;
    act(() => { language.value = "fr"; language.dispatchEvent(new Event("change", { bubbles: true })); });
    await act(async () => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Save language")?.click());
    expect(correctVocabularySourceLanguage).toHaveBeenCalledWith(1, "fr");

    act(() => container.querySelector<HTMLButtonElement>('[aria-label="Manage hello"]')?.click());
    act(() => container.querySelector<HTMLButtonElement>('[aria-label="Delete hello"]')?.click());
    expect(container.textContent).toContain("Delete this word?");
    await act(async () => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Confirm")?.click());
    expect(deleteVocabularyEntry).toHaveBeenCalledWith(1);
  });

  it("opens management as one non-reflowing drawer linked to its card even in a long wordbook", () => {
    const entries = Array.from({ length: 18 }, (_, index) => ({
      ...entry,
      id: index + 1,
      sourceText: `word-${index + 1}`,
      translatedText: `meaning-${index + 1}`,
    }));
    const api = makeStudyApi();
    act(() => root.render(<VocabularyWindow entries={entries} loading={false} related={[]} question={undefined} onSearch={vi.fn()} onSelectEntry={vi.fn()} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} studyApi={api} />));

    const manageButtons = [...container.querySelectorAll<HTMLButtonElement>('.vocabulary-card button[aria-label^="Manage "]')];
    const lastManage = manageButtons.at(-1)!;
    act(() => lastManage.click());

    const drawer = container.querySelector<HTMLElement>('[role="dialog"][data-placement="drawer"]');
    expect(drawer?.id).toBe("word-manage-18");
    expect(lastManage.getAttribute("aria-controls")).toBe("word-manage-18");
    expect(lastManage.getAttribute("aria-expanded")).toBe("true");
    expect(container.querySelectorAll(".word-manage")).toHaveLength(1);
    expect(drawer?.querySelector(":scope > .word-manage__body")).not.toBeNull();
    const footer = drawer?.querySelector(":scope > .word-manage__footer");
    const deleteSlot = footer?.querySelector(".word-manage__delete-slot");
    expect(footer).not.toBeNull();
    expect(deleteSlot).not.toBeNull();
    expect(document.activeElement?.textContent).toBe("Close");

    act(() => drawer?.querySelector<HTMLButtonElement>('[aria-label="Delete word-18"]')?.click());
    expect(drawer?.querySelector(".word-manage__delete-slot")).toBe(deleteSlot);
    expect(drawer?.querySelector(".word-manage__confirm")?.textContent).toContain("Delete this word?");

    act(() => drawer?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(container.querySelector(".word-manage")).toBeNull();
    expect(document.activeElement).toBe(lastManage);
  });

  it("wraps long learning text and renders optional POS without an empty badge", () => {
    const long = "pneumonoultramicroscopicsilicovolcanoconiosis";
    act(() => root.render(<VocabularyWindow entries={[{ ...entry, sourceText: long, translatedText: `${long} translated meaning`, partOfSpeech: "noun" }, { ...entry, id: 2, sourceText: "plain", partOfSpeech: undefined }]} loading={false} related={[]} question={undefined} onSearch={vi.fn()} onSelectEntry={vi.fn()} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} />));

    const firstCard = container.querySelectorAll(".vocabulary-card")[0];
    expect(firstCard.querySelector(".lexical-text--long")?.textContent).toBe(long);
    expect(firstCard.querySelector(".part-of-speech")?.textContent).toBe("n.");
    expect(container.querySelectorAll(".vocabulary-card")[1].querySelector(".part-of-speech")).toBeNull();
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
    const api = makeStudyApi({ listCatalog: vi.fn().mockResolvedValue(catalogChoices) });
    act(() => root.render(<VocabularyWindow entries={[entry]} loading={false} related={[]} question={undefined} onSearch={vi.fn()} onSelectEntry={vi.fn()} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} studyApi={api} />));

    const textbooks = [...container.querySelectorAll<HTMLButtonElement>(".study-nav")].find((button) => button.textContent === "Textbooks");
    act(() => textbooks?.click());
    await flushEffects();

    expect(container.textContent).toContain("Textbook shelf");
    expect(container.textContent).toContain("Everyday English · Chinese");
    expect(container.textContent).toContain("Academic English");
    expect(container.textContent).toContain("TOEIC English");
    expect(container.textContent).toContain("Business English");
    expect(container.textContent).toContain("General English Dictionary");
    expect(container.textContent).toContain("Simplified Chinese");
    expect(container.textContent).toContain("2,809 words");
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
        entries: [{ id: 81, textbookId: installedBook.id, sourceText: "ephemeral", translatedText: "短暂的", phoneticSymbols: "/ɪˈfemərəl/", partOfSpeech: "adjective", sourceLanguage: "en", targetLanguage: "zh-CN" }],
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
    expect(container.querySelector(".textbook-entry .part-of-speech")?.textContent).toBe("adj.");
    const add = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Add to my wordbook");
    await act(async () => add?.click());
    expect(addTextbookEntry).toHaveBeenCalledWith(81);
    expect(container.textContent).toContain("Added");
    expect(add?.getAttribute("aria-disabled")).toBe("true");
  });

  it("shows one combined related list with origin badges and promotes textbook results", async () => {
    const listRelated = vi.fn().mockResolvedValue([{ kind: "textbook", textbookEntryId: 81, textbookId: installedBook.id, sourceText: "helpful", translatedText: "有帮助的", sourceLanguage: "en", targetLanguage: "zh-CN", partOfSpeech: "adjective", reason: "root", promoted: false, origins: [{ kind: "textbook", textbookId: installedBook.id, textbookTitle: installedBook.title }] }]);
    const addTextbookEntry = vi.fn().mockResolvedValue({ vocabularyEntryId: 10, inserted: true });
    const api = makeStudyApi({ listDownloaded: vi.fn().mockResolvedValue([installedBook]), listRelated, addTextbookEntry });
    act(() => root.render(<VocabularyWindow entries={[entry]} loading={false} related={[]} question={undefined} onSearch={vi.fn()} onSelectEntry={vi.fn()} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} studyApi={api} />));
    act(() => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.getAttribute("aria-label") === "Open related words for hello")?.click());
    await flushEffects();
    await flushEffects();

    expect(listRelated).toHaveBeenLastCalledWith(1);
    expect(container.querySelector(".source-selector")).toBeNull();
    expect(container.textContent).toContain("helpful");
    expect(container.querySelector(".relation-list .part-of-speech")?.textContent).toBe("adj.");
    expect(container.textContent).toContain(installedBook.title);
    const relatedRow = container.querySelector(".relation-list article")!;
    const tail = relatedRow.querySelector(":scope > .relation-tail");
    expect(relatedRow.children).toHaveLength(4);
    expect(tail?.querySelector(":scope > .relation-origins")).not.toBeNull();
    expect(tail?.querySelector(":scope > .relation-add")).not.toBeNull();
    await act(async () => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Add")?.click());
    expect(addTextbookEntry).toHaveBeenCalledWith(81);
    expect(container.textContent).toContain("Added");
  });

  it("persists a practice direction and renders direction-neutral prompt fields", async () => {
    const savePracticePreferences = vi.fn().mockResolvedValue(undefined);
    const getPracticeQuestion = vi.fn().mockResolvedValue({ entryId: 1, direction: "target-to-source", prompt: "hola", promptLanguage: "es", answerLanguage: "en", choices: [{ value: "hello" }, { value: "world" }] });
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

  it("renders practice POS while submitting the raw choice value", async () => {
    const submitPracticeAnswer = vi.fn().mockResolvedValue({ correct: true, correctAnswer: "replace", direction: "target-to-source", entry });
    const api = makeStudyApi({
      getPracticeQuestion: vi.fn().mockResolvedValue({
        entryId: 1,
        direction: "target-to-source",
        prompt: "取代",
        promptLanguage: "zh-CN",
        answerLanguage: "en",
        promptPartOfSpeech: "verb",
        choices: [
          { value: "replace", partOfSpeech: "verb" },
          { value: "replacement", partOfSpeech: "noun" },
        ],
      }),
      submitPracticeAnswer,
    });
    act(() => root.render(<VocabularyWindow entries={[entry]} loading={false} related={[]} question={undefined} onSearch={vi.fn()} onSelectEntry={vi.fn()} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} studyApi={api} />));
    act(() => [...container.querySelectorAll<HTMLButtonElement>(".study-nav")].find((button) => button.textContent === "Practice")?.click());
    await flushEffects();

    expect(container.querySelector(".practice-prompt .part-of-speech")?.textContent).toBe("v.");
    const choice = [...container.querySelectorAll<HTMLButtonElement>(".practice-choice")].find((button) => button.textContent?.includes("replace"));
    expect(choice?.querySelector(".part-of-speech")?.textContent).toBe("v.");
    act(() => choice?.click());
    await act(async () => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Check answer")?.click());
    expect(submitPracticeAnswer).toHaveBeenCalledWith(1, "target-to-source", "replace");
  });

  it("keeps the newest combined related response when an older request resolves last", async () => {
    const personal = deferred<Awaited<ReturnType<StudyApi["listRelated"]>>>();
    const textbook = deferred<Awaited<ReturnType<StudyApi["listRelated"]>>>();
    const listRelated = vi.fn().mockImplementationOnce(() => personal.promise).mockImplementationOnce(() => textbook.promise);
    const api = makeStudyApi({ listDownloaded: vi.fn().mockResolvedValue([installedBook]), listRelated });
    act(() => root.render(<VocabularyWindow entries={[entry]} loading={false} related={[]} question={undefined} onSearch={vi.fn()} onSelectEntry={vi.fn()} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} studyApi={api} />));
    act(() => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.getAttribute("aria-label") === "Open related words for hello")?.click());
    await flushEffects();
    act(() => root.render(<VocabularyWindow entries={[entry]} loading={false} revision={1} related={[]} question={undefined} onSearch={vi.fn()} onSelectEntry={vi.fn()} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} studyApi={api} />));
    await flushEffects();
    await act(async () => textbook.resolve([{ kind: "textbook", textbookEntryId: 81, textbookId: installedBook.id, sourceText: "current", translatedText: "当前", sourceLanguage: "en", targetLanguage: "zh-CN", reason: "root", promoted: false, origins: [] }]));
    await act(async () => personal.resolve([{ kind: "personal", vocabularyEntryId: 2, sourceText: "stale", translatedText: "过时", sourceLanguage: "en", targetLanguage: "zh-CN", reason: "root", promoted: true, origins: [] }]));

    expect(container.textContent).toContain("current");
    expect(container.textContent).not.toContain("stale");
  });

  it("keeps the latest textbook search when an older page resolves last", async () => {
    const initial = deferred<Awaited<ReturnType<StudyApi["listTextbookEntries"]>>>();
    const latest = deferred<Awaited<ReturnType<StudyApi["listTextbookEntries"]>>>();
    const listTextbookEntries = vi.fn().mockImplementationOnce(() => initial.promise).mockImplementationOnce(() => latest.promise);
    const api = makeStudyApi({ listDownloaded: vi.fn().mockResolvedValue([installedBook]), listTextbookEntries });
    act(() => root.render(<VocabularyWindow entries={[entry]} loading={false} related={[]} question={undefined} onSearch={vi.fn()} onSelectEntry={vi.fn()} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} studyApi={api} />));
    act(() => [...container.querySelectorAll<HTMLButtonElement>(".study-nav")].find((button) => button.textContent === "Textbooks")?.click());
    await flushEffects();
    act(() => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Downloaded")?.click());
    act(() => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Browse words")?.click());
    const search = container.querySelector<HTMLInputElement>('input[placeholder="Find a word in this textbook"]')!;
    act(() => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setter?.call(search, "current");
      search.dispatchEvent(new Event("input", { bubbles: true }));
      search.closest("form")?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    });
    await act(async () => latest.resolve({ entries: [{ id: 82, textbookId: installedBook.id, sourceText: "current", translatedText: "当前", sourceLanguage: "en", targetLanguage: "zh-CN" }], total: 1, offset: 0, limit: 40 }));
    await act(async () => initial.resolve({ entries: [{ id: 81, textbookId: installedBook.id, sourceText: "stale", translatedText: "过时", sourceLanguage: "en", targetLanguage: "zh-CN" }], total: 1, offset: 0, limit: 40 }));

    expect(container.textContent).toContain("current");
    expect(container.textContent).not.toContain("stale");
  });

  it("restores the persisted direction when saving fails and retries the save", async () => {
    const savePracticePreferences = vi.fn().mockRejectedValueOnce(new Error("disk busy")).mockResolvedValueOnce(undefined);
    const api = makeStudyApi({ savePracticePreferences, getPracticeQuestion: vi.fn().mockResolvedValue({ entryId: 1, direction: "source-to-target", prompt: "hello", promptLanguage: "en", answerLanguage: "es", choices: [{ value: "hola" }, { value: "mundo" }] }) });
    act(() => root.render(<VocabularyWindow entries={[entry]} loading={false} related={[]} question={undefined} onSearch={vi.fn()} onSelectEntry={vi.fn()} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} studyApi={api} />));
    act(() => [...container.querySelectorAll<HTMLButtonElement>(".study-nav")].find((button) => button.textContent === "Practice")?.click());
    await flushEffects();
    await act(async () => container.querySelector<HTMLInputElement>('input[value="target-to-source"]')?.click());
    await flushEffects();
    expect(container.querySelector<HTMLInputElement>('input[value="random"]')?.checked).toBe(true);
    expect(container.textContent).toContain("could not be saved");
    await act(async () => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Try saving again")?.click());
    await flushEffects();
    expect(savePracticePreferences).toHaveBeenCalledTimes(2);
    expect(container.querySelector<HTMLInputElement>('input[value="target-to-source"]')?.checked).toBe(true);
  });

  it("submits each practice question at most once while scoring is pending", async () => {
    const pending = deferred<Awaited<ReturnType<StudyApi["submitPracticeAnswer"]>>>();
    const submitPracticeAnswer = vi.fn().mockReturnValue(pending.promise);
    const api = makeStudyApi({ submitPracticeAnswer, getPracticeQuestion: vi.fn().mockResolvedValue({ entryId: 1, direction: "source-to-target", prompt: "hello", promptLanguage: "en", answerLanguage: "es", choices: [{ value: "hola" }, { value: "mundo" }] }) });
    act(() => root.render(<VocabularyWindow entries={[entry]} loading={false} related={[]} question={undefined} onSearch={vi.fn()} onSelectEntry={vi.fn()} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} studyApi={api} />));
    act(() => [...container.querySelectorAll<HTMLButtonElement>(".study-nav")].find((button) => button.textContent === "Practice")?.click());
    await flushEffects();
    act(() => [...container.querySelectorAll<HTMLButtonElement>(".practice-choice")].find((button) => button.textContent === "hola")?.click());
    const check = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Check answer")!;
    act(() => { check.click(); check.click(); });

    expect(submitPracticeAnswer).toHaveBeenCalledTimes(1);
    expect(check.disabled).toBe(true);
    await act(async () => pending.resolve({ correct: true, correctAnswer: "hola", direction: "source-to-target", entry }));
  });

  it("keeps scored feedback until Next when its revision event arrives during submit", async () => {
    const pending = deferred<Awaited<ReturnType<StudyApi["submitPracticeAnswer"]>>>();
    const first = { entryId: 1, direction: "source-to-target" as const, prompt: "hello", promptLanguage: "en", answerLanguage: "es", choices: [{ value: "hola" }, { value: "mundo" }] };
    const second = { entryId: 2, direction: "source-to-target" as const, prompt: "world", promptLanguage: "en", answerLanguage: "es", choices: [{ value: "mundo" }, { value: "hola" }] };
    const getPracticeQuestion = vi.fn().mockResolvedValueOnce(first).mockResolvedValueOnce(second);
    const api = makeStudyApi({ getPracticeQuestion, submitPracticeAnswer: vi.fn().mockReturnValue(pending.promise) });
    const props = { entries: [entry], loading: false, related: [], question: undefined, onSearch: vi.fn(), onSelectEntry: vi.fn(), onStartPractice: vi.fn(), onSubmitAnswer: vi.fn(), studyApi: api };
    act(() => root.render(<VocabularyWindow {...props} revision={0} />));
    act(() => [...container.querySelectorAll<HTMLButtonElement>(".study-nav")].find((button) => button.textContent === "Practice")?.click());
    await flushEffects();
    act(() => [...container.querySelectorAll<HTMLButtonElement>(".practice-choice")].find((button) => button.textContent === "hola")?.click());
    act(() => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Check answer")?.click());

    act(() => root.render(<VocabularyWindow {...props} revision={1} />));
    await act(async () => pending.resolve({ correct: true, correctAnswer: "hola", direction: "source-to-target", entry: { ...entry, effectiveRecall: 100 } }));

    expect(getPracticeQuestion).toHaveBeenCalledTimes(1);
    expect(container.textContent).toContain("Correct");
    expect(container.textContent).toContain("Recall is now 100.");
    expect(container.textContent).toContain("hello");

    await act(async () => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Next word")?.click());
    await flushEffects();
    expect(getPracticeQuestion).toHaveBeenCalledTimes(2);
    expect(container.textContent).toContain("world");
    expect(container.textContent).not.toContain("Recall is now 100.");
  });

  it("keeps direction controls locked to the question while scoring is pending", async () => {
    const pending = deferred<Awaited<ReturnType<StudyApi["submitPracticeAnswer"]>>>();
    const savePracticePreferences = vi.fn().mockResolvedValue(undefined);
    const api = makeStudyApi({ savePracticePreferences, submitPracticeAnswer: vi.fn().mockReturnValue(pending.promise), getPracticeQuestion: vi.fn().mockResolvedValue({ entryId: 1, direction: "source-to-target", prompt: "hello", promptLanguage: "en", answerLanguage: "es", choices: [{ value: "hola" }, { value: "mundo" }] }) });
    act(() => root.render(<VocabularyWindow entries={[entry]} loading={false} related={[]} question={undefined} onSearch={vi.fn()} onSelectEntry={vi.fn()} onStartPractice={vi.fn()} onSubmitAnswer={vi.fn()} studyApi={api} />));
    act(() => [...container.querySelectorAll<HTMLButtonElement>(".study-nav")].find((button) => button.textContent === "Practice")?.click());
    await flushEffects();
    act(() => [...container.querySelectorAll<HTMLButtonElement>(".practice-choice")].find((button) => button.textContent === "hola")?.click());
    act(() => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Check answer")?.click());
    const reverse = container.querySelector<HTMLInputElement>('input[value="target-to-source"]')!;

    expect(reverse.matches(":disabled")).toBe(true);
    act(() => reverse.click());
    expect(savePracticePreferences).not.toHaveBeenCalled();
    await act(async () => pending.resolve({ correct: true, correctAnswer: "hola", direction: "source-to-target", entry }));
    expect(container.textContent).toContain("hello");
    expect(container.textContent).toContain("Correct");
  });
});
