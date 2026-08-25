# Platform Qualification Matrix

Updated: 2026-08-24

This matrix separates deterministic automated evidence from real-host evidence. A row is
`Passed` only when the stated fixture was exercised on the named host. `Pending` is not
treated as release evidence.

## Hosts

- **macOS host:** macOS 26.5.2 (25F84), Apple silicon arm64, 16 GiB RAM,
  Node.js 20.20.0, pnpm 10.30.2, rustc 1.97.1.
- **Windows host:** Windows 11 25H2 x64 (build 26200.9168), unelevated session,
  Node.js via Volta, pnpm 11.20.0, rustc 1.98.0. Display topology is a single
  primary 1536×960 32-bit surface (`\\.\DISPLAY1`); mixed-DPI and multi-monitor
  placement were **not** available on this host.

> Compile against locked `windows` 0.62 is repaired. Gating Windows CI now runs
> renderer build, `cargo fmt`, Clippy `-D warnings`, `cargo test`, NSIS package,
> and `tools/release/audit-windows-bundle.ps1`. Real-host UI Automation selection,
> overlay against a second app, SAPI, credential prompt, start-at-login, and
> Vocabulary Study chrome remain **manual-only** until those fixtures are run
> interactively. See [`windows-signing-and-supply-chain.md`](windows-signing-and-supply-chain.md).

## Deterministic gates

| Gate | macOS result | Windows result | Evidence |
| --- | --- | --- | --- |
| Frontend typecheck, tests, production build | Passed | Passed | `pnpm check` on this Windows host; 12 files and 80 tests passed including the NSIS audit suite |
| Rust unit and integration suite | Passed | Passed | `cargo test --manifest-path src-tauri/Cargo.toml`; 135 passed, 1 ignored network fixture |
| Strict Rust lint and formatting | Passed | Passed | `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings` |
| Native unsigned release build | Passed | Passed | `pnpm tauri build --bundles nsis`; `Desktop Translator_0.3.0_x64-setup.exe` 3 589 056 bytes SHA-256 `9DD565CF2FA5487D90EDD3B8638C92C76B88C32CBEBED1054A885CD0D799DCC5`; exe 9 397 248 bytes SHA-256 `FF33835546FF6A6F6D4DAE30241DA3871956D6C9D08C3D46AE601AFB53A21ECF` |
| Installer silent install / launch / reinstall / uninstall | n/a | Passed | NSIS `/S` current-user install to `%LOCALAPPDATA%\Desktop Translator`, process started, `/S` reinstall, `uninstall.exe /S` removed the directory |
| Performance harness parser and budgets | Passed | Passed (parser) | `pnpm test:perf` is included in `pnpm check`; warmed RSS/latency on Windows remain pending |

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
| Mixed-DPI, multi-monitor, and work-area edges | Automated | Topology limited | Windows unit placement tests pass; this host has one 1536×960 display so mixed-DPI/multi-monitor was not exercised |
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

Windows compile, NSIS packaging, gating CI, and silent installer smoke passed on
Windows 11 25H2 x64. Interactive UI Automation selection, overlay, SAPI,
credential prompt, start-at-login, and Vocabulary Study chrome are still
manual-only.

Task 008 cannot be accepted until:

1. remaining Windows real-host UI fixture rows are executed on Windows 10/11;
2. pending macOS real-host fixtures are executed;
3. warmed resource measurements and externally observed latency samples pass;
4. manual theme, reduced-motion, and real multi-display checks are recorded.

Signing, Windows Authenticode, Apple Developer ID, and notarization are Task 009
external prerequisites and must not place credential material in this repository.
