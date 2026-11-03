#!/usr/bin/env python3
"""Package separately linked QA programs from the same checked desktop source."""

import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import sys
import uuid


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def executable_uuid(path):
    lines = subprocess.check_output(["dwarfdump", "--uuid", str(path)], text=True).splitlines()
    if len(lines) != 1 or "(arm64)" not in lines[0]:
        raise SystemExit("QA executable must be one Apple Silicon image.")
    return str(uuid.UUID(lines[0].split()[1]))


def main():
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        raise SystemExit("GUI QA packaging requires Apple Silicon macOS.")
    source = Path(sys.argv[1]).resolve()
    output = Path(sys.argv[2]).resolve()
    binaries = Path(sys.argv[3]).resolve()
    metadata = json.loads((source / "Contents/Resources/RELEASE.json").read_text())
    if metadata["source_sha"] != os.environ["RELAYPOST_SOURCE_SHA"]:
        raise SystemExit("QA source does not match the checked CI commit.")
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(source)], check=True)
    binary_digest = digest(source / "Contents/MacOS/relay-desktop")
    canonical_uuid = executable_uuid(source / "Contents/MacOS/relay-desktop")
    seen_uuids = {canonical_uuid}
    output.mkdir(parents=True, exist_ok=False)
    manifest = {
        "kind": "relaypost.gui-qa",
        "source_sha": metadata["source_sha"],
        "version": metadata["version"],
        "desktop_sha256": binary_digest,
        "canonical_uuid": canonical_uuid,
        "build_difference": "Distinct rustc metadata salts; identical source, features and dependency lock.",
        "purpose": "Two isolated simultaneous GUI instances; not public release apps.",
        "bundles": [],
    }
    for role in ("sender", "receiver"):
        name = f"RelayPost QA {role.title()}"
        bundle = output / f"{name}.app"
        subprocess.run(["ditto", str(source), str(bundle)], check=True)
        binary = binaries / role / "relay-desktop"
        build_uuid = executable_uuid(binary)
        if build_uuid in seen_uuids:
            raise SystemExit("QA programs must have distinct main executable UUIDs (Apple TN3178).")
        seen_uuids.add(build_uuid)
        shutil.copy2(binary, bundle / "Contents/MacOS/relay-desktop")
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
        if executable_uuid(bundle / "Contents/MacOS/relay-desktop") != build_uuid:
            raise SystemExit("QA signing changed its linked executable UUID.")
        manifest["bundles"].append({"role": role, "path": bundle.name, "bundle_id": plist["CFBundleIdentifier"], "uuid": build_uuid, "rustc_metadata": f"relaypost_qa_{role}_{metadata['source_sha']}", "desktop_sha256": digest(bundle / "Contents/MacOS/relay-desktop")})
    (output / "QA.json").write_text(json.dumps(manifest, indent=2) + "\n")
    sums = [f"{digest(path)}  {path.relative_to(output)}" for path in sorted(output.rglob("*")) if path.is_file()]
    (output / "SHA256SUMS").write_text("\n".join(sums) + "\n")
    archive = output.with_suffix(".zip")
    if archive.exists():
        raise SystemExit("QA archive already exists.")
    subprocess.run(["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(output), str(archive)], check=True)
    archive.with_suffix(".zip.sha256").write_text(f"{digest(archive)}  {archive.name}\n")
    print(f"Packaged {archive.name} from {metadata['source_sha']} with independent linked UUIDs and signatures.")


if __name__ == "__main__":
    main()
