# Windows signing and software supply chain

This document separates **controls this repository already implements** from
**external certificate and account work** that must not be stored in Git.

Windows release artifacts are currently **unsigned**. That is intentional until
a publisher certificate is procured outside this repository. Do not buy, commit,
log, or embed a code-signing certificate here.

## Implemented in CI and packaging

| Control | Where it lives | Status |
| --- | --- | --- |
| Lockfile integrity | `pnpm-lock.yaml`, `src-tauri/Cargo.lock`, `pnpm install --frozen-lockfile` | Implemented |
| Gating Windows compile and package | `.github/workflows/ci.yml` Windows job | Implemented |
| NSIS x64 installer + WebView2 download bootstrapper | `src-tauri/tauri.conf.json` `bundle.windows` | Implemented |
| Bundle audit (exe, NSIS x64 setup, no developer runtimes, textbook remains embedded) | `tools/release/audit-windows-bundle.ps1` | Implemented |
| Artifact hashes | Release job writes SHA-256 for the NSIS setup and native exe | Implemented |
| Provenance attestation | `actions/attest-build-provenance` on tag/workflow_dispatch only | Implemented |
| Least-privilege installation | NSIS `installMode: currentUser` (no administrator required) | Implemented |
| Credential isolation | Native Windows Credential Manager via `keyring`; credentials never enter the WebView, settings JSON, logs, or docs | Implemented |
| Fork / pull-request isolation | `release.yml` does not run on `pull_request`; it never receives signing secrets | Implemented |
| macOS path isolation | Windows and macOS release jobs are separate; artifacts keep platform-specific names | Implemented |

Dependency review for pull requests remains the GitHub-hosted default for this
public repository. There is no committed SBOM generator yet; tag provenance
attestations are the implemented substitute.

## External prerequisites (not in this repository)

These require a human publisher identity and must use a protected GitHub
Environment or an offline ceremony. Untrusted forks must never receive them.

| Prerequisite | Purpose |
| --- | --- |
| Authenticode code-signing certificate | Publisher identity for the NSIS setup and exe |
| Certificate custody and PIN/HSM or cloud KMS | Private key never in Git, logs, or `pull_request` secrets |
| RFC 3161 timestamping authority | Signatures remain verifiable after the cert expires |
| Publisher name alignment | SmartScreen reputation accrues to one stable identity |
| SmartScreen / Microsoft reputation | New publishers are warned until reputation exists |
| Malware scanning of published installers | Optional extra gate before a production tag |
| Key rotation and revocation plan | Replace a compromised or expired cert; publish a new installer |
| Rollback | Yank or replace a GitHub Release; do not reuse a burned version |

Until those exist, document SmartScreen warnings as expected for unsigned
installers. Users may need to choose **More info → Run anyway**. That is not a
substitute for Authenticode.

## Certificate custody rules

- Store signing material only in a protected GitHub Environment or an external
  signing service.
- Grant `id-token` / `attestations` on the tag workflow; do not pass
  `WINDOWS_CERTIFICATE*` (or any analog) into pull-request jobs.
- Rotate by issuing a new cert, signing a new installer, and publishing new
  hashes. Revoke the old cert with the CA when it is compromised.
- Never put `.pfx`, passwords, or thumbprints that unlock a private key into
  `src-tauri/tauri.conf.json` on this branch.

## Related macOS note

macOS ad-hoc signing and Accessibility identity are documented in
[`macos-development-signing.md`](macos-development-signing.md). Windows does not
use that runner. Do not copy macOS signing identities into the Windows job.
