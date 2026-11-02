# Guided demo — design

The original Kiro-authored design preceded code. Codex reviewed the contract
before implementation and consolidated this final document after review;
see [provenance](../../evidence/guided-demo.md).

## Shared public contract

The existing dependencies suffice. `relay-core/src/demo.rs` owns one engine;
CLI and desktop import its crate-root re-exports. The session is opaque and is
never serialized. The public DTO contains only synthetic public facts.

```rust
pub enum DemoPhase { Prepared, Damaged, Received, Confirmed }

pub struct DemoSnapshot {
    pub phase: DemoPhase,
    pub root: PathBuf,
    pub carrier_parcel_path: Option<PathBuf>,
    pub source_sha256: BTreeMap<String, String>,
    pub received_sha256: BTreeMap<String, String>,
    pub initial_inspection: Option<Inspection>,
    pub inspection_after_damage: Option<Inspection>,
    pub five_erasure_refusal_evidence: Option<Inspection>,
    pub five_erasure_receive_error: Option<String>,
    pub damaged_shard_indices: Vec<usize>,
    pub sender_state: Option<DeliveryState>,
    pub repaired_shards: Option<usize>,
    pub received_path: Option<PathBuf>,
    pub receipt_path: Option<PathBuf>,
    pub report_path: Option<PathBuf>,
}

impl DemoSession {
    pub fn start(root: impl Into<PathBuf>) -> anyhow::Result<Self>;
    pub fn snapshot(&self) -> DemoSnapshot;
    pub fn advance(&mut self) -> anyhow::Result<DemoSnapshot>;
    pub fn export_report(&mut self) -> anyhow::Result<PathBuf>;
}
```

`DemoPhase` derives Clone, Debug, PartialEq, Eq, Serialize and Deserialize.
`DemoSnapshot` derives Clone, Debug, Serialize and Deserialize. Optional fields
are absent in JSON until their facts exist. Paths in snapshots are canonical.

## State and retained evidence

| Operation | Real work and resulting boundary |
| --- | --- |
| start → Prepared | Atomic create-new root; initialize Alice/Bob; export/import cards; pin both contacts; generate source_files/file_a.bin (20 MiB) and file_b.bin (1 MiB); seal and export; inspect actual groups; read Alice WaitingReceipt. |
| advance → Damaged | Delete carrier data 0/1; atomically corrupt parity 20/21; inspect four erasures. Corrupt fifth shard 2, attempt and refuse real receive, check no commit/index/receipt and no successful receipt export. Restore fifth from Alice's outbox; retain actual four/five inspections and error. |
| advance → Received | Real receive; require repaired_shards == 4; use the returned committed files path; hash both files; compare to source; read Alice still WaitingReceipt. |
| advance → Confirmed | Export Bob's signed receipt; import it into Alice; require returned and indexed Confirmed state. |
| export_report | Only Confirmed; create root/demo-report.json without replacement. Serialize the report_path into the same public snapshot. Repeated export errors preserve the original. |

Advancing Confirmed is an error. A failed operation does not advance the public
snapshot, although partially completed synthetic files can remain. Core tests
restore external damage before retry; the GUI offers a fresh run after an error.
All outputs remain inspectable. No automatic deletion or resume mechanism is
introduced. Shard reads use existing bounded/no-symlink filesystem primitives;
replacement uses a private temporary file and atomic persistence, preserving
external hardlink contents.

## CLI and desktop

CLI dispatches Demo before default_home, Profile::open or password handling.
It starts, advances three times and exports the report. Text mode describes real
results; JSON mode emits only the final report snapshot.

Desktop adds Welcome and Demo without changing Cmd1–4. Try guided demo starts
an operation; Set up my device enters Identity. Demo is also reachable through
its tab and CmdD. Each run chooses a new OS-temp child using process ID and clock
nanoseconds; create-new root semantics protect collisions.

The existing Action → work::execute → Completed channel remains the only worker
path. DemoStart creates the session. DemoAdvance moves Box<DemoSession> into the
worker and returns it with Box<DemoSnapshot>. Demo actions return without
refreshing the real profile. Confirmed automatically exports the report before
returning the final snapshot. DemoCopyReport performs a create-new copy to the
user's selected file. No actor, Arc/Mutex or new dependency is added.

The view renders actual data/parity health using missing-shard indices, complete
source/received SHA-256 strings, receipt-ready versus confirmed state, refusal
error and output paths. The JSON summary is unsigned; only the delivery receipt
is signed. A spinner corresponds only to actual pending work.

Verification authority: [PR checks](https://github.com/BrickerP/relaypost/pull/1/checks).
