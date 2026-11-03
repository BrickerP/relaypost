#!/bin/bash
# Package existing CI-built binaries. This script never invokes a compiler.
set -euo pipefail

if [[ "$(uname -s)" != Darwin || "$(uname -m)" != arm64 ]]; then
  echo "Native .app packaging requires Apple Silicon macOS." >&2
  exit 1
fi
binary_dir="${1:-target/release}"
output_dir="${2:-dist}"
metadata="${3:?Usage: package-macos.sh BINARY_DIR OUTPUT_DIR CARGO_METADATA_JSON}"
version="$(python3 scripts/release-metadata.py version)"
name="relaypost-$version-macos-arm64"
package="$output_dir/$name"
app="$package/RelayPost.app"
for binary in relaypost relay-desktop; do
  test -x "$binary_dir/$binary" || { echo "Missing prebuilt executable: $binary_dir/$binary" >&2; exit 1; }
done
python3 scripts/release-metadata.py prepare --target aarch64-apple-darwin --metadata "$metadata" --output "$package"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary_dir/relay-desktop" "$app/Contents/MacOS/relay-desktop"
cp "$binary_dir/relaypost" "$app/Contents/MacOS/relaypost"
for file in RELEASE.json BUILD_INFO LICENSE README.md QUICK_START.md INSTALL.md DEMO.md RELEASE.md THIRD_PARTY_NOTICES.md DEPENDENCIES.json BUNDLED_FONTS.json; do
  cp "$package/$file" "$app/Contents/Resources/$file"
done
if [[ -d "$package/licenses" ]]; then cp -R "$package/licenses" "$app/Contents/Resources/licenses"; fi
python3 - "$package" <<'PY'
import json
from pathlib import Path
import sys

package = Path(sys.argv[1])
manifest = json.loads((package / "BUNDLED_FONTS.json").read_text())
binary = (package / "RelayPost.app/Contents/MacOS/relay-desktop").read_bytes()
for file in manifest["files"]:
    if file["kind"] == "font":
        if Path(file["path"]).read_bytes() not in binary:
            raise SystemExit(f"Prebuilt desktop executable does not embed {file['path']}")
PY
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>relay-desktop</string>
<key>CFBundleIdentifier</key><string>app.relaypost.desktop</string>
<key>CFBundleName</key><string>RelayPost</string>
<key>CFBundleDisplayName</key><string>RelayPost</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>CFBundleVersion</key><string>$version</string>
<key>LSMinimumSystemVersion</key><string>12.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
plutil -lint "$app/Contents/Info.plist"
# This signature provides bundle integrity, not Developer ID or Gatekeeper approval.
codesign --force --sign - "$app/Contents/MacOS/relaypost"
codesign --force --sign - "$app/Contents/MacOS/relay-desktop"
codesign --force --sign - "$app"
python3 scripts/release-metadata.py checksums "$package"
bash scripts/verify-package.sh "$package" "${RELAYPOST_SOURCE_SHA:-$(git rev-parse HEAD)}"
archive="$output_dir/$name.zip"
test ! -e "$archive"
ditto -c -k --sequesterRsrc --keepParent "$package" "$archive"
(cd "$output_dir" && shasum -a 256 "$name.zip" > "$name.zip.sha256")
cp "$package/RELEASE.json" "$output_dir/$name.RELEASE.json"
echo "Packaged $archive (ad-hoc signed, not notarized)."
