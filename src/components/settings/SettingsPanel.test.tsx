// @ts-expect-error The runtime provides node:fs; production code remains browser-only.
import { readFileSync } from "node:fs";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { UserSettings } from "../../contracts/ipc";
import { SettingsPanel } from "./SettingsPanel";

const appCss = readFileSync("src/styles/app.css", "utf8");
const tokensCss = readFileSync("src/styles/tokens.css", "utf8");

const settings: UserSettings = {
  schemaVersion: 1,
  enabled: true,
  sourceLanguage: "auto",
  targetLanguage: "en",
  startAtLogin: false,
  theme: "system",
  maxSelectionCodePoints: 5_000,
};

describe("SettingsPanel", () => {
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

  it("exposes credential actions without accepting a credential value", () => {
    const onSaveCredential = vi.fn();
    act(() =>
      root.render(
        <SettingsPanel
          settings={settings}
          credentialStatus="ready"
          permissionStatus="granted"
          onSave={vi.fn()}
          onSaveCredential={onSaveCredential}
          onTestCredential={vi.fn()}
          onRemoveCredential={vi.fn()}
          onOpenSystemSettings={vi.fn()}
          onQuit={vi.fn()}
        />,
      ),
    );

    expect(container.querySelector('input[type="password"]')).toBeNull();
    expect(container.textContent).toContain("Save API Key");
    expect(container.textContent).toContain("Test API Key");
    expect(container.textContent).toContain("Remove API Key");
    expect(container.textContent).not.toContain("Gemini");
    expect(container.textContent).not.toContain("Explanation");

    const saveKey = Array.from(container.querySelectorAll("button")).find(
      (button) => button.textContent === "Save API Key",
    );
    act(() => saveKey?.focus());
    expect(document.activeElement).toBe(saveKey);
    act(() => saveKey?.click());
    expect(onSaveCredential).toHaveBeenCalledOnce();
  });

  it("forces monitoring off and links denied permission guidance", () => {
    const onSave = vi.fn();
    const onOpenSystemSettings = vi.fn();
    act(() =>
      root.render(
        <SettingsPanel
          settings={settings}
          credentialStatus="missing"
          permissionStatus="denied"
          onSave={onSave}
          onSaveCredential={vi.fn()}
          onTestCredential={vi.fn()}
          onRemoveCredential={vi.fn()}
          onOpenSystemSettings={onOpenSystemSettings}
          onQuit={vi.fn()}
        />,
      ),
    );

    expect(container.querySelector('[role="alert"]')?.textContent).toContain(
      "Allow access in System Settings",
    );
    expect(container.textContent).toContain("Monitoring Off");
    const enable = container.querySelector<HTMLInputElement>('input[name="enabled"]');
    expect(enable?.checked).toBe(false);
    expect(enable?.disabled).toBe(true);
    const systemSettings = Array.from(container.querySelectorAll("button")).find(
      (button) => button.textContent === "Open System Settings",
    );
    act(() => systemSettings?.click());
    expect(onOpenSystemSettings).toHaveBeenCalledOnce();

    const target = container.querySelector<HTMLSelectElement>("#target-language");
    act(() => {
      if (target) {
        target.value = "ja";
        target.dispatchEvent(new Event("change", { bubbles: true }));
      }
    });
    const form = container.querySelector("form");
    act(() => form?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(onSave).toHaveBeenCalledWith(
      expect.objectContaining({ enabled: false, targetLanguage: "ja" }),
    );
  });

  it("explains privacy, explicit sending, and API cost constraints", () => {
    act(() =>
      root.render(
        <SettingsPanel
          settings={settings}
          credentialStatus="ready"
          permissionStatus="granted"
          onSave={vi.fn()}
          onSaveCredential={vi.fn()}
          onTestCredential={vi.fn()}
          onRemoveCredential={vi.fn()}
          onOpenSystemSettings={vi.fn()}
          onQuit={vi.fn()}
        />,
      ),
    );

    expect(container.textContent).toContain(
      "Selected text is sent to Google only after you choose Translate.",
    );
    expect(container.textContent).toContain("billing, quota, and API key restrictions");
    expect(container.textContent).toContain("No content history is stored.");
  });

  it("provides theme, focus, motion, contrast, and target-size contracts", () => {
    act(() =>
      root.render(
        <SettingsPanel
          settings={settings}
          credentialStatus="ready"
          permissionStatus="granted"
          onSave={vi.fn()}
          onSaveCredential={vi.fn()}
          onTestCredential={vi.fn()}
          onRemoveCredential={vi.fn()}
          onOpenSystemSettings={vi.fn()}
          onQuit={vi.fn()}
        />,
      ),
    );

    const themes = Array.from(
      container.querySelectorAll<HTMLInputElement>('input[name="theme"]'),
      (input) => input.value,
    );
    expect(themes).toEqual(["system", "light", "dark"]);
    expect(appCss).toContain(":focus-visible");
    expect(appCss).toContain("@media (prefers-reduced-motion: reduce)");
    expect(appCss).toContain("animation-duration: 0.01ms !important");
    expect(appCss).toContain("@media (forced-colors: active)");
    expect(appCss).toContain("outline: 3px solid Highlight");
    expect(appCss).toContain("border: 2px solid ButtonText");
    expect(tokensCss).toContain('[data-theme="dark"]');
    expect(tokensCss).toContain("@media (prefers-color-scheme: dark)");
    expect(tokensCss).toContain("--target: 44px");
    expect(appCss).toMatch(
      /\.button,[\s\S]*?\.translate-trigger\s*\{[\s\S]*?min-height:\s*var\(--target\)/,
    );
    expect(appCss).toMatch(/\.field select\s*\{[\s\S]*?min-height:\s*var\(--target\)/);
    expect(appCss).toMatch(
      /\.translation-block__header select\s*\{[\s\S]*?min-height:\s*var\(--target\)/,
    );
  });
});
