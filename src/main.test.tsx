import { describe, expect, it } from "vitest";

import { mount } from "./main";

describe("application bootstrap", () => {
  it("mounts into the provided host", () => {
    const host = document.createElement("div");

    expect(() => mount(host)).not.toThrow();
  });
});
