# Platform Qualification Matrix

Updated: 2026-08-13

This matrix separates deterministic automated evidence from real-host evidence. A row is
`Passed` only when the stated fixture was exercised on the named host. `Pending` is not
treated as release evidence.

## Hosts

- **macOS host:** macOS 26.5.2 (25F84), Apple silicon arm64, 16 GiB RAM,
  Node.js 20.20.0, pnpm 10.30.2, rustc 1.97.1.
- **Windows host:** pending access to a Windows 10/11 x64 host with a normal
  unelevated session and UI Automation enabled.

## Deterministic gates

| Gate | macOS result | Windows result | Evidence |
| --- | --- | --- | --- |
| Frontend typecheck, tests, production build | Passed | Pending | `pnpm check`; 6 files and 22 tests passed |
| Rust unit and integration suite | Passed | Pending | `cargo test --manifest-path src-tauri/Cargo.toml`; 73 passed and 6 explicit manual fixtures ignored |
| Strict Rust lint and formatting | Passed | Pending | `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings` |
| Native unsigned release build | Passed | Pending | `pnpm tauri build` on macOS |
| Performance harness parser and budgets | Passed | Pending | `pnpm test:perf`; process-tree parsing, percentile calculation, CLI forwarding, and multi-budget failures |

## Selection and lifecycle fixtures

| Fixture | macOS | Windows | Evidence or remaining action |
| --- | --- | --- | --- |
| Standard editable text, primary drag selection | Passed | Pending | TextEdit current-host smoke: button appeared after mouse-up without losing the selection |
| Unfocused application HTML reading surface | Passed | Pending | Outlook reading-pane selection reproduced the focus mismatch, then passed after pointer hit-testing, ancestor traversal, and text-marker support |
| Explicit-click translation and loading/result flow | Passed | Pending | TextEdit current-host smoke with Google Cloud Translation Basic |
| Auto detection and manual source correction | Passed | Pending | TextEdit current-host smoke retranslated the same selection with an explicit source |
| Source and target local speech, stop behavior | Passed | Pending | Both macOS native pronunciation actions were exercised; native availability is now reported to the renderer |
| Dismiss, reselection, disable, and quit | Passed | Pending | Current-host smoke plus state-machine, observer, and shutdown regression tests |
| Tray ownership while all windows are hidden | Passed | Pending | Current-host smoke plus retained `TrayIcon` and implicit-exit regression tests |
| Wrapped multiline selection | Pending | Pending | Run the ignored real-host multiline fixture in a standard AX/UIA control |
| Chromium/Electron document selection | Passed | Pending | Chrome web-area drag selection reproduced `no-selection` even though `AXSelectedText`, `AXSelectedTextRange`, and `AXSelectedTextMarkerRange` were all populated; the range strategy resolved text but no geometry and aborted the element. Independent range/marker fallback repaired it and the button now appears. Electron fixtures remain pending |
| Protected/secure field suppression | Pending | Pending | Run the ignored secure-field fixture and verify no overlay, log, or request |
| Unsupported canvas/PDF control | Pending | Pending | Verify no overlay when AX/UIA exposes no selected range and geometry |
| Permission denied or elevated target | Pending | Pending | Revoke macOS Accessibility and test guidance; test a Windows elevated target from an unelevated app |
| Mixed-DPI, multi-monitor, and work-area edges | Automated | Pending | Placement and macOS display-normalization tests pass; real multi-display placement remains pending |
| Display topology change | Automated | Pending | Pure placement recomputation test passes; real hot-plug remains pending |
| Sleep/wake and observer restart | Automated | Pending | Observer stop/restart lifecycle tests pass; real sleep/wake remains pending |
| Double/triple-click word or paragraph selection | Automated | Pending | macOS gesture-state tests pass; real-host fixture remains pending |

## Security and privacy inspection

| Requirement | Result | Evidence |
| --- | --- | --- |
| API key remains native and vault-backed | Passed (static/tests) | Renderer exposes only credential status and native prompt actions; settings rejection tests cover `apiKey` fields |
| Key is absent from URL and body | Passed (local capture test) | Translation adapter test observes `x-goog-api-key` only in the request header |
| No renderer persistence | Passed (static) | No `localStorage`, `sessionStorage`, or IndexedDB use exists in application source |
| No clipboard, screenshot, OCR, or history fallback | Passed (static) | No application source path invokes these capture mechanisms |
| No private-content logging | Passed (static) | The only production diagnostic prints a stable tray action error message; selected text, translations, keys, audio, and provider bodies are not logged |
| Explicit click before network transmission | Passed (integration) | Selection does not call the provider until `translate_selection` validates the visible selection |
| No explanation/dictionary feature | Passed (UI/static) | UI test rejects Gemini content; no result or settings implementation exposes explanations |
| Provider error classification and bounded retry | Passed (tests) | Invalid/restricted/billing/quota/offline/timeout/service cases and retry bounds are covered |

## Accessibility and presentation

| Check | Result | Evidence or remaining action |
| --- | --- | --- |
| Source focus preservation and non-activating overlay | Passed on TextEdit | Current-host smoke plus overlay-originated gesture and non-focus policy tests |
| Semantic control names and Escape dismissal | Passed (DOM tests) | Contextual controls have accessible labels; Escape dismissal is covered |
| Unsupported native voice indication | Passed (DOM/native tests) | Unsupported or unresolved language voices disable the corresponding speaker action |
| Keyboard-only traversal and visible focus | Automated/static | Native overlay is intentionally non-focusable to preserve source selection; pointer actions are primary for release one |
| System light/dark theme and contrast | Pending manual | Tokens implement system theme; visual contrast requires host inspection |
| Reduced motion | Pending manual | Inspect with Reduce Motion enabled |
| Long text and work-area sizing | Automated | Placement clamps to work area; manual high-content fixture remains pending |

## Performance

The Task 008 harness measures the complete application process tree after a five-minute
warm-up and a ten-minute collection interval. It reports CPU p95, peak RSS, process
count, and externally supplied mouse-up-to-button latency samples as machine-readable
JSON. Wakeup behavior is qualified by source/profiler review because portable process
wakeup counters are unavailable without privileged host tooling.

Budgets:

- enabled idle CPU: no more than 0.5% p95;
- pre-WebView RSS: no more than 80 MiB;
- one-popup RSS: no more than 150 MiB;
- mouse-up-to-button latency: no more than 150 ms p95 when native resolution is no
  more than 100 ms.

Current macOS pre-WebView release measurement:

- five-minute warm-up followed by a ten-minute collection;
- 585 process-tree samples at a nominal one-second interval;
- idle CPU p95: **0%** — passed the 0.5% budget;
- peak RSS: **68.703 MiB** — passed the 80 MiB pre-WebView budget;
- latency: pending external samples, so the complete performance gate remains failed;
- one-popup 150 MiB budget: pending a release-host popup run.

## Release blockers

Task 008 cannot be accepted until:

1. the Windows deterministic build and real-host fixture rows pass on Windows 10/11;
2. pending macOS real-host fixtures are executed;
3. warmed resource measurements and externally observed latency samples pass;
4. manual theme, reduced-motion, and real multi-display checks are recorded.

Signing, Windows Authenticode, Apple Developer ID, and notarization are Task 009
external prerequisites and must not place credential material in this repository.
