# Kiro development evidence

The initial RelayPost implementation was developed with Codex. On 2026-11-02,
a real Kiro CLI v2 session reviewed receive recovery and generated a regression
candidate. Codex applied and simplified that candidate during review.

| Evidence | Record |
| --- | --- |
| Actual session | `10ae599a-fca2-4185-b27c-ce5a7663be5e` |
| Task | Refuse recovery/re-import of an externally deleted, still-indexed inbox batch while retaining its valid staged copy. |
| Contribution | One integration regression in `crates/relay-core/tests/profile_exchange.rs`. |
| Authorship | Kiro generated the complete candidate; Codex applied and simplified it. |
| Delivery and validation | [PR #1](https://github.com/BrickerP/relaypost/pull/1) and its [remote checks](https://github.com/BrickerP/relaypost/pull/1/checks). |

[The factual session record](../.kiro/evidence/indexed-deletion-regression.md)
contains the prompt summary, tool-approval outcome, observed CLI versions,
and review adjustments. Original candidate/logs remain in ignored local
`artifacts/kiro-2026-11-02/`; credentials and private configuration are not tracked.

All Rust compilation, formatting and tests run in GitHub Actions. This contribution
records actual Kiro use; it does not claim Kiro authored the initial product or
guarantee hackathon eligibility.
