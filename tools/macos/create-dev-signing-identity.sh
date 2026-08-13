#!/usr/bin/env bash
#
# Creates the stable self-signed code-signing identity that macOS development
# builds are signed with.
#
# Without it, `cargo` produces an ad-hoc, linker-signed binary whose code
# identity is the hash of that exact build. Keychain and TCC grants are recorded
# against that identity, so every rebuild invalidates them and macOS asks for
# the API key and Accessibility permission again. Signing every build with one
# certificate gives them a single identity to remember.
#
# Run once:
#
#   ./tools/macos/create-dev-signing-identity.sh
#
# macOS will ask for your login password twice: once to trust the certificate
# for code signing, and once to let `codesign` use its private key unattended.

set -euo pipefail

IDENTITY="Desktop Translator Dev"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This script only applies to macOS." >&2
  exit 1
fi

if security find-identity -v -p codesigning | grep -qF "$IDENTITY"; then
  echo "Signing identity \"$IDENTITY\" already exists. Nothing to do."
  exit 0
fi

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

echo "Generating a self-signed code-signing certificate…"
openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -keyout "$workdir/key.pem" \
  -out "$workdir/cert.pem" \
  -subj "/CN=$IDENTITY" \
  -addext "basicConstraints=critical,CA:true" \
  -addext "keyUsage=critical,digitalSignature,keyCertSign" \
  -addext "extendedKeyUsage=critical,codeSigning" >/dev/null 2>&1

openssl pkcs12 -export \
  -inkey "$workdir/key.pem" \
  -in "$workdir/cert.pem" \
  -out "$workdir/identity.p12" \
  -passout pass: >/dev/null 2>&1

# -T pre-authorizes codesign on the private key's access-control list.
security import "$workdir/identity.p12" -k "$KEYCHAIN" -P "" \
  -T /usr/bin/codesign -T /usr/bin/security >/dev/null

echo "Trusting the certificate for code signing (login password required)…"
security add-trusted-cert -p codeSign -k "$KEYCHAIN" "$workdir/cert.pem"

# The access-control list above is the legacy mechanism; the partition list is
# what actually suppresses the "codesign wants to sign using key" prompt.
echo "Allowing codesign to use the key unattended (login password required)…"
security set-key-partition-list \
  -S apple-tool:,apple:,codesign: -s "$KEYCHAIN" >/dev/null

echo
echo "Done. Dev builds are now signed as \"$IDENTITY\"."
echo "The next launch will ask once for the API key and for Accessibility"
echo "permission; those grants then survive every later rebuild."
