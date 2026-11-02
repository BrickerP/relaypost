#!/usr/bin/env python3
"""Describe already-built CI artifacts and collect supplied dependency notices.

No compiler, dependency resolver, or network access is invoked here. Cargo metadata
must be produced by CI for the package's actual target before invoking prepare.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tomllib
from urllib.parse import quote


ROOT = Path(__file__).resolve().parent.parent
TARGETS = {
    "aarch64-apple-darwin": {
        "platform": "macos-arm64",
        "minimum_os": "Built for macOS 12.0+; Apple Silicon only; exercised on macOS 15",
        "signing": "ad-hoc; no Developer ID signature or Apple notarization",
        "packages": ("relay-cli", "relay-desktop"),
    },
    "x86_64-unknown-linux-gnu": {
        "platform": "linux-x86_64",
        "minimum_os": "Ubuntu 24.04 x86_64 / glibc 2.39 baseline; other distributions unverified",
        "signing": "unsigned CLI archive",
        "packages": ("relay-cli",),
    },
}


def version():
    with (ROOT / "Cargo.toml").open("rb") as stream:
        value = tomllib.load(stream)["workspace"]["package"]["version"]
    if not re.fullmatch(r"\d+\.\d+\.\d+", value):
        raise SystemExit("Release packaging requires a numeric major.minor.patch version")
    return value


def source_sha():
    value = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
    ).strip()
    if not re.fullmatch(r"[0-9a-f]{40}", value):
        raise SystemExit("Expected a full Git source commit")
    expected = os.environ.get("GITHUB_SHA")
    if expected and expected != value:
        raise SystemExit("Checkout does not match GITHUB_SHA")
    return value


def collect_notices(metadata, roots, destination):
    packages = {package["id"]: package for package in metadata["packages"]}
    graph = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    pending = [
        package["id"]
        for package in packages.values()
        if package["name"] in roots and package["id"] in metadata["workspace_members"]
    ]
    if len(pending) != len(roots):
        raise SystemExit("Missing expected workspace package in dependency metadata")
    reached = set()
    while pending:
        package_id = pending.pop()
        if package_id in reached:
            continue
        reached.add(package_id)
        for dependency in graph[package_id]["deps"]:
            if any(kind["kind"] != "dev" for kind in dependency["dep_kinds"]):
                pending.append(dependency["pkg"])

    inventory = []
    lines = [
        "# Third-party notices", "",
        "RelayPost's own code is MIT licensed; see LICENSE. The bundled Rust",
        "dependencies retain their own licenses. This inventory follows normal",
        "and build dependencies reachable from this target's distributed programs;",
        "it excludes development-only dependencies and may include build-time tools.", "",
        "License expressions below are upstream Cargo metadata declarations.",
        "Supplied license, copyright, and notice files are preserved under licenses/.",
        "An absent supplied file is explicitly recorded; consult that crate's pinned",
        "source before redistributing or modifying it. This inventory is not a legal opinion.", "",
        "| Crate | Declared license | Supplied files |", "| --- | --- | --- |",
    ]
    for package in sorted(
        (packages[package_id] for package_id in reached if packages[package_id]["source"]),
        key=lambda package: (package["name"], package["version"]),
    ):
        package_root = Path(package["manifest_path"]).parent
        supplied = set()
        for path in package_root.iterdir():
            if path.is_file() and re.match(r"(?i)^(licen[cs]e|copying|copyright|notice|authors)", path.name):
                supplied.add(path)
        license_file = package.get("license_file")
        if license_file:
            path = package_root / license_file
            if not path.is_file():
                raise SystemExit(f"Declared license file is missing: {package['name']}")
            supplied.add(path)
        for dirname in ("license", "licenses", "LICENSES"):
            path = package_root / dirname
            if path.is_dir():
                supplied.update(item for item in path.rglob("*") if item.is_file())
        slug = f"{package['name']}-{package['version']}"
        copied = []
        for path in sorted(supplied):
            relative = path.resolve().relative_to(package_root.resolve())
            output = destination / "licenses" / slug / relative
            output.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, output)
            copied.append(output.relative_to(destination).as_posix())
        license_expression = package.get("license") or "See supplied license file"
        inventory.append({
            "name": package["name"], "version": package["version"],
            "license": package.get("license"), "source": package["source"],
            "repository": package.get("repository"), "supplied_files": copied,
        })
        paths = ", ".join(f"[{Path(path).name}]({path})" for path in copied)
        if not paths:
            paths = f"[Upstream source](https://crates.io/crates/{package['name']}/{package['version']}) (no license file supplied)"
        lines.append(f"| {slug} | {license_expression.replace('|', '/')} | {paths} |")
    (destination / "THIRD_PARTY_NOTICES.md").write_text("\n".join(lines) + "\n")
    (destination / "DEPENDENCIES.json").write_text(json.dumps(inventory, indent=2) + "\n")


def prepare(args):
    target = TARGETS[args.target]
    destination = Path(args.output)
    if destination.exists():
        raise SystemExit(f"Refusing to replace existing package directory: {destination}")
    metadata = json.loads(Path(args.metadata).read_text())
    release_version = version()
    for package in metadata["packages"]:
        if package["id"] in metadata["workspace_members"] and package["version"] != release_version:
            raise SystemExit("Workspace package version differs from release version")
    sha = source_sha()
    destination.mkdir(parents=True)
    manifest = {
        "schema_version": 1, "name": "RelayPost", "version": release_version,
        "source_sha": sha, "source_url": f"https://github.com/BrickerP/relaypost/commit/{sha}",
        "target": args.target, "platform": target["platform"],
        "minimum_os": target["minimum_os"], "signing": target["signing"],
        "reproducibility": "Locked dependencies and exact source identity; byte-for-byte reproducibility is not claimed",
    }
    (destination / "RELEASE.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (destination / "BUILD_INFO").write_text(
        f"version={release_version}\nsource_sha={sha}\ntarget={args.target}\n"
    )
    documents = (
        ("LICENSE", "LICENSE"), ("README.md", "README.md"),
        ("docs/INSTALL.md", "INSTALL.md"), ("docs/INSTALL.md", "QUICK_START.md"),
        ("docs/DEMO.md", "DEMO.md"), ("docs/RELEASE.md", "RELEASE.md"),
        ("scripts/verify-package.sh", "verify-package.sh"),
    )
    local_links = {str((ROOT / source).resolve()): name for source, name in documents}
    local_links[str((ROOT / "docs/INSTALL.md").resolve())] = "INSTALL.md"
    for source, name in documents:
        source_path = ROOT / source
        if source_path.suffix == ".md":
            def replace_link(match):
                target, separator, fragment = match[1].partition("#")
                path = (source_path.parent / target).resolve()
                link = local_links.get(str(path))
                if link is None:
                    relative = path.relative_to(ROOT).as_posix()
                    link = f"https://github.com/BrickerP/relaypost/blob/{sha}/{quote(relative)}"
                return "](" + link + (separator + fragment if separator else "") + ")"
            text = re.sub(r"\]\((?![a-zA-Z][a-zA-Z0-9+.-]*:|#)([^)\s]+)\)", replace_link, source_path.read_text())
            (destination / name).write_text(text)
        else:
            shutil.copyfile(source_path, destination / name)
    collect_notices(metadata, target["packages"], destination)
    toolchain_licenses = Path(os.environ["RUST_LICENSE_DIR"])
    for name in ("COPYRIGHT.html", "COPYRIGHT-library.html"):
        source = toolchain_licenses / name
        if not source.is_file():
            raise SystemExit(f"Rust toolchain redistribution notice is missing: {source}")
        output = destination / "licenses" / "rust-toolchain" / name
        output.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, output)
    license_directory = toolchain_licenses / "licenses"
    for name in ("MIT.txt", "Apache-2.0.txt"):
        if not (license_directory / name).is_file():
            raise SystemExit(f"Rust toolchain license is missing: {name}")
    shutil.copytree(license_directory, destination / "licenses" / "rust-toolchain" / "licenses")


def checksums(args):
    directory = Path(args.directory)
    lines = []
    for path in sorted(directory.rglob("*")):
        if path.is_symlink():
            raise SystemExit(f"Unexpected symbolic link in distribution: {path}")
        if path.is_file() and path != directory / "SHA256SUMS":
            with path.open("rb") as stream:
                digest = hashlib.file_digest(stream, "sha256").hexdigest()
            lines.append(f"{digest}  {path.relative_to(directory).as_posix()}")
    if not lines:
        raise SystemExit("Cannot checksum an empty package")
    (directory / "SHA256SUMS").write_text("\n".join(lines) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("version")
    command = commands.add_parser("prepare")
    command.add_argument("--target", choices=TARGETS, required=True)
    command.add_argument("--metadata", required=True)
    command.add_argument("--output", required=True)
    command = commands.add_parser("checksums")
    command.add_argument("directory")
    args = parser.parse_args()
    if args.command == "version":
        print(version())
    elif args.command == "prepare":
        prepare(args)
    else:
        checksums(args)


if __name__ == "__main__":
    main()
