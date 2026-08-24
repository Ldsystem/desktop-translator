export type WindowMode = "overlay" | "quick" | "settings" | "study";

/** Resolves the surface selected by the native window URL without native coupling. */
export function getWindowMode(location: Pick<Location, "search" | "pathname"> = window.location): WindowMode {
  const requestedMode = new URLSearchParams(location.search).get("mode");
  if (requestedMode === "overlay" || location.pathname.endsWith("/overlay")) {
    return "overlay";
  }
  if (requestedMode === "quick" || location.pathname.endsWith("/quick")) {
    return "quick";
  }
  if (requestedMode === "study" || location.pathname.endsWith("/study")) {
    return "study";
  }
  return "settings";
}
