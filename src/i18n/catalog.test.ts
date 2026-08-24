import { describe, expect, it } from "vitest";

import { messages } from "./catalog";

describe("bilingual message catalog", () => {
  it("covers the same keys in English and Simplified Chinese", () => {
    expect(Object.keys(messages("zh-CN"))).toEqual(Object.keys(messages("en")));
    expect(messages("zh-CN").settings).toBe("设置");
    expect(messages("en").settings).toBe("Settings");
  });

  it("keeps familiar service abbreviations where they are clearer", () => {
    expect(messages("zh-CN").appId).toBe("APP ID");
    expect(messages("zh-CN").apiKey).toContain("API");
  });
});
