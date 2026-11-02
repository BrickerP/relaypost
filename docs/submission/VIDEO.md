# Native demonstration plan — 164 seconds

This is a recording script, not evidence that recording has happened. Record the
actual CI-built macOS app on an Apple Silicon Mac after the exact candidate
passes CI. No generated footage, dummy progress or staged success text. The
matching [captions](captions.vtt) are planned timings; align them to the final
recording before upload.

## Capture setup

- Record only the app and an uncluttered desktop; conceal personal notifications
  and unrelated profiles. Use a fresh synthetic demo root.
- Record the app binary SHA-256, candidate commit and successful CI URL beside
  the original footage. Do not invent a test count in a video overlay.
- Keep actual full hashes and group facts legible by choosing a reasonable
  window size. Crop personal paths if necessary without concealing outcomes.
- The visible status must settle at each stage before the next click. If an
  operation takes longer than its allotted scene, cut waiting time with an
  explicit “processing time shortened” label. Never speed up or replace an
  outcome to fit a script.
- Use the actual final button labels. The actions below describe their purpose
  until those labels are verified. Keep the total published cut below 180s.

## Storyboard and narration

| Time | Real on-screen action | Spoken English |
| --- | --- | --- |
| 00:00–00:17 | Open the native app's guided demo. Keep “one-machine / directory carrier” visible. | “What if two computers cannot be online together? RelayPost brings back store-and-forward: prepare an encrypted parcel, carry it, and return proof of a verified save. This is the real native Mac app.” |
| 00:17–00:35 | Start a new demo; show actual preparation until Prepared is reached. | “The guided demo creates two disposable identities and twenty-one mebibytes of synthetic files. Everything stays in a new directory. This recording uses one machine and a directory carrier. It does not demonstrate physical USB media or a second computer.” |
| 00:35–00:51 | Show Prepared, carrier path, real group layout and WaitingReceipt. | “The sender has exported a real encrypted, signed parcel. Its first group has twenty data shards and four parity shards; the smaller tail has its own budget. Exported still means waiting for a receipt.” |
| 00:51–01:15 | Advance to Damaged. Show actual four-erasure inspection and separate five-erasure refusal evidence. | “Now we damage two data shards and two parity shards. Their hashes expose all four erasures. That group remains repairable. A separate five-erasure attempt is refused: no committed batch, and no success receipt. The demo retains that evidence, then restores the fifth shard.” |
| 01:15–01:43 | Advance to Received. Show four repaired shards, matching actual source/received hashes, received path and sender WaitingReceipt. | “Receive runs the production engine. It reconstructs the ciphertext, checks its digest, decrypts, and validates the files before publishing a complete batch. Four shards were repaired. These are the actual source and received SHA-256 values, and they match. The receiver has saved the files, but the sender is still waiting.” |
| 01:43–02:05 | Advance to Confirmed. Show returned receipt and sender Confirmed. | “Only now do we carry the signed receipt back. The sender verifies the expected recipient and the parcel binding before becoming confirmed. That distinction is the point: carried, repaired, saved, and confirmed are separate facts. The receipt proves this verified save, not human readership or permanent storage.” |
| 02:05–02:25 | Export/show actual JSON report path; keep real native results visible. | “The command line and this interface share one Rust demo engine. The report preserves its real inspections, repaired count, hashes and states. UUCP inspired the workflow; NNCP and repair tools remain modern baselines. RelayPost's experiment is bringing the complete handoff into one native journey.” |
| 02:25–02:44 | Keep completed app visible; show brief link card for actual Kiro evidence, exact CI and repository. | “Kiro CLI contributed specifications and implementation work, with the actual session records and review changes linked in the repository. The initial product used Codex. All Rust builds and tests run in GitHub Actions. This prototype still needs two-device and physical-media testing, and an easier signed installation.” |

The Kiro sentence is supported by the
[actual guided-demo sessions](../../.kiro/evidence/guided-demo.md). Keep Codex's
initial authorship and final review amendments visible in the linked usage
record; do not imply Kiro wrote all code or that a throttled session completed.
Use only first-party footage, original voice or an authorized narration voice,
and no unlicensed background music. A silent cut may use these English captions.

## Public video metadata draft

**Title:** RelayPost — encrypted offline handoff, real repair, verified receipt

**Description:** A native macOS walkthrough of RelayPost's shared Rust guided
demo: 21 MiB of synthetic data, four-shard recovery, five-shard refusal, matching
file hashes, and receipt-gated confirmation. Recorded on one Mac using two
isolated profiles and a directory carrier. Physical USB and two-device exchange
have not been tested. Source / working build / exact CI / actual Kiro evidence:
**TBD: approved public URLs**. Category: Resurrection.

No upload is authorized by this script. The public YouTube URL remains TBD until
the owner approves the final cut and upload.
