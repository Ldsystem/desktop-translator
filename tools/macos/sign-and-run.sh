#!/usr/bin/env bash
#
# Cargo runner for macOS. Cargo invokes it with the freshly built executable and
# its arguments, which is the only point between linking and launch where the
# binary can be given a stable code identity.
#
# Signing is best-effort: a machine without the development identity still runs
# the binary, it just gets the ad-hoc identity and the repeated system prompts
# that come with it.

set -euo pipefail

BINARY="$1"
shift

IDENTITY="Desktop Translator Dev"
# Matches `identifier` in tauri.conf.json so dev and bundled builds present the
# same code identity to the Keychain.
BUNDLE_ID="com.desktoptranslator.desktop"

if [[ "$(uname -s)" == "Darwin" ]]; then
  if security find-identity -v -p codesigning 2>/dev/null | grep -qF "$IDENTITY"; then
    codesign --force --sign "$IDENTITY" --identifier "$BUNDLE_ID" "$BINARY" 2>/dev/null ||
      echo "warning: could not sign $BINARY; macOS will prompt for the API key again" >&2
  else
    echo "note: run ./tools/macos/create-dev-signing-identity.sh to stop repeated Keychain prompts" >&2
  fi
fi

exec "$BINARY" "$@"
