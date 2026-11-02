# Implementation contract

This contract coordinates parallel implementation. The user approved the v0.1 project plan and implementation. Do not compile or run Rust locally. All verification belongs to GitHub Actions.

## Ownership

- Core worker owns crates/relay-core including its Cargo.toml and src. Root may request fixes but does not overwrite concurrent edits.
- Interface worker owns crates/relay-cli and crates/relay-desktop including their manifests.
- Verification worker owns .github/workflows, crates/relay-core/tests, scripts and docs/DEMO.md. Coordinate dev-dependencies with core worker.
- Root owns workspace manifest, repository setup, project documentation, Kiro integration, PR and CI orchestration.

## Public API of relay_core

Export Profile from the crate root. Profile stores paths rather than an open shared SQLite connection, can be cloned, and is safe to move into a UI background worker. Methods return anyhow::Result. Public data types are serde serializable and Clone/Debug; secret types are not Debug/Serialize.

Required methods (paths below are &std::path::Path unless noted):

```text
Profile::open(home: impl Into<PathBuf>) -> Result<Profile>
profile.initialize(name: &str, password: &str) -> Result<PeerCard>
profile.public_card() -> Result<PeerCard>
profile.export_card(output: &Path) -> Result<PeerCard>
profile.import_contact(path: &Path) -> Result<Contact>
profile.trust_contact(fingerprint: &str) -> Result<()>
profile.contacts() -> Result<Vec<Contact>>
profile.seal(inputs: &[PathBuf], recipient: &str, password: &str) -> Result<ParcelRecord>
profile.export_parcel(parcel_id: &str, carrier: &Path) -> Result<PathBuf>
profile.inspect_parcel(parcel: &Path) -> Result<Inspection>
profile.receive_parcel(parcel: &Path, password: &str) -> Result<ReceiveResult>
profile.export_receipt(parcel_id: &str, carrier: &Path) -> Result<PathBuf>
profile.import_receipt(receipt: &Path) -> Result<ParcelRecord>
profile.deliveries() -> Result<Vec<ParcelRecord>>
profile.recover(password: &str) -> Result<RecoverySummary>
profile.discard_incomplete_receives() -> Result<usize>
profile.cancel(parcel_id: &str) -> Result<()>
```

Profile::open may create the isolated app home and state database but must not create keys. initialize refuses an existing identity. import_contact verifies the self-signed binding and stores an unverified contact; trust_contact pins its complete fingerprint. Both sender and recipient need to trust one another before exchange. Self-card import/trust and self-addressed parcels are refused.

Public data fields:

```text
PeerCard { version:u32, name:String, fingerprint:String, age_recipient:String,
           signing_public_key:String, signature:String }
Contact { card:PeerCard, verified:bool }
Direction = Outgoing | Incoming
DeliveryState = Ready | WaitingReceipt | SavedReceiptReady | Confirmed | Failed | Cancelled
ParcelRecord { parcel_id:String, direction:Direction, state:DeliveryState,
               sender:String, recipient:String, files:usize, total_bytes:u64,
               parcel_path:Option<PathBuf>, received_path:Option<PathBuf>,
               receipt_path:Option<PathBuf>, local_error:Option<String>,
               created_at:String, updated_at:String }
GroupInspection { index:usize, data_shards:usize, parity_shards:usize,
                  valid_shards:usize, missing_shards:Vec<usize>, recoverable:bool }
Inspection { parcel_id:String, sender:String, recipient:String,
             ciphertext_bytes:u64, groups:Vec<GroupInspection>, recoverable:bool }
ReceiveResult { record:ParcelRecord, repaired_shards:usize, duplicate:bool }
RecoverySummary { recovered:usize, pending:usize, conflicts:Vec<String> }
```

Export these types at crate root. Fingerprints are full stable fingerprints of the canonical age and Minisign public keys and version domain. Do not store raw passwords or unlocked keys in Profile.

## Core invariants

- age recipient encryption, Minisign authenticated peer/envelope/receipt records, SHA-256, GF8 Reed-Solomon. Read upstream docs/types before implementation. Do not invent a cipher or NNCP compatibility layer.
- 64 MiB total raw files, 100 static regular files; UTF-8 relative paths, no symlinks or unsafe/colliding names. No original ownership/xattr semantics.
- Full recovery groups: 20 data + 4 parity, maximum 1 MiB per shard. Tail n=ceil(group_bytes/1MiB), r=ceil(n/5), equal shard length ceil(group_bytes/n), zero padding; authenticate and validate all lengths/limits.
- Envelope limit 64 KiB, ciphertext limit 80 MiB, at most 4 groups and 24 shards per group, internal manifest limit 256 KiB, path depth 16 and UTF-8 path size 1024 bytes.
- parcel_id is SHA-256 of the immutable age ciphertext. Envelope signs kind/schema, both trusted identities, recipient age and receipt keys, lengths, group parameters and every shard digest. Verify exact raw signed envelope bytes; reject duplicate JSON fields and conflicting signed copies. Hash-bad shards become erasures before reconstruction.
- Retrying/exporting the same parcel reuses exact bytes and ID. A new seal is a new delivery even for identical plaintext.
- Only commit after ciphertext digest, age authentication, archive and file-list checks pass. Use same-filesystem private staging, prepared metadata, file/dir sync and atomic no-replace directory publication. Linux CI and macOS use their actual no-replace primitives; no racy exists()+rename replacement fallback. Unsupported filesystems fail clearly.
- Receiver commit authority is verified published content/metadata, with SQLite as its index. Recover crashes between publication, index and receipt. Never automatically sign success for externally changed content.
- Receipt binds kind/schema, parcel_id, envelope digest, committed manifest digest and recipient signing identity. Only pinned expected key can confirm outgoing delivery. No success receipt on unrecoverable/tampered input.
- Cancel stops local re-export only, never recalls exported copies or deletes source files.
- Outgoing state preserves its delivery phase. Local retained-copy damage is stored in optional `local_error`, cleared when its bytes verify again. It never turns `WaitingReceipt` into `Ready` or overrides signed confirmation/cancellation. `Failed` remains an incoming content-conflict state.
- Carrier receipts contain exactly receipt JSON/signature. Local acknowledgment artifacts contain those two files plus the exact authenticated original envelope JSON/signature. Recover verifies acknowledgments independently of local shards before reconciling outbox health. No receipt-only local-acknowledgment fallback is accepted.
- Recover verifies prepared receive stages before exclusive publication, cleans superseded staging only after matching the verified final batch, and keeps partial/conflicting work pending. Explicit discard removes only owned `inbox/.staging/batch-*` receive directories under the profile lock and syncs the staging parent. It does not affect committed batches or source files.
- Reject special file metadata before opening carrier records; Unix reads use nonblocking/no-follow flags plus descriptor type validation so FIFO input cannot hold the profile lock indefinitely.

## Interface behavior

The guided onboarding/demo extension has one shared `DemoSession` engine in
`relay-core`, reused by noninteractive CLI and staged desktop actions. Its exact
public DTO, four phases, isolation and evidence requirements are defined in
[the used Kiro specification](../.kiro/specs/guided-demo/design.md). Every demo
run uses a fresh private root, synthetic profiles and real production APIs.
The JSON report is an unsigned evidence summary; the return receipt is signed.

Use English UI and help, 4 screens: Send, Receive, History, Identity & Contacts. Show real group health and outcomes, no fake progress/results. Keep expensive work off the UI thread.

CLI shape: global --home; identity init/export, contacts import/trust/list, seal --to, export --carrier, inspect, receive, receipt export/import, deliveries, recover, cancel. `recover --discard-incomplete` performs only explicit staging discard and prints `discarded_staging`; no password is needed. Desktop discard requires a confirmation dialog. History selections use `(parcel_id, direction)` and show local-copy health separately. Password through masked interactive input, --password-stdin, or a password file; never a plaintext password argument. Structured --json output must not contain secrets.

## Verification

Core unit tests cover codec and crypto properties; independent integration tests cover two profiles, trust, normal roundtrip, full/tail recovery budgets, corrupted parity, over-budget refusal, manifest/receipt tamper, wrong recipient, duplicate import, externally modified destination and recovery of committed-but-unindexed batches. All cargo build/test/fmt/clippy commands run remotely only.

