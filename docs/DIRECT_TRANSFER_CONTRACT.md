# Local-network delivery contract

Approved scope: 2026-11-03 user decision, direct delivery on the same local
network with explicit offline packages for disconnected exchange. No cloud
relay, account service, Internet traversal or remote execution.

`relay-core` remains the only trust, save and receipt authority. `relay-direct`
treats the network as an untrusted carrier. The desktop integrates its bounded
background command/snapshot API; socket work never blocks an egui frame.

## Modules and source ownership

- Core executor: `relay-core/src` and core integration tests. No network
  dependency, duplicate cryptography or new database table.
- Direct executor: new `crates/relay-direct`, including its public handle,
  discovery, framing, service lifecycle and network tests.
- UI executor: desktop `app.rs` and `work.rs`.
- Root: workspace/dependency wiring, CI, packaging, documentation and final
  source/GUI acceptance. All Rust execution occurs in GitHub Actions.

## Core boundary

The exact public DTOs are implemented in `relay-core`; consumers use those
types instead of redefining them. The accepted boundary is:

| API | Authority |
| --- | --- |
| `inspect_contact_card` / `import_contact_card` | Verify a public self-signed card; importing never grants trust. |
| `prepare_direct_handoff` | Verify the retained parcel and produce a guarded streamed bundle plus signed offer artifact. |
| `inspect_direct_offer` | Existing envelope authentication, pinned sender and intended local recipient; no save. |
| `stage_direct_bundle` / `direct_offers` | Verify and atomically publish/recover managed encrypted incoming packages. |
| `record_direct_handoff` | Enter WaitingReceipt only after the receiver ACKs a complete staged copy. |
| `receipt_artifact` | Return signed save evidence only after revalidating committed content. |
| `import_receipt_artifact` | Existing parcel/envelope/manifest/recipient binding and retained acknowledgment authority. |
| `finalize_direct_offer` | Remove only owned staged packages whose legitimate receipt already exists. |

`DirectOfferArtifact` contains `schema`, the envelope JSON and Minisign text,
and exact `bundle_bytes`. `DirectOffer` exposes parcel ID, sender/recipient
fingerprints, file count, plaintext/ciphertext lengths and bundle length.
`PreparedDirectHandoff` owns its temporary bundle and provides `bundle_len()`
and `open_bundle()`. `StagedDirectParcel` contains `parcel_path` and the existing
`Inspection`. `ReceiptArtifact` carries only receipt JSON and Minisign text.
Wire artifacts reject unknown fields.

The bundle is deterministic tar containing only envelope/signature copies and
canonical shard paths. The parser manually checks every regular-file entry;
it rejects absolute/parent paths, duplicates, unexpected entries, symlinks,
hardlinks, devices, FIFO, oversized and truncated bodies. Maximum bundle size
is 100 MiB. The archive is streamed, not buffered in memory.

Incoming encrypted packages live under
`profile/direct/incoming/<parcel_id>.parcel`, published with the existing
exclusive no-replace filesystem primitive. Repeated identical delivery is
idempotent; conflicts are rejected. Staging does not decrypt, publish user
files or sign success. Existing `receive_parcel()` remains the only save path.

## Discovery and pairing

Use `mdns-sd = { version = "=0.21.7", default-features = false }` with
`_relaypost._tcp.local.`. No HTTP or async runtime is added. The instance name
and service ID are ephemeral. TXT contains only protocol version and service
ID, never filenames, private keys or a trust assertion. Fetch and verify the
public card before presenting a device. Exclude self by fingerprint, not IP;
two profiles on one host have different identities. First implementation uses
IPv4 private, link-local and loopback endpoints.

Retain at most 64 latest resolved candidates when a probe is busy or temporarily
fails. The coordinator fairly retries due candidates with bounded concurrency;
healthy probes prefer the last successful address only while it remains in the
latest resolved set and verify the same full fingerprint. Service removal,
expiry and address replacement prevent an older in-flight result from reviving
a stale candidate. Successful health checks renew both candidate and peer life.

The daemon monitor must surface binding/permission/discovery failures. Explicit
shutdown waits for daemon/listener completion rather than relying on Drop.
Upstream references: [API](https://docs.rs/mdns-sd/0.21.7/mdns_sd/),
[source and changelog](https://github.com/keepsimple1/mdns-sd).

Pairing exchanges public cards and queues a request for the other user. Accept
imports an unverified identity. Each user separately compares every group of
the complete safety code and explicitly grants local trust through the core.
PeerAccepted means only that the public card was accepted; it does not claim
that the other user has granted trust. Unknown sender offers cannot silently
create contacts or bypass this flow.

## Bounded TCP carrier

Each connection carries one request/response, with an eight-byte `RLYPST1\n`
magic/version, one-byte request kind, big-endian u64 body length and exact body.
Kinds are Card, PairingRequest, PairingStatus, Offer, OfferStatus, Bundle and
Receipt. JSON frames are at most 140 KiB and reject unknown fields. A Bundle
frame starts with the 64-character parcel ID and streams the bounded archive.
The exact accepted encoding is defined in the implementation and its tests;
it exposes no filesystem path or execution endpoint.

Bound active connections to four, pending offers to eight, pending bundles to
two, idle reads/writes to 30 seconds and an operation to ten minutes. Never
hold the Profile lock across a socket operation. Shutdown must interrupt live
sockets and complete within two seconds.

An offer is authenticated and matched to the local recipient before entering
the decision queue. The recipient explicitly accepts before large bundle data
is accepted. Only complete verified staging returns a handoff ACK. Received
bytes, saved files and verified confirmation remain separate facts.

## Handle and UI lifecycle

`DirectHandle::spawn(Profile, Arc<dyn Fn() + Send + Sync>)` returns immediately.
`snapshot()` clones bounded public state; `command(DirectCommand)` uses a
bounded queue; `shutdown()` is explicit and idempotent. Errors distinguish
queue-full/stopped/shutdown timeout. `on_change` only requests egui repaint.

The shared contract exposes:

- Service state: DormantNoIdentity, Starting, Ready or an actionable
  Unavailable reason (listener/discovery/local-network).
- Nearby peers: session peer ID, verified public card and local trust fact.
- Incoming pairing requests and outgoing pairing attempts, including waiting,
  accepted, declined and retryable failure; never an indefinite fake success.
- Incoming offers: AwaitingDecision, actual byte progress, ReadyToSave with
  authenticated parcel/inspection, declined or failed.
- Outgoing transfers: offering, waiting for recipient, actual byte progress,
  waiting for save, confirmed, declined or failed.

Commands cover visibility/discovery, request/accept/decline pairing,
send/accept/decline transfer, retry and ReloadProfile. A generation tracks
snapshot changes; profile_revision advances only after actual core changes.
The desktop refreshes its core facts once per profile revision. No identity is
a valid dormant state; Initialize followed by ReloadProfile starts networking.

After staging, the receiver uses the existing explicit password/save action.
The generated signed receipt is fetched by exact parcel ID and imported via
the core. Sender restart resumes receipt polling from WaitingReceipt plus the
pinned recipient fingerprint; receiver restart discovers managed staged
packages. No second durable transfer state machine is added.

## Verification and platform limits

Core tests cover untrusted/wrong-recipient/tampered offers, unsafe tar entries,
truncation/limits, duplicate/conflict staging, no premature receipt, a complete
signed round trip and recovery. Network integration tests use two independent
profiles and loopback ports, covering consent, decline, transfer, save,
receipt, interruption, retry and restart. Discovery record/event mapping and
self-identity filtering are tested without requiring CI multicast access.

Loopback does not prove physical two-device Wi-Fi discovery. GUI acceptance
and actual two-device evidence are recorded separately. Source must pass
remote formatting, tests, Clippy and exact-source packaging before release.

Same-host GUI QA uses independently linked programs from the same source and
dependency lock, with distinct rustc metadata salts, main executable UUIDs and
bundle identifiers. The manifest records each identity and signed binary hash.
Copying one executable and only changing bundle names/identifiers is not a
valid independent local-network privacy setup; Apple documents UUID collisions
in [TN3178](https://developer.apple.com/documentation/technotes/tn3178-checking-for-and-resolving-build-uuid-problems).
These QA apps are not the public release distribution.

macOS bundles explain local-network use with
`NSLocalNetworkUsageDescription` and declare `_relaypost._tcp` in
`NSBonjourServices`. The macOS app is not sandboxed; do not add unrelated
entitlements. Apple explicitly states the multicast entitlement is not
required on macOS. Local-network denial must remain visible and retryable;
never bypass privacy settings or require root. See
[Apple TN3179](https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy).
