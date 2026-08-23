import { describe, expect, it, vi } from "vitest";

import { createPronouncer } from "./pronunciation";

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => { resolve = done; });
  return { promise, resolve };
}

describe("createPronouncer", () => {
  it("stops current speech before speaking the newest request", async () => {
    const firstStop = deferred();
    const secondStop = deferred();
    const stop = vi.fn()
      .mockReturnValueOnce(firstStop.promise)
      .mockReturnValueOnce(secondStop.promise);
    const speak = vi.fn().mockResolvedValue(undefined);
    const pronounce = createPronouncer({ stop, speak, onError: vi.fn() });

    pronounce("first", "en");
    pronounce("second", "en");
    await vi.waitFor(() => expect(stop).toHaveBeenCalledTimes(1));

    // Even if the second stop would already be resolved, it is not allowed to
    // run ahead of the first and then be undone by that older stop operation.
    secondStop.resolve();
    expect(stop).toHaveBeenCalledTimes(1);
    firstStop.resolve();
    await vi.waitFor(() => {
      expect(stop).toHaveBeenCalledTimes(2);
      expect(speak).toHaveBeenCalledTimes(1);
      expect(speak).toHaveBeenCalledWith("second", "en");
    });
  });

  it("continues after a stop failure and reports only current playback failures", async () => {
    const onError = vi.fn();
    const speak = vi.fn().mockRejectedValue(new Error("voice failed"));
    const pronounce = createPronouncer({
      stop: vi.fn().mockRejectedValue(new Error("nothing playing")),
      speak,
      onError,
    });

    pronounce("hello", "en");
    await vi.waitFor(() => {
      expect(speak).toHaveBeenCalledWith("hello", "en");
      expect(onError).toHaveBeenCalledTimes(1);
    });
  });
});
