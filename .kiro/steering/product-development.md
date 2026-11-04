# RelayPost — product development steering

## What this product is

RelayPost is an offline file-delivery tool. Two devices share public identity
cards once, then exchange encrypted parcels through a USB drive or any ordinary
directory without ever being online at the same time. A signed receipt from the
recipient's key proves a verified save happened.

The current v0.1 scope is a macOS-only desktop prototype (egui) plus a
cross-platform CLI (`relaypost`). The core library `relay-core` is shared.
Windows is not supported. Linux receives CLI testing only via CI.

## Approved product features (v0.1)

- Identity: X25519 age key + Minisign signing key, self-signed PeerCard, stored
  in a passphrase-encrypted age file under a private profile directory.
- Contacts: import and pin trust via complete fingerprint comparison; self-import
  refused; both devices must trust each other before exchange.
- Seal: snapshot 1–100 regular files (≤ 64 MiB), encrypt for one trusted
  recipient, sign envelope, write immutable parcel shards with Reed–Solomon
  parity into the local outbox.
- Export parcel: copy immutable shards to a carrier directory and record
  WaitingReceipt.
- Inspect: verify signed envelope + measure actual shard health per group without
  decrypting.
- Receive: authenticate sender, repair shards, decrypt, validate every file via
  manifest, atomic no-replace commit to inbox, sign receipt.
- Export receipt / Import receipt: carry signed receipt back; sender confirms
  delivery only after the receipt binds the exact original envelope.
- History: show DeliveryState (Ready / WaitingReceipt / SavedReceiptReady /
  Confirmed / Failed / Cancelled), local-copy health separately, group detail.
- Recover: resume verified staged receives, rebuild index and missing receipts
  from verified published files. Conflicts reported, not silently resolved.
- Discard incomplete: remove only owned `inbox/.staging/batch-*` entries;
  explicit confirmation required in GUI; no password needed.
- Cancel: stop future local re-exports; cannot recall already-exported copies;
  does not delete source files.
- Guided demo: always-accessible interactive walkthrough using isolated synthetic
  profiles; see the guided-demo spec for details.

## Approved product scope additions (guided demo)

First-launch and always-accessible CmdD (GUI) / `relaypost demo` (CLI) guided
demo using two synthetic profiles in a new private root directory. Full details
in `.kiro/specs/guided-demo/`.

## Author and process constraints

- **No local compilation.** No `cargo`, `rustc`, `rustfmt`, `cargo clippy`, or
  `cargo test` may be run locally by any contributing agent or automated step.
  All compilation, formatting checks and tests run exclusively in GitHub Actions
  on the CI branch.
- **File-only agent.** The Kiro agent uses only `read` and `write` tool calls
  on files in this personal repository. No shell commands, MCP calls, subagent
  spawning or external network requests.
- **Personal repository only.** This project is independent of any company
  codebase. No company code, credentials, or internal data must appear here.
- **No secrets in outputs.** Private keys, passwords, and passphrase values must
  never appear in reports, snapshots, JSON exports, log output, or spec files.
  Reports contain only public facts: fingerprints, paths, hashes of content, and
  delivery state.
- **Genuine outcomes only.** Do not claim tests passed unless GitHub Actions
  actually ran them. Do not claim Kiro authored work it did not perform. Mark
  unimplemented tasks as `[ ] NOT YET IMPLEMENTED`. Mark source-implemented tasks
  separately from CI-verified; do not say "passes" until actual CI runs confirm it.
- **Respect parallel ownership (v0.2 candidate).** Kiro owns the guided demo
  engine (`relay-core/src/demo.rs`, `relay-core/src/lib.rs` module+re-exports
  only), CLI demo subcommand (`relay-cli/src/main.rs`), and desktop demo feature
  (`relay-desktop/src/{app.rs,work.rs,main.rs,demo_view.rs}`). Independent QA
  owns all integration tests in `crates/relay-core/tests/`. Release worker owns
  workspace manifests, `Cargo.lock`, CI workflows, scripts, and release docs.
  Codex reviewer owns authorship evidence in `docs/KIRO_USAGE.md`. Root owns
  workspace manifest, project docs, and Kiro integration specs. Do not write
  duplicate or tautological unit tests; QA owns independent integration tests
  and remote-only verification.
- **Existing dependencies only.** Do not add crates to any `Cargo.toml` that are
  not already present in the workspace.

## Approved product scope additions (novice UX, 2026-11-03)

The user requested a real first-time, nontechnical-user walkthrough and a modern
native UI redesign. The current code is a first submitted version, not a frozen
product direction. This change replaces the original desktop presentation and
orchestration directly; it does not add a classic mode, web shell or runtime AI.

- Primary destinations are Home, Send, Receive, Activity and People. The existing
  guided walkthrough moves to Help. A state-based Home recommends a next task
  from locally verified delivery facts, never an invented remote event.
- Device setup and adding a contact lead to explicit next actions. Explain
  connection cards and complete safety-code comparison in ordinary language.
  Keep full fingerprint comparison and both devices' trust requirements.
- Connect prepare/copy, automatic carrier inspection/save, and returning/importing
  signed confirmations into clear task flows. All operations still use the shared
  core and its background desktop worker. A directory picker must help locate a
  transfer on a chosen drive using bounded discovery, not require the user to
  understand protocol directory names or recursively scan unrelated files.
- Technical paths, hashes, IDs, shard grids and raw errors stay available under
  expandable details. Only the expected recipient's verified signed receipt can
  label an outgoing transfer confirmed. Copying to a drive is not remote delivery.
- Use the bundled, original Atkinson Hyperlegible Next Regular and Medium TTFs
  with the upstream OFL. Root owns distribution attribution and packaging.
- The UX implementer/Kiro owns `crates/relay-desktop/src/**` and the new
  `.kiro/specs/novice-ux/**` inputs. Root owns manifests, release tooling,
  documentation, provenance review and CI. Font assets have a separate owner.
  Do not modify core or CLI for this redesign or claim capabilities not present
  in their public API. Existing no-local-Rust and file-only Kiro rules remain.

## Shared core structure

```
relaypost (clap CLI) ─┐
                      ├─ relay-core ─ age / Minisign / SHA-256 / Reed–Solomon
relay-desktop (egui) ─┘              └─ SQLite index + private profile dirs
```

`relay-core` owns every cryptographic, codec, and storage decision. CLI and
desktop are thin shells. The guided demo engine lives inside `relay-core` as a
new public module; both CLI and desktop call the same `DemoSession` API.

## Key invariants the implementation must preserve

- SHA-256 of the age ciphertext is the parcel ID. The envelope signs both
  identities, all group + shard parameters, and every shard digest. Reject
  duplicate JSON fields and conflicting signed copies.
- Reed–Solomon groups: 20 data + 4 parity shards for full groups (20 MiB each).
  Tail groups use n = ⌈bytes/MiB⌉ data shards and r = ⌈n/5⌉ parity shards.
  Hash-bad shards are erasures; over-budget groups refuse to receive.
- Atomic no-replace directory publication via `renameat2(RENAME_NOREPLACE)` on
  Linux and `renamex_np(RENAME_EXCL)` on macOS. No check-then-rename fallback.
- Receiver commit authority comes from verified published content + metadata. The
  SQLite database is an index only; it cannot authorize a successful receipt.
- A signed receipt is written only after the full batch is committed and verified.
  Only the pinned expected recipient key can confirm an outgoing delivery.
- Profile stores a path only. No open connections or unlocked keys are retained
  between API calls. Profile is `Clone` and safe to move into a worker thread.
