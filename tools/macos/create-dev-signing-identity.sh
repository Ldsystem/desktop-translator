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

# Guards the PKCS#12 bundle for the seconds it exists on disk; Security.framework
# rejects an empty one. It is discarded with the temporary directory.
TRANSIT_PASSWORD="$(openssl rand -hex 16)"

echo "Generating a self-signed code-signing certificate…"
openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -keyout "$workdir/key.pem" \
  -out "$workdir/cert.pem" \
  -subj "/CN=$IDENTITY" \
  -addext "basicConstraints=critical,CA:true" \
  -addext "keyUsage=critical,digitalSignature,keyCertSign" \
  -addext "extendedKeyUsage=critical,codeSigning" >/dev/null 2>&1

# OpenSSL 3 defaults to an AES/SHA-256 PKCS#12 MAC that Security.framework
# cannot verify, so ask for the legacy encoding where that option exists.
# LibreSSL, which is what /usr/bin/openssl is, already emits the old encoding
# and rejects the flag.
if ! openssl pkcs12 -export -legacy \
  -inkey "$workdir/key.pem" \
  -in "$workdir/cert.pem" \
  -out "$workdir/identity.p12" \
  -passout pass:"$TRANSIT_PASSWORD" >/dev/null 2>&1; then
  openssl pkcs12 -export \
    -inkey "$workdir/key.pem" \
    -in "$workdir/cert.pem" \
    -out "$workdir/identity.p12" \
    -passout pass:"$TRANSIT_PASSWORD" >/dev/null 2>&1
fi

# -T pre-authorizes codesign on the private key's access-control list.
security import "$workdir/identity.p12" -k "$KEYCHAIN" -P "$TRANSIT_PASSWORD" \
  -T /usr/bin/codesign -T /usr/bin/security >/dev/null

echo "Trusting the certificate for code signing (login password required)…"
security add-trusted-cert -p codeSign -k "$KEYCHAIN" "$workdir/cert.pem"

# The import above already grants codesign access to the private key. Widening
# the partition list is belt-and-braces for systems that still consult it, and
# it needs the login password on stdin, so a failure here is not fatal: the
# worst case is one "codesign wants to sign using key" prompt you can approve.
if [[ -t 0 ]]; then
  echo "Optional: allowing codesign to use the key unattended."
  echo "Enter your login password, or press Ctrl-D to skip."
  security set-key-partition-list \
    -S apple-tool:,apple:,codesign: -s "$KEYCHAIN" >/dev/null 2>&1 ||
    echo "Skipped; approve the codesign prompt if it appears." >&2
fi

echo
echo "Done. Dev builds are now signed as \"$IDENTITY\"."
echo "The next launch will ask once for the API key and for Accessibility"
echo "permission; those grants then survive every later rebuild."
