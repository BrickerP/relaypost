#!/usr/bin/env python3
"""Package two identifiable QA apps from the same CI-built desktop executable."""

import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import sys
import tempfile


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def unsigned_digest(path):
    # Re-signing changes the embedded signature, including its Info.plist hash.
    # Remove signatures only from throwaway copies to compare the compiled bytes.
    with tempfile.TemporaryDirectory(prefix="relaypost-code-check-") as directory:
        copy = Path(directory) / "relay-desktop"
        shutil.copy2(path, copy)
        subprocess.run(["codesign", "--remove-signature", str(copy)], check=True)
        return digest(copy)


def main():
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        raise SystemExit("GUI QA packaging requires Apple Silicon macOS.")
    source = Path(sys.argv[1]).resolve()
    output = Path(sys.argv[2]).resolve()
    metadata = json.loads((source / "Contents/Resources/RELEASE.json").read_text())
    if metadata["source_sha"] != os.environ["RELAYPOST_SOURCE_SHA"]:
        raise SystemExit("QA source does not match the checked CI commit.")
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(source)], check=True)
    binary_digest = digest(source / "Contents/MacOS/relay-desktop")
    compiled_digest = unsigned_digest(source / "Contents/MacOS/relay-desktop")
    output.mkdir(parents=True, exist_ok=False)
    manifest = {
        "kind": "relaypost.gui-qa",
        "source_sha": metadata["source_sha"],
        "version": metadata["version"],
        "desktop_sha256": binary_digest,
        "unsigned_desktop_sha256": compiled_digest,
        "purpose": "Two isolated simultaneous GUI instances; not public release apps.",
        "bundles": [],
    }
    for role in ("sender", "receiver"):
        name = f"RelayPost QA {role.title()}"
        bundle = output / f"{name}.app"
        subprocess.run(["ditto", str(source), str(bundle)], check=True)
        plist_path = bundle / "Contents/Info.plist"
        with plist_path.open("rb") as handle:
            plist = plistlib.load(handle)
        plist["CFBundleIdentifier"] = f"app.relaypost.qa.{role}"
        plist["CFBundleName"] = name
        plist["CFBundleDisplayName"] = name
        with plist_path.open("wb") as handle:
            plistlib.dump(plist, handle)
        subprocess.run(["codesign", "--force", "--sign", "-", str(bundle)], check=True)
        subprocess.run(["codesign", "--verify", "--deep", "--strict", str(bundle)], check=True)
        if unsigned_digest(bundle / "Contents/MacOS/relay-desktop") != compiled_digest:
            raise SystemExit("QA packaging changed the checked desktop executable.")
        manifest["bundles"].append({"role": role, "path": bundle.name, "bundle_id": plist["CFBundleIdentifier"], "desktop_sha256": digest(bundle / "Contents/MacOS/relay-desktop")})
    (output / "QA.json").write_text(json.dumps(manifest, indent=2) + "\n")
    sums = [f"{digest(path)}  {path.relative_to(output)}" for path in sorted(output.rglob("*")) if path.is_file()]
    (output / "SHA256SUMS").write_text("\n".join(sums) + "\n")
    archive = output.with_suffix(".zip")
    if archive.exists():
        raise SystemExit("QA archive already exists.")
    subprocess.run(["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(output), str(archive)], check=True)
    archive.with_suffix(".zip.sha256").write_text(f"{digest(archive)}  {archive.name}\n")
    print(f"Packaged {archive.name} from {metadata['source_sha']} with matching compiled bytes and separate signatures.")


if __name__ == "__main__":
    main()
