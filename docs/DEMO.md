# Reproducible offline exchange demonstration

Use the `relaypost-macos-arm64` artifact from a successful **Verify and package**
GitHub Actions run. It contains an ad-hoc signed native application and CLI for
Apple Silicon. It is not Apple Developer signed or notarized. This is a CI
artifact, not an installer or a verified public release.

All Rust compilation, tests, formatting and Clippy checks run in GitHub Actions.
These instructions only execute the prebuilt binaries.

## Try the native guided demo

1. Extract the candidate macOS bundle, verify the ZIP against its SHA-256 file,
   and open `RelayPost.app` on an Apple Silicon Mac.
2. Click **Try guided demo**, or **Demo → Run new guided demo**. No identity setup, password or
   development toolchain is required. The app creates two disposable profiles
   and 21 MiB of synthetic files in a new directory separate from its normal
   profile. It displays that directory for inspection.
3. Use **Damage carrier copy →**, **Repair and receive →**, then **Carry back
   signed receipt →** to advance through **Prepared**, **Damaged**, **Received**
   and **Confirmed**. Check the four repaired shards, matching SHA-256 values,
   five-erasure receive refusal, and sender waiting until the signed receipt
   returns. The final `demo-report.json` is generated automatically;
   **Export JSON report…** copies it to a new chosen filename.

The GUI and CLI call the same `relay-core::DemoSession`; this is a real
cryptographic/file-system workflow with pauses between actual stages. It uses
two profiles on one machine and a directory carrier. It is not a physical USB,
two-device, performance or power-loss test.

The bundled CLI runs all the same stages noninteractively:

```sh
/absolute/path/to/RelayPost.app/Contents/MacOS/relaypost \
  demo --output /absolute/path/to/NEW-synthetic-demo --json
```

The output path must **not exist**, including as an empty directory. Existing
output is neither overwritten nor resumed; a restart creates another fresh
run. The demo command dispatches before opening the ordinary profile and ignores
profile/password input. It emits a public-facts snapshot and writes the same
JSON to `demo-report.json`. Synthetic private keys remain only in their
disposable profiles, encrypted by clearly labeled fixture passphrases; neither
passphrases nor private keys appear in the report.

Source files remain in `source_files/`. Alice's retained outbox has the intact
signed parcel. The carrier has two missing data shards and two corrupt parity
shards in its first full 20+4 group. A temporary fifth erasure causes an actual
receive refusal with no committed batch or success receipt; the fifth shard is
then restored before the successful four-shard repair. Raw tar and encryption
overhead affects the tail layout, so inspect its recorded budget rather than
assuming one data shard per MiB of original input.

At **Received**, Bob's published files and signed receipt exist, their hashes
match the sources, and Alice remains **WaitingReceipt**. Advancing to
**Confirmed** exports and verifies the return receipt. `export_report` is valid
only after confirmation and refuses to replace an existing report. The JSON
contains actual inspections, evidence paths, the five-erasure receive error,
source/received hash maps, repaired count and sender state.

For judge access, use the stable public release selected in the final
submission. Actions artifacts are development evidence and can expire. See
[English judge instructions](submission/TESTING.md) and the
[candidate evidence record](submission/READINESS.md).

## Independent command-line acceptance harness

Unzip the artifact into a chosen directory, verify its checksum against the
adjacent `.sha256` file, then run from the repository:

```sh
python3 scripts/demo.py \
  --binary /absolute/path/to/RelayPost.app/Contents/MacOS/relaypost \
  --output /absolute/path/to/new-synthetic-demo
```

The output directory must be new or empty. The script creates two isolated
profiles named Synthetic Alice and Synthetic Bob, an explicitly synthetic
password file, and a static 21 MiB sample. It never loads your normal profile.
The script uses `--password-file`; passwords are not command-line arguments or
printed in the transcript. Treat the generated profiles as disposable fixtures.

The script retains its independent checks of the real CLI chain:

1. Initialize two identities, exchange self-signed contact cards, and explicitly
   pin the complete fingerprints. In a real exchange, first compare those
   fingerprints through a trusted channel. Reject importing Alice's own card
   and sending to herself, without creating a delivery.
2. Seal the sample for Bob and export it to a local carrier directory. Alice's
   state must remain `WaitingReceipt`.
3. Remove five shards from a copy of the first full group. Inspection must report
   the group as unrecoverable, receive must fail, and receipt export must fail.
4. Remove two source shards and corrupt two parity shards in a separate copy.
   Inspection must identify all four erasures. Bob must reconstruct them and
   commit a sample with exactly the original SHA-256 digest.
5. Import the same parcel again. It must report a duplicate and retain the same
   single destination directory.
6. Export Bob's signed receipt, return it to Alice, and require the outgoing
   delivery to become `Confirmed` only after receipt verification.
7. Recreate a durable prepublication window using only that synthetic received
   batch: move its authenticated files into private staging and remove the
   synthetic incoming index and receipt. `recover` must verify and publish the
   prepared batch, regenerate its receipt, preserve the original payload digest,
   and delete the staging wrapper, including its retained plaintext archive.
   Also recreate the actual postpublication residue: an ID-bound wrapper with
   archive and ciphertext but no batch child. Recovery must verify the published
   batch before deleting that wrapper.
8. Create an incomplete synthetic receive stage. Normal `recover` must retain it
   as pending. Run `recover --discard-incomplete` without a password argument;
   it must remove the managed stage and preserve the committed payload.
9. Damage a retained outgoing shard after confirmation. `recover` must preserve
   `Confirmed` while reporting `local_error`. Restore the exact original shard;
   recovery must clear the health error and retain confirmation.
10. Run the product's shared `demo` command in a separate new root with an
    unusable ordinary-profile path and nonexistent password file. Independently
    hash the real source, received files and carrier shards, rebuild the intact
    retained ciphertext to verify its ID, check the receipt binding and sender
    index, and compare stdout with the immutable report. Reusing that output
    must fail without replacing the report.

`demo-report.json` records the actual group health, repaired count, original and
received SHA-256 values, duplicate outcome, final sender state, and command exit
codes, plus the self-send, staged recovery, explicit discard and outgoing health
checks. CI retains this report with its test logs. The report contains public
identifiers and synthetic fixture paths; it contains no passwords or private
keys.

The harness is developer acceptance infrastructure. It does not provide a
second product implementation of the guided journey; step 10 invokes the
shared Rust engine and verifies its resulting bytes and public facts.

The demonstration uses separate CLI processes on **one machine** and a directory
as the carrier. It does not prove two-device interoperability, actual USB-media
behavior, physical power-loss durability, notarization, or measured performance.

## Open the native interface with a sample profile

After the sample succeeds, either open the bundle with arguments or run its
prebuilt native executable directly:

```sh
open -n /absolute/path/to/RelayPost.app --args \
  --home /absolute/path/to/new-synthetic-demo/alice

/absolute/path/to/RelayPost.app/Contents/MacOS/relay-desktop \
  --home /absolute/path/to/new-synthetic-demo/bob
```

Use separate application processes for Alice and Bob. The History screen should
show Alice's confirmed outgoing delivery and Bob's saved incoming delivery. For
a live walkthrough of the earlier states, create fresh profiles in new homes
and follow the command sequence printed by the script. Unlock only with the
synthetic password created in the demo directory.

If macOS blocks opening an unnotarized application, review it through the normal
macOS Privacy & Security flow. The artifact makes no Gatekeeper or signing
guarantee. Do not replace or weaken your system-wide security settings.

## Remote verification and artifacts

The workflow has four independent concerns:

| Job | Evidence |
| --- | --- |
| Resolve dependencies | Requires the committed `Cargo.lock`, verifies dependency resolution with `cargo metadata --locked`, and retains the lock and dependency inventory. |
| Rust formatting | Runs `cargo fmt --all -- --check`. On failure, retains the failing status and uploads a `format.patch`; it never commits a fix. |
| Linux core and CLI | Runs integration and unit tests, Clippy with warnings denied, and the actual CLI sample. |
| macOS arm64 native application | Tests both real macOS publication semantics and the CLI, builds the native interface, checks its Clippy warnings, runs the release CLI sample, and packages the app with a SHA-256 file. |

Build and test logs, reports, lockfile and app artifacts are retained for 90 days.
No workflow merges, publishes a GitHub Release, or uploads signing credentials.
The Linux job compiles only the core and CLI, so it does not require desktop
X11/Wayland system packages. The native app is compiled on macOS.

GitHub currently assigns `macos-15` to an arm64 standard runner; Intel uses
`macos-15-intel`. Runner selection follows the
[official runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).

## Acceptance coverage and limits

`crates/relay-core/tests/profile_exchange.rs` exercises real public `Profile`
operations with synthetic identities and files. It covers trusted roundtrip,
both trust pins, wrong passwords and recipients, card/envelope/receipt tampering,
receipt-gated confirmation, duplicate import, immutable resend, cancellation,
and externally modified or unexpected destination files.

Outgoing health tests retain `Ready`, `WaitingReceipt`, `Cancelled`, and
`Confirmed` through source corruption or removal, report the problem separately,
and clear it only after exact restoration. Durable acknowledgments include the
recipient's signed receipt and its authenticated original envelope; transport
receipts remain two files. Tests lose or roll back the outgoing index before
corrupting or removing the entire outbox parcel, then require recovery to rebuild
confirmation from those four authenticated acknowledgment files. Tampering with
any of the four files cannot create confirmation.

The erasure matrix checks a full 20+4 group with zero through four missing data
shards, corrupt parity shards, and mixed erasures, plus five-erasure refusal. Tail
tests independently check real per-group budgets from one through four parity
shards, exactly-budget recovery, and budget-plus-one refusal. Truncated and
extended shards must be counted as erasures. The codec's separate unit tests
cover every tail size and exact group boundaries.

Recovery tests deliberately delete the SQLite incoming index and receipt after
publication, then reopen the profile. Verified published bytes must recreate the
index and receipt; externally changed bytes must produce a conflict without a
new success receipt. Prepared but unpublished stages must resume publication;
matching already-published stages must lose their superseded plaintext wrapper.
Conflicting destinations and corrupt prepared stages retain staged evidence,
and incomplete stages remain pending until explicit cancellation. Cancellation
preserves unrelated entries and refuses linked contents. Unix FIFO envelope and
receipt inputs must return without waiting for a writer and release the profile
lock for subsequent operations; these tests have bounded waits and rescue a
regressed blocking read so it cannot indefinitely hang CI. Self-card import and
self-send also fail without creating delivery rows, including a synthetic old
database containing a trusted self-contact.

These tests reproduce durable crash-window states; they do not simulate a
physical power cut. Fault-case receiver homes are copies of a
pristine **synthetic** identity, allowing each case to exercise reconstruction
without a previous import's duplicate fast path.

This document describes the runnable checks. The linked workflow result and
retained reports determine whether a particular commit passed them. Security
audit, clean-account distribution, fair baseline comparisons and actual
two-device/media verification remain separate release tasks.
