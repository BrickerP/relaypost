#!/bin/bash
# Package an existing CI-built Linux CLI. No compiler is invoked.
set -euo pipefail
[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]]
binary_dir="${1:-target/release}"
output_dir="${2:-dist}"
metadata="${3:?Usage: package-linux.sh BINARY_DIR OUTPUT_DIR CARGO_METADATA_JSON}"
version="$(python3 scripts/release-metadata.py version)"
name="relaypost-$version-linux-x86_64"
package="$output_dir/$name"
test -x "$binary_dir/relaypost"
python3 scripts/release-metadata.py prepare --target x86_64-unknown-linux-gnu --metadata "$metadata" --output "$package"
mkdir "$package/bin"
cp "$binary_dir/relaypost" "$package/bin/relaypost"
python3 scripts/release-metadata.py checksums "$package"
bash scripts/verify-package.sh "$package" "${RELAYPOST_SOURCE_SHA:-$(git rev-parse HEAD)}"
archive="$output_dir/$name.tar.gz"
test ! -e "$archive"
source_time="$(git show -s --format=%ct HEAD)"
tar --sort=name --mtime="@$source_time" --owner=0 --group=0 --numeric-owner -C "$output_dir" -cf - "$name" | gzip -n > "$archive"
(cd "$output_dir" && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
cp "$package/RELEASE.json" "$output_dir/$name.RELEASE.json"
echo "Packaged $archive."
