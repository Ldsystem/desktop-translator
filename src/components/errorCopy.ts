import type { AppErrorCode } from "../contracts/ipc";

/** Operator-facing copy shared by every surface that reports a failure. */
export const errorCopy: Record<AppErrorCode, { title: string; guidance: string }> = {
  "permission-denied": {
    title: "Accessibility Permission Required",
    guidance: "Allow selection access in System Settings.",
  },
  "unsupported-control": {
    title: "Text Control Not Supported",
    guidance: "Select text in a standard editable or document control.",
  },
  "no-selection": {
    title: "No Text Selected",
    guidance: "Select text, then try again.",
  },
  "missing-credential": {
    title: "API Key Required",
    guidance: "Save an API key in Settings.",
  },
  "invalid-credential": {
    title: "API Key Not Accepted",
    guidance: "Replace the API key in Settings.",
  },
  "api-restricted": {
    title: "API Access Restricted",
    guidance: "Allow translation access for this API key.",
  },
  "billing-required": {
    title: "Billing Required",
    guidance: "Enable billing for the translation service.",
  },
  "quota-exceeded": {
    title: "Translation Quota Reached",
    guidance: "Check your service quota, then retry.",
  },
  offline: {
    title: "You’re Offline",
    guidance: "Reconnect to the internet, then retry.",
  },
  timeout: {
    title: "Translation Timed Out",
    guidance: "Check your connection, then retry.",
  },
  "service-unavailable": {
    title: "Translation Service Unavailable",
    guidance: "Wait a moment, then retry.",
  },
  "invalid-language-pair": {
    title: "Language Pair Not Supported",
    guidance: "Choose a different source or target language.",
  },
  internal: {
    title: "Translation Failed",
    guidance: "Dismiss this result and try again.",
  },
};
