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

## Novice desktop redesign

On 2026-11-03, actual Kiro CLI session
`141b6526-64d9-4803-b702-c3bffba6cab0` authored the novice UX requirements,
design and task specification, plus the native `theme.rs` and `components.rs`.
The file-only session completed successfully. Its components provide font,
spacing and color configuration, accessible task controls and a handoff ribbon
whose states represent actual local facts.

Codex supplied the design brief, sourced the original licensed fonts, integrated
the modules, implemented the production task orchestration and bounded drive
discovery, and authored independent desktop logic tests. Review amendments,
packaging and runtime walkthroughs remain separately attributed to Codex.

The [novice UX evidence](../.kiro/evidence/novice-ux.md) records the actual write
events and review changes. The corresponding
[requirements](../.kiro/specs/novice-ux/requirements.md),
[design](../.kiro/specs/novice-ux/design.md) and
[tasks](../.kiro/specs/novice-ux/tasks.md) distinguish source implementation from
remote checks and actual usability evidence. A generated layout is not proof
that an independent novice completed a two-device exchange.

## User-feedback and direct-delivery iteration

The subsequent 2026-11-03 user-feedback iteration removes Home, simplifies
recipient setup and adds local-network direct delivery as the primary workflow.
That implementation and its transport/security tests are authored by Codex;
the earlier Kiro inputs are retained as historical evidence rather than
relabeled as the new flow. The current contracts are
[product flow](UX_PRODUCT_FLOW.md) and
[direct delivery](DIRECT_TRANSFER_CONTRACT.md). The original Kiro visual
foundation and shared walkthrough remain in use with separately attributed
integration changes.

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
