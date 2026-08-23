import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { PracticeQuestion, VocabularyEntry } from "../../contracts/ipc";
import { VocabularyWindow } from "./VocabularyWindow";

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
});
