import { describe, expect, it } from "vitest";

import { getWindowMode } from "./windowMode";

describe("getWindowMode", () => {
  it("resolves each native surface from the window URL", () => {
    expect(getWindowMode({ search: "?mode=overlay", pathname: "/" })).toBe("overlay");
    expect(getWindowMode({ search: "?mode=quick", pathname: "/" })).toBe("quick");
    expect(getWindowMode({ search: "?mode=study", pathname: "/" })).toBe("study");
    expect(getWindowMode({ search: "?mode=settings", pathname: "/" })).toBe("settings");
  });

  it("falls back to settings for an unknown or missing mode", () => {
    expect(getWindowMode({ search: "", pathname: "/" })).toBe("settings");
    expect(getWindowMode({ search: "?mode=nonsense", pathname: "/" })).toBe("settings");
  });
});
