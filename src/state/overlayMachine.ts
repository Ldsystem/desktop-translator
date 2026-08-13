import type {
  AppError,
  SelectionSnapshot,
  TranslationResult,
} from "../contracts/ipc";

/** Complete UI lifecycle state; content-bearing variants own ephemeral text. */
export type OverlayState =
  | { mode: "disabled" }
  | { mode: "idle"; latestSelectionId: number; generation: number }
  | { mode: "pointer-down"; latestSelectionId: number; generation: number }
  | {
      mode: "resolving-selection";
      latestSelectionId: number;
      generation: number;
      requestId: number;
    }
  | { mode: "button-visible"; selection: SelectionSnapshot; generation: number }
  | { mode: "translating"; selection: SelectionSnapshot; generation: number }
  | {
      mode: "result-visible";
      selection: SelectionSnapshot;
      result: TranslationResult;
      generation: number;
    }
  | {
      mode: "error-visible";
      selection: SelectionSnapshot;
      error: AppError;
      generation: number;
    };

/** Events accepted by the pure newest-request-wins overlay reducer. */
export type OverlayEvent =
  | { type: "enable" }
  | { type: "disable" }
  | { type: "pointer-down" }
  | { type: "pointer-up"; requestId: number }
  | {
      type: "selection-resolved";
      requestId: number;
      selection: SelectionSnapshot;
    }
  | { type: "selection-rejected"; requestId: number }
  | { type: "translate" }
  | { type: "translation-resolved"; result: TranslationResult }
  | { type: "translation-failed"; selectionId: number; error: AppError }
  | { type: "dismiss" };

/** Enabled startup state with no captured selection. */
export const initialOverlayState: OverlayState = {
  mode: "idle",
  latestSelectionId: 0,
  generation: 0,
};

function latestSelectionId(state: OverlayState): number {
  if ("selection" in state) {
    return state.selection.id;
  }

  return "latestSelectionId" in state ? state.latestSelectionId : 0;
}

function generation(state: OverlayState): number {
  return "generation" in state ? state.generation : 0;
}

/**
 * Applies one coordinator event.
 * Matching request generations prevent invalidated asynchronous work from reviving.
 */
export function reduceOverlayState(
  state: OverlayState,
  event: OverlayEvent,
): OverlayState {
  if (event.type === "disable") {
    return { mode: "disabled" };
  }

  if (state.mode === "disabled") {
    return event.type === "enable"
      ? { mode: "idle", latestSelectionId: 0, generation: 0 }
      : state;
  }

  switch (event.type) {
    case "enable":
      return state;
    case "pointer-down":
      return {
        mode: "pointer-down",
        latestSelectionId: latestSelectionId(state),
        generation: generation(state) + 1,
      };
    case "pointer-up":
      return {
        mode: "resolving-selection",
        latestSelectionId: latestSelectionId(state),
        generation: generation(state) + 1,
        requestId: event.requestId,
      };
    case "selection-resolved":
      if (
        state.mode !== "resolving-selection" ||
        event.requestId !== state.requestId ||
        event.selection.id <= state.latestSelectionId
      ) {
        return state;
      }
      return {
        mode: "button-visible",
        selection: event.selection,
        generation: state.generation,
      };
    case "selection-rejected":
      if (state.mode !== "resolving-selection" || event.requestId !== state.requestId) {
        return state;
      }
      return {
        mode: "idle",
        latestSelectionId: state.latestSelectionId,
        generation: state.generation,
      };
    case "dismiss":
      return {
        mode: "idle",
        latestSelectionId: latestSelectionId(state),
        generation: generation(state) + 1,
      };
    case "translate":
      return state.mode === "button-visible" ||
        state.mode === "result-visible" ||
        state.mode === "error-visible"
        ? {
            mode: "translating",
            selection: state.selection,
            generation: state.generation,
          }
        : state;
    case "translation-resolved":
      return state.mode === "translating" &&
        event.result.selectionId === state.selection.id
        ? {
            mode: "result-visible",
            selection: state.selection,
            result: event.result,
            generation: state.generation,
          }
        : state;
    case "translation-failed":
      return state.mode === "translating" &&
        event.selectionId === state.selection.id
        ? {
            mode: "error-visible",
            selection: state.selection,
            error: event.error,
            generation: state.generation,
          }
        : state;
  }
}
