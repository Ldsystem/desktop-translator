import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { UserSettings } from "../contracts/ipc";
import type { OverlayState } from "../state/overlayMachine";
import App from "./App";

const settings: UserSettings = {
  schemaVersion: 2,
  enabled: true,
  sourceLanguage: "auto",
  targetLanguage: "en",
  startAtLogin: false,
  theme: "system",
  maxSelectionCodePoints: 5_000,
  uiLocale: "en",
  translationProvider: "google",
  microsoftCloud: "global",
};

const overlayState: OverlayState = {
  mode: "button-visible",
  generation: 1,
  selection: {
    id: 7,
    text: "retrieval",
    boundsPhysicalPx: [{ x: 20, y: 20, width: 80, height: 24 }],
    anchorPhysicalPx: { x: 20, y: 20, width: 80, height: 24 },
    capturedAtEpochMs: 1,
  },
};

describe("App overlay settings", () => {
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

  it("uses a later saved target language for selection translation", () => {
    const onTranslate = vi.fn();
    act(() =>
      root.render(
        <App
          mode="overlay"
          overlayState={overlayState}
          initialSettings={settings}
          onTranslate={onTranslate}
        />,
      ),
    );
    act(() =>
      root.render(
        <App
          mode="overlay"
          overlayState={overlayState}
          initialSettings={{ ...settings, targetLanguage: "zh-CN" }}
          onTranslate={onTranslate}
        />,
      ),
    );

    const button = container.querySelector<HTMLButtonElement>(
      'button[aria-label="Translate Selected Text"]',
    );
    expect(button).not.toBeNull();
    act(() => button?.click());
    expect(onTranslate).toHaveBeenCalledWith({
      selectionId: 7,
      text: "retrieval",
      sourceLanguage: "auto",
      targetLanguage: "zh-CN",
    });
  });
});
