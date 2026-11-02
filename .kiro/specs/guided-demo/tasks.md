# Guided demo — implementation and verification record

Kiro wrote the original task breakdown before implementation. Codex consolidated
this status record after the actual source writes. A source change is not a
claim that compilation or tests passed. The exact validation result belongs to
[PR #1 checks](https://github.com/BrickerP/relaypost/pull/1/checks).

| Task | Source and authorship | Verification evidence |
| --- | --- | --- |
| Specify useful demo and shared API | Kiro wrote steering, requirements, design and tasks; Codex reviewed before implementation and consolidated final docs. | [Actual sessions](../../evidence/guided-demo.md). |
| Implement shared synthetic engine | Kiro directly wrote DemoSession, DTOs, real Profile operations, four stages, hashes and report. Codex final review adds bounded atomic shard mutation. | Independent core integration tests inspect real files, index and receipts. |
| Prove refusal and confirmation boundaries | Kiro added actual fifth-erasure receive refusal, no commit/index/exportable receipt, restored fifth shard, WaitingReceipt and Confirmed guards. | Four/five inspections, actual error and receipt binding in the public report; independent refusal tests. |
| Implement noninteractive CLI | Kiro directly added Demo before real profile/password handling, source-based summaries and JSON report output; corrected its macro and imports. | scripts/demo.py supplies an invalid real home and missing password file, then independently validates output and refused reuse. |
| Implement first-use and guided desktop | Kiro completed welcome, direct demo start, staged UI, Box<DemoSession> worker movement, auto report and save-dialog copy. Codex reviews final health counts, accurate wording and failed-session controls. | Native macOS interaction and release artifacts accompany remote compilation. |
| Add independent acceptance tests | Codex QA wrote core integration tests and external process/report validation, including carrier link isolation. Kiro's redundant inline tests were removed. | Remote CI; no local Rust invocation. |
| Deliver release and submission assets | Codex release/QA workers own manifests, CI, package scripts, release metadata, install/submission docs and media evidence. | PR checks and the release package's own manifest and verification records. |

Remaining work must be read from current PR checks and release evidence, not
inferred from these source statuses. No final media, CI pass, submission or
eligibility result is asserted by this document.
