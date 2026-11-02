#!/bin/bash
# Verify an extracted distribution without Rust, Python, or development tools.
set -euo pipefail

package_dir="${1:-.}"
expected_sha="${2:-}"
cd "$package_dir"
if command -v sha256sum >/dev/null 2>&1; then
  sha256sum --check --strict SHA256SUMS
else
  shasum -a 256 --check SHA256SUMS
fi
version="$(sed -n 's/^version=//p' BUILD_INFO)"
source_sha="$(sed -n 's/^source_sha=//p' BUILD_INFO)"
target="$(sed -n 's/^target=//p' BUILD_INFO)"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
[[ "$source_sha" =~ ^[0-9a-f]{40}$ ]]
if [[ -n "$expected_sha" && "$source_sha" != "$expected_sha" ]]; then
  echo "Package source commit does not match the expected source." >&2
  exit 1
fi
for file in README.md QUICK_START.md INSTALL.md DEMO.md RELEASE.md LICENSE RELEASE.json DEPENDENCIES.json THIRD_PARTY_NOTICES.md; do
  test -s "$file"
done
case "$target" in
  aarch64-apple-darwin)
    [[ "$(uname -s)" == Darwin && "$(uname -m)" == arm64 ]]
    app="RelayPost.app"
    binary="$app/Contents/MacOS/relaypost"
    test -x "$app/Contents/MacOS/relay-desktop"
    test -x "$binary"
    test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist")" = "$version"
    test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$app/Contents/Info.plist")" = "$version"
    cmp RELEASE.json "$app/Contents/Resources/RELEASE.json"
    codesign --verify --deep --strict "$app"
    ;;
  x86_64-unknown-linux-gnu)
    [[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]]
    binary="bin/relaypost"
    test -x "$binary"
    ;;
  *) echo "Unsupported package target: $target" >&2; exit 1 ;;
esac
actual_version="$("./$binary" --version)"
test "$actual_version" = "relaypost $version"
echo "Verified RelayPost $version; source $source_sha; target $target."
echo "Checksums identify package bytes; an ad-hoc signature does not establish publisher identity."
