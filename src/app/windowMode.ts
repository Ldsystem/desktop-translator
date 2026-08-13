export type WindowMode = "overlay" | "settings";

/** Resolves the surface selected by the native window URL without native coupling. */
export function getWindowMode(location: Pick<Location, "search" | "pathname"> = window.location): WindowMode {
  const requestedMode = new URLSearchParams(location.search).get("mode");
  if (requestedMode === "overlay" || location.pathname.endsWith("/overlay")) {
    return "overlay";
  }
  return "settings";
}
