# macOS development signing

## The problem

Granting "Always Allow" on the Keychain prompt for the Google Cloud Translation
API key does not stop the prompt from returning after the next rebuild.

macOS does not remember *an application*; it records the **code identity** of
the program that asked, on the keychain item's access-control list. A Cargo
build is ad-hoc, linker-signed:

```
Identifier=desktop_translator-035a6f343c2115fc
Signature=adhoc
CDHash=4aeec1fa4d2be26098233a618818e0da100737b1
```

For an ad-hoc signature that identity is essentially the hash of those exact
bytes, and the identifier carries a build hash of its own. Every rebuild
relinks and produces a different one, so each grant applies to a binary that no
longer exists. The same mechanism governs the Accessibility (TCC) grant, which
is why that also has to be re-approved.

## The fix

Sign every development build with one self-signed certificate. The recorded
identity then becomes the certificate plus a fixed bundle identifier, which is
stable across rebuilds, so a single grant holds.

Run once per machine:

```sh
./tools/macos/create-dev-signing-identity.sh
```

macOS asks for your login password twice, to trust the new certificate for code
signing and to let `codesign` use its private key unattended.

Signing is applied automatically from then on. `tauri dev` launches the app
through `cargo run`, so `src-tauri/.cargo/config.toml` registers
`tools/macos/sign-and-run.sh` as the Cargo runner: it signs the freshly linked
binary and then executes it. A machine without the identity still builds and
runs, it just keeps the ad-hoc identity and the repeated prompts.

After setup, the next launch prompts once more for the API key and for
Accessibility permission. Those grants then persist.

## Release builds

`tauri build` uses `cargo build`, which the runner does not cover, so bundled
builds are still ad-hoc. That is fine for local testing but means shipped
updates would re-prompt every user, because each release has a different
ad-hoc identity. Distribution needs a real Developer ID identity configured
under `bundle.macOS.signingIdentity` in `src-tauri/tauri.conf.json`.
