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
    expect(container.textContent).toContain("Configure API key");
    expect(container.textContent).toContain("Test");
    expect(container.textContent).toContain("Remove");
    expect(container.textContent).not.toContain("Gemini");
    expect(container.textContent).not.toContain("Explanation");

    const saveKey = Array.from(container.querySelectorAll("button")).find(
      (button) => button.textContent === "Configure API key",
    );
    act(() => saveKey?.focus());
    expect(document.activeElement).toBe(saveKey);
    act(() => saveKey?.click());
    expect(onSaveCredential).toHaveBeenCalledWith("google", "api-key");
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
      "reopen Desktop Translator",
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

  it("persists the target language when the dropdown changes", () => {
    const onSave = vi.fn();
    act(() =>
      root.render(
        <SettingsPanel
          settings={settings}
          credentialStatus="ready"
          permissionStatus="granted"
          onSave={onSave}
          onSaveCredential={vi.fn()}
          onTestCredential={vi.fn()}
          onRemoveCredential={vi.fn()}
          onOpenSystemSettings={vi.fn()}
          onQuit={vi.fn()}
        />,
      ),
    );

    const target = container.querySelector<HTMLSelectElement>("#target-language");
    act(() => {
      if (target) {
        target.value = "zh-CN";
        target.dispatchEvent(new Event("change", { bubbles: true }));
      }
    });

    expect(onSave).toHaveBeenCalledWith(
      expect.objectContaining({ targetLanguage: "zh-CN", sourceLanguage: "auto" }),
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
      "Text is sent only to the selected provider after you choose Translate.",
    );
    expect(container.textContent).toContain("Credentials stay in the operating-system vault.");
    expect(container.textContent).toContain("Personal vocabulary and practice data stay on this device.");
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
    const appearanceCard = container.querySelector(".settings-card--appearance");
    expect(appearanceCard?.querySelector(".theme-picker")).not.toBeNull();
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
