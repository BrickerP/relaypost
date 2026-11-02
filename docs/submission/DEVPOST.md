# RelayPost — submission draft

This is review copy, not a submitted entry. Replace the explicit `TBD` links
after publication is approved and verify every statement against the submitted
commit and recorded app. No award eligibility is asserted here.

## Project name

RelayPost / 驿站

## Elevator pitch

Carry encrypted files offline, repair damaged pieces, and confirm a verified
save with a signed return receipt. A native resurrection of store-and-forward.

## Category

Resurrection. No optional bonus-category claim is included in this draft.

## Inspiration

UUCP made a useful promise in an intermittently connected world: prepare work
locally, carry it when a route becomes available, and process it at the other
end. RelayPost reimagines that workflow for people handing files between
computers that cannot be online together.

A possible use is carrying a static batch of research files to an offline
workstation and returning evidence that it saved the correct bytes. This is a
target scenario, not a claimed field deployment. A copied folder alone leaves
the operator to coordinate encryption, damage recovery, file verification and
confirmation by hand.

Store-and-forward is still alive. [NNCP](https://www.nncpgo.org/) is a modern
reference, and tools such as
[par2cmdline](https://github.com/Parchive/par2cmdline) already repair damaged
data. RelayPost does not claim to invent these
mechanisms or replace NNCP's routing features. Its experiment is a smaller,
one-to-one native workflow in which repair and confirmation share one set of
delivery facts. A fair comparative usability study remains future work.

## What it does

Two device identities exchange public contact cards. The operator compares full
fingerprints through a trusted channel and pins both contacts. The sender seals
static regular files into a recipient-encrypted, signed parcel and exports it
to an ordinary carrier directory.

The receiver authenticates the sender and envelope, checks shard hashes,
reconstructs within each group's actual repair budget, verifies the complete
ciphertext, decrypts and validates every file. It publishes a complete batch
without replacing an existing destination and only then signs a receipt.

Exporting a parcel leaves the sender **WaitingReceipt**. Even a successful save
on the receiver does not change that. Only the expected recipient's verified
return receipt makes the sender **Confirmed**. A receipt records the recipient
key's verified save; it does not prove that a human read the files or that the
storage will last forever.

The native guided demo and CLI use the same Rust demo engine. It creates two
disposable identities and 21 MiB of synthetic files, shows a full 20-data/4-parity
group plus a smaller tail, and pauses at the actual export, damage, receive and
confirmation stages. Two missing data shards and two corrupt parity shards are
repaired; original and received SHA-256 values must match. A separate five-shard
case refuses reception and produces no success receipt. No result is a canned
animation. This is a one-machine, directory-carrier simulation; no physical USB
or two-device test is claimed.

## How we built it

Rust provides one shared core for a clap CLI and an egui native interface.
Existing age and Minisign libraries provide recipient encryption and signatures;
SHA-256 identifies content, and Reed–Solomon supplies grouped erasure repair.
SQLite indexes delivery state; verified published files and authenticated
metadata authorize recovery. No NNCP runtime or protocol compatibility is
included.

All Rust builds, formatting, linting and tests run in GitHub Actions. The
submission's exact commit, successful run, checksums and recorded binary must be
linked in the release evidence before this copy is finalized. We do not use a
test count from an older commit as evidence for the candidate.

## How Kiro was used

The initial product was implemented with Codex. Actual Kiro CLI work is
documented separately from that authorship. Kiro first generated a receive
recovery regression candidate that Codex applied and simplified. See
[the session evidence](../../.kiro/evidence/indexed-deletion-regression.md) and
[the Kiro usage record](../KIRO_USAGE.md).

For the guided-demo iteration, an actual repository-local Kiro CLI agent used
read/write tools to inspect the existing code, write steering and a three-part
specification, then implement the shared engine, CLI command and staged native
interface. The strategy was to review the specification first, especially the
receipt boundary and real failure evidence, then have Kiro implement that
contract through existing core and worker APIs. Independent Codex workers wrote
acceptance tests and release tooling. Kiro service throttling interrupted some
sessions; Codex made the final safety and presentation corrections. Those edits
remain credited to Codex.

[The guided-demo evidence](../../.kiro/evidence/guided-demo.md) identifies each
actual session, direct file writes, interrupted work and review amendments.
Root `.kiro/` contains the agent, steering and specs used in that work. No IDE,
Crew, hook, model or MCP use is claimed; this agent's tools were read/write only.

## Challenges and lessons

“Copied,” “repairable,” “saved” and “confirmed” are different facts. The hard
boundary is allowing neither a plausible progress bar nor a damaged local copy
to rewrite them. Repair must happen before the authenticated payload is
accepted; confirmation must wait for a receipt that actually returns.

A 21 MiB sample deliberately crosses the 20 MiB full-group boundary. It exposes
the tail's real budget and makes the four-versus-five failure boundary visible.
It is a demonstration fixture, not a throughput benchmark. Crash-window tests
recreate durable states; they do not simulate a physical power cut.

## What's next

Test a complete handoff on two Macs with physical removable media, improve the
installation experience, and compare repeated handoffs against a fairly
configured NNCP-plus-repair workflow. Measure operator steps and mistaken
confirmation judgments before expanding scope.

## Built with

Rust, egui/eframe, clap, age, Minisign, SHA-256, reed-solomon-erasure, SQLite,
Kiro CLI, Codex, GitHub Actions.

## Submission links

- Source: [RelayPost on GitHub](https://github.com/BrickerP/relaypost) — visibility
  must be verified after approval.
- Working application / stable download: **TBD: public candidate release URL**.
- Demonstration: **TBD: public YouTube URL, under three minutes**.
- Candidate commit and successful CI: **TBD: exact commit and run URL**.
- English testing instructions: [TESTING.md](TESTING.md).
- Development evidence: [KIRO_USAGE.md](../KIRO_USAGE.md) and
  [guided-demo sessions](../../.kiro/evidence/guided-demo.md).

## Current limits

Apple Silicon macOS desktop prototype; no Developer ID signature or notarization
is claimed. Input is limited to 100 static regular files and 64 MiB per batch.
There is no cloud recovery, folder synchronization, multi-hop routing or remote
execution. Trusted profile storage and a trusted host are required. There is no
independent security audit or measured field reliability result.
