// @vitest-environment node
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const script = join(dirname(fileURLToPath(import.meta.url)), "audit-windows-bundle.ps1");

function runAudit(releaseDir: string) {
  return spawnSync(
    "powershell.exe",
    ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script, releaseDir],
    { encoding: "utf8" },
  );
}

function makeReleaseLayout(options: {
  setup?: boolean;
  exe?: boolean;
  developerRuntime?: boolean;
  looseTextbook?: boolean;
}) {
  const root = mkdtempSync(join(tmpdir(), "windows-bundle-audit-"));
  mkdirSync(join(root, "bundle", "nsis"), { recursive: true });
  if (options.setup !== false) {
    writeFileSync(
      join(root, "bundle", "nsis", "Desktop Translator_0.3.0_x64-setup.exe"),
      "setup",
    );
  }
  if (options.exe !== false) {
    writeFileSync(join(root, "desktop-translator.exe"), "exe");
  }
  if (options.developerRuntime) {
    writeFileSync(join(root, "node.exe"), "node");
  }
  if (options.looseTextbook) {
    mkdirSync(join(root, "resources", "textbooks"), { recursive: true });
    writeFileSync(join(root, "resources", "textbooks", "starter-en-zh.sqlite3"), "db");
  }
  return root;
}

describe("Windows bundle audit", () => {
  it("fails when the NSIS x64 installer is missing", () => {
    const root = makeReleaseLayout({ setup: false });
    try {
      const result = runAudit(root);
      expect(result.status).not.toBe(0);
      expect(`${result.stdout}${result.stderr}`).toMatch(/nsis|setup|installer/i);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it("fails when the native executable is missing", () => {
    const root = makeReleaseLayout({ exe: false });
    try {
      const result = runAudit(root);
      expect(result.status).not.toBe(0);
      expect(`${result.stdout}${result.stderr}`).toMatch(/desktop-translator\.exe/i);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it("fails when a developer runtime is packaged", () => {
    const root = makeReleaseLayout({ developerRuntime: true });
    try {
      const result = runAudit(root);
      expect(result.status).not.toBe(0);
      expect(`${result.stdout}${result.stderr}`).toMatch(/developer/i);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it("fails when the starter textbook is a loose resource file", () => {
    const root = makeReleaseLayout({ looseTextbook: true });
    try {
      const result = runAudit(root);
      expect(result.status).not.toBe(0);
      expect(`${result.stdout}${result.stderr}`).toMatch(/textbook|sqlite/i);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it("passes a compact x64 NSIS layout without developer runtimes", () => {
    const root = makeReleaseLayout({});
    try {
      const result = runAudit(root);
      expect(result.status, `${result.stdout}${result.stderr}`).toBe(0);
      expect(`${result.stdout}${result.stderr}`).toMatch(/bundle audit passed/i);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});
