# Kiro development evidence

The initial RelayPost implementation was developed with Codex. The guided-demo
iteration used actual Kiro CLI sessions to author the specification first,
amend it after review, and directly implement a useful shared engine, CLI
command, first-use onboarding and staged desktop walkthrough. Codex configured
the scoped agent, directed review, supplied independent tests, and handles final
review amendments and delivery.

The [guided-demo evidence](../.kiro/evidence/guided-demo.md) records the exact
sessions, successful direct file writes, service-throttle interruptions and
coauthorship boundary. Its real inputs are the
[requirements](../.kiro/specs/guided-demo/requirements.md),
[design](../.kiro/specs/guided-demo/design.md) and
[tasks](../.kiro/specs/guided-demo/tasks.md), created by Kiro before implementation.
The [project agent](../.kiro/agents/relaypost-product.json) exposes only file
read/write tools and reads the project steering when directed; it cannot run
local Rust commands. All Rust validation runs in GitHub Actions.

## Earlier receive-recovery regression

On 2026-11-02, a real Kiro CLI v2 session reviewed receive recovery and generated
a regression candidate. Codex applied and simplified that candidate during review.

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
