#!/bin/bash
# Package existing CI-built binaries. This script never invokes a compiler.
set -euo pipefail

if [[ "$(uname -s)" != Darwin ]]; then
  echo "Native .app packaging requires macOS." >&2
  exit 1
fi

binary_dir="${1:-target/release}"
output_dir="${2:-dist}"
app="$output_dir/RelayPost.app"
for binary in relaypost relay-desktop; do
  if [[ ! -x "$binary_dir/$binary" ]]; then
    echo "Missing prebuilt executable: $binary_dir/$binary" >&2
    exit 1
  fi
done
if [[ -e "$app" ]]; then
  echo "Refusing to replace existing application: $app" >&2
  exit 1
fi
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary_dir/relay-desktop" "$app/Contents/MacOS/relay-desktop"
cp "$binary_dir/relaypost" "$app/Contents/MacOS/relaypost"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>relay-desktop</string>
<key>CFBundleIdentifier</key><string>app.relaypost.desktop</string>
<key>CFBundleName</key><string>RelayPost</string>
<key>CFBundleDisplayName</key><string>RelayPost</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>CFBundleVersion</key><string>1</string>
<key>LSMinimumSystemVersion</key><string>12.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
plutil -lint "$app/Contents/Info.plist"
# Ad-hoc signing lets the arm64 executable carry a local signature. It does not
# supply an Apple Developer identity, notarization, or Gatekeeper approval.
codesign --force --sign - "$app/Contents/MacOS/relaypost"
codesign --force --sign - "$app/Contents/MacOS/relay-desktop"
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"
archive="$output_dir/relaypost-macos-arm64.zip"
ditto -c -k --sequesterRsrc --keepParent "$app" "$archive"
(cd "$output_dir" && shasum -a 256 relaypost-macos-arm64.zip > relaypost-macos-arm64.zip.sha256)
echo "Packaged $archive (ad-hoc signed, not notarized)."
