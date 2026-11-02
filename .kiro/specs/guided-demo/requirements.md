# Guided demo — requirements

Kiro authored and used the original specification before implementation on
2026-11-02. Codex reviewed it before implementation and consolidated these final
requirements afterward; the [evidence record](../../evidence/guided-demo.md)
preserves the authorship and session order.

RelayPost demonstrates an offline encrypted delivery on one machine using real
production operations. The carrier is a directory; the walkthrough does not
claim a physical USB transfer or two independent devices.

| ID | Acceptance requirement |
| --- | --- |
| R1 | When the first refresh finds no identity, show a product explanation, **Try guided demo**, and **Set up my device**. Trying the demo starts it without creating the user's keys. Setup opens the existing Identity screen. An existing unreadable identity routes to Identity with its error. Keep the four production screens, Cmd1–4, and always-accessible Demo / CmdD. |
| R2 | When a demo starts, atomically create a new private root (0700 on Unix), refusing any existing file, directory or symlink. Create only synthetic Alice and Bob profiles, sibling carrier, cards and two deterministic files totalling exactly 21 MiB beneath it. Never consult real identity, contacts, inputs or password files. Restart creates a new root and preserves every prior output. |
| R3 | In **Prepared**, use real initialize, card exchange, trust, seal, export and inspection APIs. Record exact source SHA-256 and actual outgoing WaitingReceipt state. Require a healthy full 20-data + 4-parity group; derive all tail counts from actual inspection, including archive/encryption overhead. |
| R4 | In **Damaged**, delete carrier data shards 0 and 1 and hash-corrupt parity shards 20 and 21. Temporarily corrupt shard 2, actually attempt receiving, and require a repair-budget refusal with no receiver commit, incoming index or exportable receipt. Retain its actual error and inspection. Restore shard 2 from the immutable sender copy and confirm precisely four remaining erasures before advancing. |
| R5 | In **Received**, use the real receive API, require four repaired shards, hash both committed files and match every source digest. Keep Alice WaitingReceipt until the signed receipt is carried back. |
| R6 | In **Confirmed**, export Bob's signed receipt and import it into Alice; require the actual Confirmed state. Only then write the create-new JSON demo report with its own report_path included. The aggregate JSON is unsigned public facts; its referenced delivery receipt is signed. Do not serialize passwords or private keys. |
| R7 | CLI `relaypost demo --output NEW_DIRECTORY [--json]` runs all stages without pauses before resolving/opening the real profile. JSON stdout is only the final snapshot and agrees with the report file. GUI pauses at the four real boundaries, performs slow work through the existing background channel, renders actual health/hashes/states, and offers a create-new report copy through a save dialog. |
| R8 | Carrier mutation uses bounded reads, refuses symbolic-link paths and special files, and replaces entries atomically so a hardlinked external file's bytes remain unchanged. Failures preserve the public phase; the GUI surfaces the error and offers a fresh run. No fake progress, timers, speculative success states, new dependencies or local Rust execution. |

Independent acceptance evidence lives in
`crates/relay-core/tests/guided_demo.rs` and `scripts/demo.py`: raw shard hashes,
ciphertext reconstruction, source/received file hashes, actual index and receipt
bindings, refused reuse, link isolation, unchanged unrelated synthetic profile,
and fresh restart. Compilation, formatting, Clippy and tests are verified only by
[the delivery PR checks](https://github.com/BrickerP/relaypost/pull/1/checks).
