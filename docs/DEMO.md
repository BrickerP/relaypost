# Reproducible offline exchange demonstration

Use the `relaypost-macos-arm64` artifact from a successful **Verify and package**
GitHub Actions run. It contains an ad-hoc signed native application and CLI for
Apple Silicon. It is not Apple Developer signed or notarized. This is a CI
artifact, not an installer or a verified public release.

All Rust compilation, tests, formatting and Clippy checks run in GitHub Actions.
These instructions only execute the prebuilt binaries.

## Run the sample

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

The script checks the entire real CLI chain:

1. Initialize two identities, exchange self-signed contact cards, and explicitly
   pin the complete fingerprints. In a real exchange, first compare those
   fingerprints through a trusted channel.
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

`demo-report.json` records the actual group health, repaired count, original and
received SHA-256 values, duplicate outcome, final sender state, and command exit
codes. CI retains this report with its test logs. The report contains public
identifiers and synthetic fixture paths; it contains no passwords or private
keys.

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
| Resolve dependencies | Generates the initial `Cargo.lock` when absent; verifies committed locks with `--locked`; uploads the lock for review and commit. |
| Rust formatting | Runs `cargo fmt --all -- --check`. On failure, retains the failing status and uploads a `format.patch`; it never commits a fix. |
| Linux core and CLI | Runs integration and unit tests, Clippy with warnings denied, and the actual CLI sample. |
| macOS arm64 native application | Tests both real macOS publication semantics and the CLI, builds the native interface, checks its Clippy warnings, runs the release CLI sample, and packages the app with a SHA-256 file. |

Build and test logs, reports, lockfile and app artifacts are retained for 30 days.
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

The erasure matrix checks a full 20+4 group with zero through four missing data
shards, corrupt parity shards, and mixed erasures, plus five-erasure refusal. Tail
tests independently check real per-group budgets from one through four parity
shards, exactly-budget recovery, and budget-plus-one refusal. Truncated and
extended shards must be counted as erasures. The codec's separate unit tests
cover every tail size and exact group boundaries.

Recovery tests deliberately delete the SQLite incoming index and receipt after
publication, then reopen the profile. Verified published bytes must recreate the
index and receipt; externally changed bytes must produce a conflict without a
new success receipt. This reproduces durable crash-window states; it does not
simulate a physical power cut. Fault-case receiver homes are copies of a
pristine **synthetic** identity, allowing each case to exercise reconstruction
without a previous import's duplicate fast path.

This document describes the runnable checks. The linked workflow result and
retained reports determine whether a particular commit passed them. Security
audit, clean-account distribution, fair baseline comparisons and actual
two-device/media verification remain separate release tasks.
