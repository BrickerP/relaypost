# RelayPost / 驿站

**Carry encrypted files offline. Repair damaged pieces. Verify their arrival.**

RelayPost revives UUCP's store-and-forward idea as a Rust CLI and native desktop
tool. Two devices exchange public identity cards once, pin complete fingerprints,
and carry encrypted parcels through a USB drive or any ordinary directory. They
never need to be online together. A signed return receipt confirms a verified
recipient commit.

## Try the native application

1. Open a successful [Verify and package run](https://github.com/BrickerP/relaypost/actions/workflows/ci.yml).
2. Download `relaypost-macos-arm64`, extract it, and verify the ZIP against its
   adjacent SHA-256 file. The ZIP contains `RelayPost.app` and its bundled CLI.
3. Open the app and create an identity in **Identity & Contacts**. On the other
   device do the same, exchange public cards, compare the complete fingerprints
   through a trusted channel, and pin trust on both devices.
4. In **Send**, select a trusted recipient and regular files, seal a parcel, and
   export it to a carrier directory. The recipient uses **Receive** to inspect
   and save it, exports its receipt, and returns that receipt to the sender.

The macOS bundle is an Apple Silicon prototype with an ad-hoc signature, without
Developer ID signing or notarization. Core and CLI are tested on Linux and macOS;
desktop builds currently target macOS only. Windows is not supported. Compilation
and Rust checks run in GitHub Actions, with no local compilation.

For a completely isolated two-profile example, see the [runnable demo](docs/DEMO.md).
It executes the real prebuilt CLI, damages synthetic data, verifies repair,
checks duplicate import, and returns a signed receipt. `--home DIRECTORY` gives
either interface its own profile; otherwise they share the OS application-data
directory. Run the bundled `relaypost --help` for commands. Use a masked prompt,
`--password-stdin`, or `--password-file`; never put a password in command arguments.

## What works

| Operation | Actual behavior |
| --- | --- |
| Identity | X25519 age recipient key and Minisign signing key, bound in a self-signed contact card. Private keys are stored in a passphrase-encrypted age file. |
| Seal | Snapshot 1–100 regular files, at most 64 MiB total, hash a bounded manifest, encrypt it, and sign the parcel envelope. Plaintext filenames remain inside the encrypted archive. |
| Repair | Each full group has 20 data and 4 parity shards. Tail groups use a smaller explicit budget. Missing and bad-hash pieces count as erasures within that group. |
| Receive | Authenticate the trusted sender, reconstruct ciphertext, check its SHA-256, decrypt, validate every file, and publish one complete batch with an OS-exclusive rename. |
| Confirm | Save a receipt signed by the expected recipient only after commit. The sender stays `WaitingReceipt` until the returned receipt verifies. |

**History** shows local delivery records, recovery results and cancellation.
Re-importing the same parcel verifies and reuses its committed batch. Recovery
rebuilds a missing index or receipt from verified published files; changed files
produce a conflict. Cancellation stops future local exports and cannot recall
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
- [Kiro development evidence](docs/KIRO_USAGE.md)

