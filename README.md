# RelayPost

Recoverable encrypted parcels for devices that do not need to be reachable at the same time.

RelayPost reimagines UUCP-style store-and-forward delivery as a local Rust CLI and native desktop application. It combines recipient encryption, signed manifests, bounded Reed-Solomon repair, verified batch commits, and signed delivery receipts.

The implementation is developed on a pull request. Compilation and tests run in GitHub Actions; no local compilation is used.

- [Project plan (Chinese)](docs/PROJECT_PLAN.zh-CN.md)
- [Implementation contract](docs/IMPLEMENTATION_CONTRACT.md)
- [Kiro development evidence](docs/KIRO_USAGE.md)

The application does not provide cloud synchronization, remote execution, or a guarantee of recovery after losing an entire carrier. A receipt records the recipient key's statement about a verified commit, not human readership or permanent durability.

