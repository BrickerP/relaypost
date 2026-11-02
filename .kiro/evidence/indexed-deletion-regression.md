# Indexed inbox deletion regression: actual Kiro contribution

Date: 2026-11-02. Actual Kiro CLI session:
`10ae599a-fca2-4185-b27c-ce5a7663be5e` (`v2`, 26 messages in the local
JSON session listing; updated `2026-11-02T02:46:33.977Z`).
The pre-session CLI version observation was `2.37.0`; the post-session
observation was `2.37.1`. The precise runtime build was not independently
captured, and no explicit upgrade command was run during this work.

The actual prompt asked Kiro to read the project rules, implementation
contract, receive/recovery implementation, and existing synthetic integration
helpers. Its task was to author one regression for a valid stale prepared
receive stage when the final inbox batch had been externally deleted but its
incoming index row still existed. It prohibited terminal commands, local Rust
compilation/tests/formatting, external tools, unrelated reads, dependencies,
and production edits.

Kiro read `AGENTS.md`, `docs/IMPLEMENTATION_CONTRACT.md`,
`crates/relay-core/src/profile.rs`, and
`crates/relay-core/tests/profile_exchange.rs`, then generated a complete
168-line candidate. Its direct file writes were denied by non-interactive
approval; an attempted shell fallback was also denied. Codex applied the
complete returned Rust block, then simplified it during review: public
`deliveries()` proves the indexed incoming row instead of a SQL query;
implementation commentary was shortened; a two-iteration loop retains the
recovery, receipt, and receive-retry assertions. The final test is therefore
**Kiro generated; Codex applied and simplified during review**, not an
unchanged Kiro output.

Contribution: appended
`recovery_refuses_to_resurrect_an_indexed_but_externally_deleted_final_batch_from_valid_stage`
in `crates/relay-core/tests/profile_exchange.rs`. It checks zero recoveries,
one pending stage, a conflict, an absent final destination, unchanged staged
file digests, unavailable success receipts, and repeated refusal to replace
the deleted destination. No production code or dependency was changed.

Raw prompt, session log, generated candidate, JSON session listing, and
original hashes are retained locally under ignored
`artifacts/kiro-2026-11-02/`. No account email, credential, token, or private
configuration is in this evidence file. This file is a Codex-authored factual
summary; it does not assert that a steering file was used.

Required validation is GitHub Actions compilation, formatting, Clippy and tests.
The contribution is delivered through [PR #1](https://github.com/BrickerP/relaypost/pull/1);
its [checks](https://github.com/BrickerP/relaypost/pull/1/checks) record the result for
each commit. No local Rust checks were run. This contribution does not by itself
establish hackathon eligibility.
