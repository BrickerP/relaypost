# Novice desktop UX: actual Kiro contribution

Date: 2026-11-03. This records work performed during the RelayPost UX redesign,
not a reconstruction of the original application's authorship.

## Actual session

- Kiro CLI session: `141b6526-64d9-4803-b702-c3bffba6cab0`.
- Agent: the repository's `relaypost-product`, with only `read` and `write`
  tools allowed; no shell, MCP, local Rust commands or subagents.
- Invocation used `chat --agent relaypost-product --trust-tools=read,write
  --no-interactive --output-format stream-json` with the actual redesign brief.
- The process returned exit code 0; the recorded `runFinished` event reports
  `status: success` and `stopReason: end_turn`.
- Its stream contains 15 tool calls and 15 completed tool-call updates. Eight
  completed edit calls created/revised the five distinct files below. The other
  seven calls read project instructions, design input and current source, then
  checked the resulting files. The raw local stream is retained in ignored
  `artifacts/ux-redesign/kiro-ui-session.jsonl`; it is not a public fixture.

## Files Kiro actually wrote

| File | Contribution |
|---|---|
| `.kiro/specs/novice-ux/requirements.md` | Requirements for novice tasks, truthful states, accessibility and security boundaries |
| `.kiro/specs/novice-ux/design.md` | Native theme/component design and integration contract |
| `.kiro/specs/novice-ux/tasks.md` | Implementation and remote verification tasks, with incomplete work identified |
| `crates/relay-desktop/src/theme.rs` | Bundled Regular/Medium font registration, color/spacing/typography configuration |
| `crates/relay-desktop/src/components.rs` | Accessible task surfaces/buttons/headings and the fact-driven handoff ribbon |

The completed edit-call sequence was requirements, design, tasks, theme,
components, theme, components, theme. File creation claims above come from the
recorded `tool_call_update` events, each with `kind: edit`, `status: completed`
and the corresponding source-file location.

## Codex work and review amendments

Codex authored the design brief supplied to that session, sourced the original
font assets and their license, integrated the new modules, and implemented the
task-oriented application workflow in `app.rs`, `main.rs` and `work.rs`.
This includes Home/People/Send/Receive/Activity navigation, setup continuation,
password sheets, drag-and-drop, bounded drive discovery, automatic inspection,
confirmation-return actions, opening verified saved folders and recovery UI.
Codex also authored five bounded-discovery tests for remote CI.

After reviewing Kiro's modules, Codex set consistent rounded widget/primary
button corners and changed the source headers to make remote validation's
pending status explicit. Codex also added explicit Done / Current step / Not yet
text to the ribbon, so phase meaning does not depend on color or painted marks.
Static review additionally fixed wrong-recipient
inspection, local-copy attention priority, failed-receive confirmation gating,
cached receive-state refresh and modal navigation isolation. These amendments
are credited to Codex, not to Kiro.

People exposes a copy action for this computer's complete original safety code.
The verification sheet directs the other person to copy their code from People
and send it through a separate trusted channel. It still requires a complete
matching input (allowing whitespace between groups), not automatic trust.
Codex updated task statuses to distinguish source implementation from CI/runtime
verification; a compact rail and decorative step transitions remain deferred.

## Verification status

Source changes are implemented. `git diff --check` passed. No local cargo,
rustc, rustfmt, Clippy, Rust tests or language-server compilation was run.
Compilation, formatting, Clippy, core/CLI/desktop tests and native packaging
remain GitHub Actions responsibilities. Runtime usability and visual checks
must use that exact CI-built candidate; source implementation alone is not
evidence that a first-time user completed a two-computer exchange.

The app still uses the same production core. Complete safety-code comparison,
mutual trust, operation-scoped zeroizing passwords, signed receipt binding,
no-overwrite publication and the distinction between delivery phase and local
copy health remain required. No runtime online AI feature was added.
