import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { QuickTranslatePanel, type QuickTranslateStatus } from "./QuickTranslatePanel";

describe("QuickTranslatePanel", () => {
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

  function render(status: QuickTranslateStatus = { mode: "idle" }, overrides = {}) {
    const props = {
      status,
      sourceLanguage: "auto" as const,
      targetLanguage: "es" as const,
      onTranslate: vi.fn(),
      onSpeak: vi.fn(),
      ...overrides,
    };

    act(() => root.render(<QuickTranslatePanel {...props} />));
    return props;
  }

  function typeInto(value: string) {
    const input = container.querySelector<HTMLTextAreaElement>("textarea");
    expect(input).not.toBeNull();
    act(() => {
      const setter = Object.getOwnPropertyDescriptor(
        HTMLTextAreaElement.prototype,
        "value",
      )?.set;
      setter?.call(input, value);
      input?.dispatchEvent(new Event("input", { bubbles: true }));
    });
    return input;
  }

  function submit() {
    const form = container.querySelector("form");
    act(() => {
      form?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    });
  }

  it("translates the typed text with the selected language pair", () => {
    const props = render();
    typeInto("bonjour");
    submit();

    expect(props.onTranslate).toHaveBeenCalledWith({
      selectionId: 0,
      text: "bonjour",
      sourceLanguage: "auto",
      targetLanguage: "es",
    });
  });

  it("adopts a later default target language from settings", () => {
    const onTranslate = vi.fn();
    render({ mode: "idle" }, { onTranslate, targetLanguage: "en" });
    render({ mode: "idle" }, { onTranslate, targetLanguage: "zh-CN" });
    typeInto("retrieval");
    submit();

    expect(onTranslate).toHaveBeenCalledWith({
      selectionId: 0,
      text: "retrieval",
      sourceLanguage: "auto",
      targetLanguage: "zh-CN",
    });
  });

  it("does not translate blank input", () => {
    const props = render();
    typeInto("   ");
    submit();

    expect(props.onTranslate).not.toHaveBeenCalled();
  });

  it("shows the translated text once it resolves", () => {
    render({
      mode: "result",
      result: {
        selectionId: 0,
        translatedText: "hola",
        detectedSourceLanguage: "fr",
        effectiveSourceLanguage: "fr",
        targetLanguage: "es",
        senses: [
          { text: "hola", rank: 0, isPrimary: true, partOfSpeech: "interjection" },
          { text: "buenas", rank: 1, isPrimary: false, partOfSpeech: "interjection" },
        ],
      },
    });

    expect(container.textContent).toContain("hola");
    expect(container.querySelector(".translation-senses")?.textContent).toContain("Other translations (1)");
    expect(container.querySelector(".translation-senses")?.textContent).toContain("buenas");
  });

  it("bounds a large alternative set until Show all is activated", () => {
    render({
      mode: "result",
      result: {
        selectionId: 0,
        translatedText: "one",
        effectiveSourceLanguage: "en",
        targetLanguage: "es",
        senses: [
          { text: "one", rank: 0, isPrimary: true },
          ...["two", "three", "four", "five", "six"].map((text, index) => ({ text, rank: index + 1, isPrimary: false })),
        ],
      },
    });
    const toggle = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Show all");
    expect(toggle?.getAttribute("aria-expanded")).toBe("false");
    expect(container.querySelectorAll(".translation-senses__item")).toHaveLength(3);
    expect(container.querySelector("details:not([open]) .translation-senses__item")).toBeNull();
    expect(container.querySelector(".translation-senses")?.tagName).toBe("SECTION");
    act(() => toggle?.click());
    expect(container.querySelectorAll(".translation-senses__item")).toHaveLength(5);
    expect(toggle?.getAttribute("aria-expanded")).toBe("true");
  });

  it("reports a failure without clearing the typed text", () => {
    render({
      mode: "error",
      error: { code: "offline", message: "You are offline", retryable: true },
    });

    expect(container.textContent).toContain("You’re Offline");
    expect(container.querySelector("textarea")).not.toBeNull();
  });

  it("localizes quick translation controls in Simplified Chinese", () => {
    render({ mode: "idle" }, { locale: "zh-CN" });

    expect(container.textContent).toContain("源语言");
    expect(container.textContent).toContain("目标语言");
    expect(container.querySelector("textarea")?.placeholder).toBe("输入或粘贴要翻译的文本");
  });

  it("localizes quick-translation error copy in Simplified Chinese", () => {
    render({
      mode: "error",
      error: { code: "missing-credential", message: "Missing key", retryable: false },
    }, { locale: "zh-CN" });

    expect(container.textContent).toContain("需要 API 密钥");
    expect(container.textContent).toContain("请在设置中保存 API 密钥。");
    expect(container.textContent).not.toContain("API Key Required");
  });
});
