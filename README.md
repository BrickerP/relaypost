# RelayPost / 驿站

**Send files offline. Know when the other computer saved them.**

RelayPost helps you carry protected files between computers on a USB drive or an
ordinary folder. It checks a carried copy, repairs damage within its recovery
budget, and brings back confirmation that the other computer saved the files.
Both computers use RelayPost; they never need to be online together.

The native desktop app and CLI share one Rust core. The design revives UUCP's
store-and-forward idea with recipient encryption and a signed return receipt.

## Install and try it

RelayPost 0.3.0 provides an Apple Silicon macOS app with a bundled CLI and an
Ubuntu 24.04 x86_64 Linux CLI archive. Rust, Python and a source checkout are not
needed to run the packages. See the [three-step installation guide](docs/INSTALL.md).

1. Download the platform archive and its SHA-256 file from the versioned
   [GitHub Release](https://github.com/BrickerP/relaypost/releases) when published.
   Before publication, a successful [Verify and package run](https://github.com/BrickerP/relaypost/actions/workflows/ci.yml)
   supplies `relaypost-0.3.0-candidate` for review. An Actions artifact is not the
   final public distribution link.
2. On macOS, double-click the ZIP, move `RelayPost.app` into Applications, and
   open it. The desktop app does not require Terminal. Optional checksum and
   package-verification commands are in the installation guide.
3. Start on **Home**. Set up this computer and follow the next-action card to
   connect the other computer. To try a disposable sample first, open
   **Help & walkthrough** and choose **Start a new walkthrough**. CLI users can
   run `relaypost demo --output NEW_DIRECTORY`.

For your own files, **People** walks through sharing connection cards and
checking each computer's complete safety code. Both computers must check the
other before exchanging files. **Send files** prepares a transfer and copies it
to your chosen drive or folder. **Receive files** finds a transfer there,
checks it automatically and saves the verified files. Put its confirmation on
the drive and bring it back. **Home** offers the waiting sender a direct action
to add that confirmation. Copying to a drive alone is never shown as delivered.

The macOS app is built for Apple Silicon/macOS 12+ and exercised on macOS 15;
macOS 12 itself has not been tested. It is ad-hoc signed, without Developer ID
signing or notarization. Linux CLI validation uses Ubuntu 24.04/glibc 2.39; other
distributions are unverified. Intel Macs, Linux GUI and Windows are unsupported.
Compilation and Rust checks run in GitHub Actions, with no local compilation.

See the [runnable demo and verification limits](docs/DEMO.md). `--home DIRECTORY`
gives either interface its own profile; otherwise they share the OS
application-data directory. Run the bundled `relaypost --help` for commands. Use
a masked prompt, `--password-stdin`, or `--password-file`; never put a password
in command arguments. Packages include this project's MIT license, dependency
inventory and supplied third-party notices; see [release provenance](docs/RELEASE.md).

## What works

| Operation | Actual behavior |
| --- | --- |
| Identity | X25519 age recipient key and Minisign signing key, bound in a self-signed contact card. Private keys are stored in a passphrase-encrypted age file. |
| Seal | Snapshot 1–100 regular files, at most 64 MiB total, hash a bounded manifest, encrypt it, and sign the parcel envelope. Plaintext filenames remain inside the encrypted archive. |
| Repair | Each full group has 20 data and 4 parity shards. Tail groups use a smaller explicit budget. Missing and bad-hash pieces count as erasures within that group. |
| Receive | Authenticate the trusted sender, reconstruct ciphertext, check its SHA-256, decrypt, validate every file, and publish one complete batch with an OS-exclusive rename. |
| Confirm | Save a receipt signed by the expected recipient only after commit. The sender stays `WaitingReceipt` until the returned receipt verifies. |

**Activity** shows local delivery records, recovery results and cancellation.
Re-importing the same parcel verifies and reuses its committed batch. Recovery
rebuilds a missing index or receipt from verified published files; changed files
produce a conflict. A damaged local outbox copy has its own health warning;
`WaitingReceipt`, `Confirmed`, and cancellation remain delivery facts. Verified
returned acknowledgments retain their original signed envelope, allowing their
confirmation to be recovered even when the local outbox copy is gone.

Recovery also resumes fully verified receive stages and removes superseded
temporary copies after checking the published batch. Partial or conflicting
stages remain pending. Discarding interrupted receive work in **Activity** offers an
explicit confirmation, or use `relaypost recover --discard-incomplete` to discard
only managed receive staging. This command requires no password. Import the
parcel again from its carrier when you want to retry it.

Contacts must identify another device; importing or sending to this identity's
own card is refused. Cancellation stops future local exports and cannot recall
copies already carried elsewhere. Original input files are never removed.

## Architecture

```text
relaypost (clap CLI) ─┐
                     ├─ relay-core ─ age / Minisign / SHA-256 / Reed–Solomon
relay-desktop (egui) ─┘             └─ private files + SQLite index
```

`relay-core` owns trust, parcel parsing, repair, commit and receipt decisions.
The desktop moves work to a background thread and displays actual group health.
The SQLite database is an index; verified published batch files authorize recovery.
NNCP is a design reference, not a dependency or compatible transport protocol.

## Boundaries

This is a hackathon implementation, without an independent security audit.
Carrier media may be damaged or untrusted; profile directories and the host OS
must remain trusted. Same-user hostile processes that race filesystem changes
are outside the current protection model. Repair cannot exceed a group's parity
budget or recover a lost carrier. Keep your protected identity profile backed up;
there is no password reset or key-recovery service.

Recipients save into app-owned inbox directories. The tool handles static regular
files, not folders, symlinks, applications, live databases, filesystem permissions
or arbitrary metadata. It never executes received content. It has no accounts,
cloud service, multi-hop routing or remote execution. A receipt is the recipient
key's statement of a verified save, not evidence of human readership or permanent
durability. Physical power-loss behavior, real USB media and two-machine exchange
still require field testing.

- [Project plan (Chinese)](docs/PROJECT_PLAN.zh-CN.md)
- [Implementation contract](docs/IMPLEMENTATION_CONTRACT.md)
- [Demo and verification limits](docs/DEMO.md)
- [Install prebuilt packages](docs/INSTALL.md)
- [Release process and provenance](docs/RELEASE.md)
- [Kiro development evidence](docs/KIRO_USAGE.md)
- [Review closure (Chinese)](docs/REVIEW_CLOSURE.zh-CN.md)

