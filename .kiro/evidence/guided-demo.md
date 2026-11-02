# Guided demo: actual Kiro spec and implementation

Date: 2026-11-02. Runtime observed: Kiro CLI `2.37.1`, agent engine `v2`.
This is a Codex-authored factual evidence summary, not a Kiro-generated account
of its own contribution.

## Work order and provenance

Codex configured the repository-local `relaypost-product` agent and supplied the
approved product scope. Its only enabled and trusted tools are `read` and
`write`; it has no shell, MCP, subagent or code-intelligence tools. The config
was validated by the installed Kiro CLI before the session. Canonical
`read,write` trust entries allowed real file edits; the earlier regression
session had used non-matching file-tool trust names.

Kiro first read the existing core and UI, then directly wrote
`.kiro/steering/product-development.md` and all three
`.kiro/specs/guided-demo/` documents. Codex reviewed the actual specification
before authorizing implementation. Review required real five-erasure receive
refusal, noninteractive CLI dispatch before real-profile access, first-use
onboarding, existing worker channels, create-new private roots, actual group
counts, and receipt-gated confirmation. Kiro amended the specification before
writing feature code.

Kiro then directly wrote the shared core engine and CLI command, and completed
the first-use welcome and staged desktop UI through its actual file tools. The
implementation calls the existing `Profile` APIs on two synthetic profiles,
uses two deterministic files totalling 21 MiB, retains real inspection/hash/
receipt evidence, and exposes a shared public `DemoSession` contract. The
independent integration tests and release work belong to other Codex workers.

| Actual session | Observed contribution and outcome |
| --- | --- |
| `9ce85e84-a3a7-4b3b-aa71-e83cb0347978` | Read existing implementation; directly wrote steering, requirements, design and tasks; completed. |
| `678c118b-b63a-419f-b78c-e3ce7d7629f6` | Amended reviewed specs; directly wrote core demo, CLI and partial desktop integration; interrupted by service throttling. |
| `28713f93-488c-46ee-9747-607fd798f0a6` | Corrected mixed missing/corrupt damage, removed redundant inline tests, fixed CLI macro/imports and partial welcome integration; interrupted by service throttling. |
| `f6b305cb-cbde-43ae-bac8-d9f816428447` | Completed staged desktop UI in the existing app, existing per-click worker channels, first-use demo start, automatic final report and create-new export copy; completed. |
| `8b3dee5f-8940-4438-9dc3-82f62f39bdba` | Added actual unavailable-receipt probe during five-erasure refusal, fail-closed I/O checks, actual 20+4 and WaitingReceipt boundary guards; completed. |
| `87470dcb-f1e3-476b-b22e-355c500dde01` | Read and planned final review corrections; throttled before any file writes. No implementation is attributed to this session. |

## Review boundary

This is **Kiro-authored spec and feature implementation, with Codex review and
final amendments**, not a claim that Kiro authored the initial product or that
its first candidate passed CI unchanged. The initial product remains
Codex-authored, as recorded in the earlier regression evidence.

Codex final review covers bounded atomic replacement of damaged carrier shards,
accurate per-kind shard counts, unsigned JSON-report wording, receipt-ready
wording and failed-session controls. After the last service throttle, Codex
stopped retries and took over those review edits. Codex also consolidated final
spec documents and task statuses, preserving the contract Kiro used. These
amendments remain attributed to Codex; they are not retroactively claimed as
Kiro writes. No denied or throttled edit is counted as completed.

Raw prompts, complete session logs and session-list JSON are retained under
ignored local `artifacts/kiro-onboarding-2026-11-02/`. They are not submission
assets. This tracked summary contains no account email, token, credential,
private profile or secret key. Synthetic fixture passwords are confined to
source, never to the public report DTO.

All Rust compilation, formatting, Clippy and tests are remote-only. No local
Rust tools were run and no unobserved CI pass is asserted here. The authoritative
result for each candidate is recorded by [PR #1 checks](https://github.com/BrickerP/relaypost/pull/1/checks);
final amendment scope and release verification are recorded by the delivery PR
and release evidence.
