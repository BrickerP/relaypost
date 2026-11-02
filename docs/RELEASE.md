# RelayPost 0.2.0 release procedure

This document prepares a reviewable candidate. CI does not create tags, publish
Releases, merge pull requests, or use signing credentials. Publishing a public
Release and final submission require the owner's review of the concrete assets.

## Candidate contents and provenance

The single workspace version in `Cargo.toml` is inherited by all three crates.
Only those three workspace package versions change in `Cargo.lock` for 0.2.0;
dependency versions remain locked. CI derives archive names, bundle version and
release metadata from that workspace value, then checks the built CLI's
`--version` against it.

| Asset | Contents |
| --- | --- |
| `relaypost-0.2.0-macos-arm64.zip` | `RelayPost.app` with both native UI and CLI, README, Quick Start/install guide, demo guide, MIT license, third-party notices/files, exact-source manifest and compiler-free verifier. |
| `relaypost-0.2.0-linux-x86_64.tar.gz` | Release CLI plus the same guides, license material, exact-source manifest and verifier. |
| `*.sha256` | Individual archive checksums. |
| `*.RELEASE.json` | Version, full build source SHA, source URL, target, signing state and tested platform boundary. Each archive includes the same manifest. |
| `SHA256SUMS` | Combined checksums of both archives and their external source manifests. |

Every extracted distribution also has a per-file `SHA256SUMS`. The macOS app's
resources retain provenance, guides and dependency notices. CI gathers license
expressions and supplied license/notice files from the actual resolved Cargo
metadata for each target, walking normal and build dependencies and excluding
development-only edges. The inventory may include build-time tools and does not
assert that every listed crate is statically linked. Nested font notices are
included; CI requires the four bundled font notice files. Where published crates
omit their workspace license texts, `third-party/catalog.json` supplies checked-in
copies from exact upstream commits or checksum-pinned published crates. If an old
upstream declares a license but supplies no standalone text, its original
declaration and actual author metadata accompany the primary canonical license
text; no copyright holder or year is invented. These evidence kinds are explicit
in the catalog and packaged inventory. Packaging verifies catalog text hashes and
crate checksums against `Cargo.lock`, preserves original copyright notices,
records provenance, and fails if a dependency has neither provided nor catalogued
notices. Canonical terms require a pinned upstream declaration as well. This is
attribution material, not an independent legal audit. Rust standard-library/toolchain licensing also applies to
the compiler's runtime components; the installed toolchain's `COPYRIGHT-library.html`,
toolchain copyright inventory and associated license texts are retained under
`licenses/rust-toolchain/`, following the [Rust distribution layout](https://doc.rust-lang.org/nightly/nightly-rustc/src/bootstrap/core/build_steps/dist.rs.html).

macOS binaries are built with `MACOSX_DEPLOYMENT_TARGET=12.0`; CI checks both
arm64 binaries' Mach-O minimum OS load commands and exercises them on macOS 15.
This does not mean macOS 12 was tested. The app is ad-hoc signed, without Developer
ID or notarization. Installation uses normal macOS approval and may be restricted
on managed Macs. Linux targets Ubuntu 24.04 x86_64/glibc 2.39, without a GUI or a
general compatibility claim. Intel macOS, Linux ARM and Windows are unsupported.

Source and dependencies are identified reproducibly. Byte-for-byte reproducible
binaries are not claimed: the current workflow uses the stable Rust toolchain and
hosted runner images, which can change. SHA-256 identifies the actual delivered
bytes; it is not an independent publisher signature.

## Build and review

1. Push the implementation branch and open/update its PR. Compilation, tests,
   rustfmt, Clippy, synthetic workflow checks and packaging run only in GitHub
   Actions. A formatting failure uploads `format.patch` while retaining failure.
   No local Rust toolchain command is required or permitted by `AGENTS.md`.
2. Require all jobs to succeed. The final **Assemble checked release candidate**
   job depends on dependencies, formatting, Linux and macOS. It checks both
   manifests' exact source SHA/version and archive checksums, then uploads
   `relaypost-0.2.0-candidate`. Platform jobs extract the final compressed bytes,
   verify every payload file, check executable paths and run `--version` again;
   macOS additionally checks the bundle signature and plist version.
3. Every job explicitly checks out the PR's exact head SHA, or the event commit
   for non-PR runs. `RELAYPOST_SOURCE_SHA` carries that expected commit without
   overriding GitHub's own variables. Packaging reads `git rev-parse HEAD` and
   refuses a mismatch; the archive verifier and final assembly require the same
   SHA. A successful exact-head PR build is therefore a valid deliverable
   candidate before merging. Use that full reviewed head SHA for both source
   manifests and the eventual draft/tag target, rather than GitHub's synthetic
   PR merge `GITHUB_SHA`. A manual `workflow_dispatch` build of the reviewed
   commit is optional once this workflow exists on the default branch; it is
   not a prerequisite for validating or preparing exact-head PR build assets.
4. Download the complete candidate, verify it on the target machine, and record
   the successful run URL, exact source SHA, asset filenames and checksum values
   in the owner-reviewed release notes. Exercise opening the downloaded app with
   normal macOS approval and the UI's **Try guided demo** from the actual archive. Record
   any validation that has not been performed instead of inferring it from tests.

All uploaded Actions artifacts request **90-day retention**, the public-repository
maximum, subject to repository policy. Workflow log retention follows repository
settings. See the [GitHub retention policy](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/enabling-features-for-your-repository/managing-github-actions-settings-for-a-repository). Actions links can expire and require authentication. They are intermediate review evidence,
not the final distribution link. Attach the approved bytes to a **public GitHub
Release** before final submission, and keep that Release/assets available through
at least January 30, 2027. A draft Release is review material, not public delivery.

## Prepare a draft after the candidate passes

The following commands are for the owner after a concrete candidate is reviewed.
Download is read-only. Creating the draft is an external write; do not execute it
without the owner's authorization. The workflow itself has only `contents: read`.

```sh
gh run download RUN_ID --repo BrickerP/relaypost \
  --name relaypost-0.2.0-candidate --dir release-candidate
cd release-candidate
shasum -a 256 --check SHA256SUMS
```

Read both source manifests. Write reviewed `RELEASE_NOTES.md` containing that full
SHA, the successful CI run URL, the filenames/checksums, platform/signing limits,
installation link, demo steps and the actual manual validation outcome. Keep the
downloaded artifacts unchanged. Then, only after authorization:

```sh
gh release create v0.2.0 --repo BrickerP/relaypost --draft \
  --target FULL_REVIEWED_SOURCE_SHA --title 'RelayPost 0.2.0' \
  --notes-file RELEASE_NOTES.md \
  relaypost-0.2.0-macos-arm64.zip relaypost-0.2.0-macos-arm64.zip.sha256 \
  relaypost-0.2.0-linux-x86_64.tar.gz relaypost-0.2.0-linux-x86_64.tar.gz.sha256 \
  relaypost-0.2.0-macos-arm64.RELEASE.json \
  relaypost-0.2.0-linux-x86_64.RELEASE.json SHA256SUMS
```

The draft command can create the version tag; run it only after checking that the
tag does not exist and that both manifests match the intended immutable commit.
Review the draft, test its exact asset bytes, and obtain the owner's separate
public-publish approval. The public release URL will be
`https://github.com/BrickerP/relaypost/releases/tag/v0.2.0` **after publication**.
Do not replace assets under a published version: fixes require a new source
commit and version. Final submission must link the published Release, not a
logged-in Actions artifact or an unpublished draft.
