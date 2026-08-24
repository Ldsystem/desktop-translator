import { describe, expect, it } from "vitest";

import fixtures from "../contracts/fixtures.json";
import type { TranslationResult } from "../contracts/ipc";
import { initialOverlayState, reduceOverlayState } from "./overlayMachine";

describe("overlay state machine", () => {
  it("moves from idle to the newest visible selection", () => {
    const resolving = reduceOverlayState(initialOverlayState, {
      type: "pointer-up",
      requestId: 1,
    });
    const visible = reduceOverlayState(resolving, {
      type: "selection-resolved",
      requestId: 1,
      selection: fixtures.selection,
    });

    expect(visible).toEqual({
      mode: "button-visible",
      selection: fixtures.selection,
      generation: 1,
    });
  });

  it("hides an existing button on the next pointer down", () => {
    const visible = {
      mode: "button-visible" as const,
      selection: fixtures.selection,
      generation: 1,
    };

    expect(reduceOverlayState(visible, { type: "pointer-down" })).toEqual({
      mode: "pointer-down",
      latestSelectionId: fixtures.selection.id,
      generation: 2,
    });
  });

  it("discards stale translation results", () => {
    const translating = {
      mode: "translating" as const,
      selection: fixtures.selection,
      generation: 1,
    };
    const staleResult: TranslationResult = {
      ...fixtures.translationResult,
      partOfSpeech: "adjective",
      selectionId: fixtures.selection.id - 1,
    };

    expect(
      reduceOverlayState(translating, {
        type: "translation-resolved",
        result: staleResult,
      }),
    ).toBe(translating);
  });

  it("returns to disabled and drops in-memory content", () => {
    const visible = {
      mode: "button-visible" as const,
      selection: fixtures.selection,
      generation: 1,
    };

    expect(reduceOverlayState(visible, { type: "disable" })).toEqual({
      mode: "disabled",
    });
  });

  it("does not revive a dismissed or invalidated selection", () => {
    const resolving = reduceOverlayState(initialOverlayState, {
      type: "pointer-up",
      requestId: 1,
    });
    const dismissed = reduceOverlayState(resolving, { type: "dismiss" });
    const late = reduceOverlayState(dismissed, {
      type: "selection-resolved",
      requestId: 1,
      selection: fixtures.selection,
    });

    expect(late).toBe(dismissed);
  });

  it("accepts a new request after the user dismisses a visible result", () => {
    const visible = {
      mode: "button-visible" as const,
      selection: fixtures.selection,
      generation: 1,
    };
    const dismissed = reduceOverlayState(visible, { type: "dismiss" });
    const next = reduceOverlayState(dismissed, {
      type: "pointer-up",
      requestId: 2,
    });

    expect(next).toMatchObject({
      mode: "resolving-selection",
      requestId: 2,
    });
  });

  it("coalesces pending work to the newest request", () => {
    const first = reduceOverlayState(initialOverlayState, {
      type: "pointer-up",
      requestId: 1,
    });
    const newest = reduceOverlayState(first, {
      type: "pointer-up",
      requestId: 2,
    });
    const stale = reduceOverlayState(newest, {
      type: "selection-resolved",
      requestId: 1,
      selection: fixtures.selection,
    });

    expect(stale).toBe(newest);
    expect(newest).toMatchObject({
      mode: "resolving-selection",
      requestId: 2,
      generation: 2,
    });
  });
});
