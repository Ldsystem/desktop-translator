// @vitest-environment node
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const root = fileURLToPath(new URL("../../", import.meta.url));
const artifactPath = "src-tauri/resources/lexicon/morphemes-v1.json";

describe("bundled lexicon integrity", () => {
  it("embeds exactly the bytes verified by the release manifest", () => {
    const bytes = readFileSync(new URL(`../../${artifactPath}`, import.meta.url));
    const manifest = JSON.parse(readFileSync(
      new URL("../../src-tauri/resources/lexicon/morphemes-v1.manifest.json", import.meta.url),
      "utf8",
    )) as { expectedBytes: number; sha256: string };

    expect(bytes.length).toBe(manifest.expectedBytes);
    expect(createHash("sha256").update(bytes).digest("hex")).toBe(manifest.sha256);
  });

  it.each(["false", "true"])("pins LF even with core.autocrlf=%s", (autocrlf) => {
    // Git's checkout policy must not rewrite the bytes before Rust include_bytes!.
    const attribute = execFileSync("git", [
      "-c", `core.autocrlf=${autocrlf}`, "check-attr", "eol", "--", artifactPath,
    ], { cwd: root, encoding: "utf8", timeout: 10_000 });

    expect(attribute.trim()).toBe(`${artifactPath}: eol: lf`);
  });
});
