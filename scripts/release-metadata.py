#!/usr/bin/env python3
"""Describe already-built CI artifacts and collect supplied dependency notices.

No compiler, dependency resolver, or network access is invoked here. Cargo metadata
must be produced by CI for the package's actual target before invoking prepare.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
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
    expected = os.environ.get("RELAYPOST_SOURCE_SHA")
    if expected and expected != value:
        raise SystemExit("Checkout does not match RELAYPOST_SOURCE_SHA")
    return value


def load_license_catalog():
    directory = (ROOT / "third-party").resolve()
    document = json.loads((directory / "catalog.json").read_text())
    if document.get("schema_version") != 1:
        raise SystemExit("Unsupported third-party license catalog schema")
    with (ROOT / "Cargo.lock").open("rb") as stream:
        locked = {
            (package["name"], package["version"]): package.get("checksum")
            for package in tomllib.load(stream)["package"]
            if package.get("source", "").startswith("registry+")
        }
    packages = {}
    for entry in document["packages"]:
        key = (entry["name"], entry["version"])
        if key in packages or not entry["files"]:
            raise SystemExit(f"Duplicate or empty license catalog entry: {key}")
        commit = entry.get("upstream_commit")
        crate_hash = entry.get("crate_sha256")
        if commit and not re.fullmatch(r"[0-9a-f]{40}", commit):
            raise SystemExit(f"Invalid license catalog upstream commit: {key}")
        if crate_hash and crate_hash != locked.get(key):
            raise SystemExit(f"License catalog crate checksum differs from Cargo.lock: {key}")
        if not commit and not crate_hash:
            raise SystemExit(f"License catalog needs a pinned commit or crate checksum: {key}")
        kinds = {file["kind"] for file in entry["files"]}
        if not kinds <= {"upstream_license", "upstream_notice", "upstream_declaration", "canonical_license"}:
            raise SystemExit(f"Unknown license catalog evidence kind: {key}")
        if not kinds & {"upstream_license", "canonical_license"}:
            raise SystemExit(f"License catalog contains no license terms: {key}")
        if "canonical_license" in kinds and (
            "upstream_declaration" not in kinds or not crate_hash or "authors" not in entry
        ):
            raise SystemExit(f"Canonical license needs the pinned upstream declaration and author metadata: {key}")
        for file in entry["files"]:
            relative = PurePosixPath(file["path"])
            if relative.is_absolute() or ".." in relative.parts:
                raise SystemExit(f"License catalog path must be relative: {file['path']}")
            path = (directory / file["path"]).resolve()
            path.relative_to(directory)
            with path.open("rb") as stream:
                digest = hashlib.file_digest(stream, "sha256").hexdigest()
            if digest != file["sha256"] or not file["source_url"].startswith("https://"):
                raise SystemExit(f"Invalid pinned license source: {file['path']}")
        packages[key] = entry
    return packages


def collect_notices(metadata, roots, destination, catalog):
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
        "Supplied license, copyright, font, and notice files are preserved under licenses/.",
        "Workspace license texts omitted from published crates are supplemented from",
        "the checked-in third-party/catalog.json. Their exact upstream source commits,",
        "URLs and file hashes are retained in DEPENDENCIES.json. This inventory is",
        "attribution material, not an independent legal audit or proof of static linkage.", "",
        "| Crate | Declared license | Supplied files |", "| --- | --- | --- |",
    ]
    for package in sorted(
        (packages[package_id] for package_id in reached if packages[package_id]["source"]),
        key=lambda package: (package["name"], package["version"]),
    ):
        package_root = Path(package["manifest_path"]).parent
        supplied = set()
        for path in package_root.rglob("*"):
            if not path.is_file():
                continue
            parents = path.relative_to(package_root).parts[:-1]
            if (
                re.search(r"(?i)(^|[-_.])(licen[cs]e|copying|copyright|notice|authors)", path.name)
                or any(part.casefold() in ("license", "licenses") for part in parents)
                or ("fonts" in parents and path.suffix.casefold() == ".txt")
            ):
                supplied.add(path)
        license_file = package.get("license_file")
        if license_file:
            path = package_root / license_file
            if not path.is_file():
                raise SystemExit(f"Declared license file is missing: {package['name']}")
            supplied.add(path)
        if package["name"] == "epaint_default_fonts":
            required = {
                "fonts/Hack-Regular.txt", "fonts/OFL.txt", "fonts/UFL.txt",
                "fonts/emoji-icon-font-mit-license.txt",
            }
            found = {path.relative_to(package_root).as_posix() for path in supplied}
            if not required <= found:
                raise SystemExit("Bundled fonts are missing their required upstream notices")
        slug = f"{package['name']}-{package['version']}"
        copied = []
        for path in sorted(supplied):
            relative = path.resolve().relative_to(package_root.resolve())
            output = destination / "licenses" / slug / relative
            output.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, output)
            copied.append(output.relative_to(destination).as_posix())
        pinned = catalog.get((package["name"], package["version"]))
        provenance = None
        if pinned:
            records = []
            for file in pinned["files"]:
                output = destination / "licenses" / slug / "upstream" / file["path"]
                output.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / "third-party" / file["path"], output)
                relative = output.relative_to(destination).as_posix()
                copied.append(relative)
                records.append({**file, "catalog_path": file["path"], "path": relative})
            provenance = {**pinned, "files": records}
        if not copied:
            raise SystemExit(f"No provided or catalogued license notices: {slug}")
        license_expression = package.get("license") or "See supplied license file"
        inventory.append({
            "name": package["name"], "version": package["version"],
            "license": package.get("license"), "source": package["source"],
            "repository": package.get("repository"), "supplied_files": copied,
            "upstream_catalog": provenance,
        })
        paths = ", ".join(f"[{Path(path).name}]({path})" for path in copied)
        lines.append(f"| {slug} | {license_expression.replace('|', '/')} | {paths} |")
    (destination / "THIRD_PARTY_NOTICES.md").write_text("\n".join(lines) + "\n")
    (destination / "DEPENDENCIES.json").write_text(json.dumps(inventory, indent=2) + "\n")


def collect_desktop_font_notices(destination):
    document = json.loads((ROOT / "third-party/desktop-fonts.json").read_text())
    commit = document["upstream_commit"]
    if (
        document.get("schema_version") != 1
        or document["license"] != "OFL-1.1"
        or not re.fullmatch(r"[0-9a-f]{40}", commit)
        or not re.fullmatch(r"[a-z0-9-]+", document["id"])
    ):
        raise SystemExit("Invalid pinned desktop font catalog")
    kinds = [file["kind"] for file in document["files"]]
    if kinds.count("license") != 1 or kinds.count("font") < 1 or set(kinds) != {"font", "license"}:
        raise SystemExit("Desktop fonts require font data and one complete license")
    license_directory = destination / "licenses" / document["id"]
    license_directory.mkdir(parents=True)
    files = []
    for file in document["files"]:
        relative = PurePosixPath(file["path"])
        if relative.is_absolute() or ".." in relative.parts:
            raise SystemExit("Desktop font paths must be relative to the repository")
        path = (ROOT / file["path"]).resolve()
        path.relative_to(ROOT)
        data = path.read_bytes()
        if (
            len(data) != file["size"]
            or hashlib.sha256(data).hexdigest() != file["sha256"]
            or not file["source_url"].startswith("https://")
            or f"/{commit}/" not in file["source_url"]
        ):
            raise SystemExit(f"Pinned desktop font source mismatch: {file['path']}")
        record = dict(file)
        if file["kind"] == "license":
            output = license_directory / "OFL.txt"
            shutil.copyfile(path, output)
            record["packaged_path"] = output.relative_to(destination).as_posix()
            copyright_notice = data.decode("utf-8").splitlines()[0]
            license_path = record["packaged_path"]
        else:
            record["embedded_in"] = "RelayPost.app/Contents/MacOS/relay-desktop"
        files.append(record)
    relative = PurePosixPath(document["source_record"])
    if relative.is_absolute() or ".." in relative.parts:
        raise SystemExit("Desktop font provenance path must be relative to the repository")
    source = (ROOT / document["source_record"]).resolve()
    source.relative_to(ROOT)
    output = license_directory / "SOURCE.md"
    shutil.copyfile(source, output)
    source_record = {
        "path": document["source_record"],
        "packaged_path": output.relative_to(destination).as_posix(),
        "sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
    }
    manifest = {**document, "files": files, "source_record": source_record}
    (destination / "BUNDLED_FONTS.json").write_text(json.dumps(manifest, indent=2) + "\n")
    styles = ", ".join(file["style"] for file in files if file["kind"] == "font")
    with (destination / "THIRD_PARTY_NOTICES.md").open("a") as stream:
        stream.write(
            f"\n## Embedded desktop font: {document['family']}\n\n"
            f"The native desktop executable embeds unmodified {styles} font data.\n\n"
            f"{copyright_notice}\n\n"
            f"License: [{document['license']} — complete original text]({license_path}). "
            f"[Pinned source record]({source_record['packaged_path']}) and "
            "[font file hashes and provenance](BUNDLED_FONTS.json).\n"
        )


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
    collect_notices(metadata, target["packages"], destination, load_license_catalog())
    if "relay-desktop" in target["packages"]:
        collect_desktop_font_notices(destination)
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
