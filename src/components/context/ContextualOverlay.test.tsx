import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OverlayState } from "../../state/overlayMachine";
import { ContextualOverlay } from "./ContextualOverlay";

const selection = {
  id: 7,
  text: "Bonjour",
  boundsPhysicalPx: [{ x: 20, y: 20, width: 80, height: 24 }],
  anchorPhysicalPx: { x: 20, y: 20, width: 80, height: 24 },
  capturedAtEpochMs: 1,
};

describe("ContextualOverlay", () => {
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

  function render(state: OverlayState, overrides = {}) {
    const props = {
      state,
      sourceLanguage: "auto",
      targetLanguage: "en",
      onTranslate: vi.fn(),
      onCorrectSource: vi.fn(),
      onSpeak: vi.fn(),
      onDismiss: vi.fn(),
      ...overrides,
    };

    act(() => root.render(<ContextualOverlay {...props} />));
    return props;
  }

  it("starts translation from the compact contextual button", () => {
    const state: OverlayState = {
      mode: "button-visible",
      selection,
      generation: 1,
    };
    const props = render(state);

    const button = container.querySelector<HTMLButtonElement>(
      'button[aria-label="Translate Selected Text"]',
    );
    expect(button).not.toBeNull();
    act(() => button?.focus());
    expect(document.activeElement).toBe(button);
    expect(button?.tabIndex).toBe(0);
    act(() => button?.click());
    expect(props.onTranslate).toHaveBeenCalledWith({
      selectionId: 7,
      text: "Bonjour",
      sourceLanguage: "auto",
      targetLanguage: "en",
    });
  });

  it("renders source correction, speech actions, and dismisses with Escape", () => {
    const state: OverlayState = {
      mode: "result-visible",
      selection,
      generation: 1,
      result: {
        selectionId: 7,
        translatedText: "Hello",
        detectedSourceLanguage: "fr",
        effectiveSourceLanguage: "fr",
        targetLanguage: "en",
      },
    };
    const props = render(state);

    expect(container.textContent).toContain("Bonjour");
    expect(container.textContent).toContain("Hello");
    expect(container.querySelectorAll('button[aria-label^="Speak"]').length).toBe(2);

    const select = container.querySelector<HTMLSelectElement>("#overlay-source-language");
    act(() => {
      if (select) {
        select.value = "de";
        select.dispatchEvent(new Event("change", { bubbles: true }));
      }
    });
    expect(props.onCorrectSource).toHaveBeenCalledWith({
      selectionId: 7,
      text: "Bonjour",
      sourceLanguage: "de",
      targetLanguage: "en",
    });

    act(() => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
    expect(props.onDismiss).toHaveBeenCalledOnce();
  });

  it("disables speech actions when no installed voice supports the language", () => {
    const state: OverlayState = {
      mode: "result-visible",
      selection,
      generation: 1,
      result: {
        selectionId: 7,
        translatedText: "Hello",
        detectedSourceLanguage: "fr",
        effectiveSourceLanguage: "fr",
        targetLanguage: "en",
      },
    };

    render(state, {
      speechAvailability: {
        fr: false,
        en: true,
      },
    });

    expect(
      container.querySelector<HTMLButtonElement>('button[aria-label="Speak Source Text"]')
        ?.disabled,
    ).toBe(true);
    expect(
      container.querySelector<HTMLButtonElement>('button[aria-label="Speak Translation"]')
        ?.disabled,
    ).toBe(false);
  });

  it("announces loading and distinguishes a permission error", () => {
    render({
      mode: "translating",
      selection,
      generation: 1,
    });
    expect(container.querySelector('[role="status"]')?.textContent).toContain("Translating…");

    render({
      mode: "error-visible",
      selection,
      generation: 1,
      error: {
        code: "permission-denied",
        message: "Selection access is unavailable.",
        retryable: false,
      },
    });
    expect(container.querySelector('[role="alert"]')?.textContent).toContain(
      "Accessibility Permission Required",
    );
  });

  it("localizes the compact selection surface in Simplified Chinese", () => {
    render({ mode: "translating", selection, generation: 1 }, { locale: "zh-CN" });

    expect(container.querySelector('[role="status"]')?.textContent).toContain("正在翻译…");
    expect(container.querySelector('aside[aria-label="翻译"]')).not.toBeNull();
  });

  it("localizes stable error copy in Simplified Chinese", () => {
    render({
      mode: "error-visible",
      selection,
      generation: 1,
      error: { code: "offline", message: "You are offline", retryable: true },
    }, { locale: "zh-CN" });

    const alert = container.querySelector('[role="alert"]');
    expect(alert?.textContent).toContain("当前处于离线状态");
    expect(alert?.textContent).toContain("请重新连接网络后重试。");
    expect(alert?.textContent).not.toContain("You’re Offline");
  });
});
