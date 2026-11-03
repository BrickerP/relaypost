# RelayPost — Novice UX Redesign: Tasks

Status: Approved 2026-11-03. Kiro owns items marked [K]; Codex integration agent
owns items marked [C]. CI validation pending for all source-authored items.

---

## Phase 1 — Foundation (Kiro)

### T-01  theme.rs [K]

[x] SOURCE AUTHORED — CI validation pending

Public color constants: INK, MUTED, PAPER, LINE, TEAL, GREEN, AMBER, RED, RAIL.
Public `configure(ctx: &egui::Context)` function that:
- Loads `AtkinsonHyperlegibleNext-Regular.ttf` via `include_bytes!` as default
  proportional font.
- Loads `AtkinsonHyperlegibleNext-Medium.ttf` via `include_bytes!` and registers
  it as `FontFamily::Name("RelayDisplay".into())`.
- Keeps existing default font definitions for emoji and monospace coverage.
- Sets light visuals, override text color INK, selection colors.
- Sets text styles: Body 16 px, Button 15 px, Heading 30 px (medium family),
  Small 13 px.
- Sets interact_size.y = 44, item_spacing, button_padding.
- Uses only existing eframe/egui 0.31.1 APIs; no new crates.

### T-02  components.rs [K]

[x] SOURCE AUTHORED — CI validation pending

Public functions:
- `page_title(ui, title, subtitle)` — heading + MUTED subtitle + 24 px gap.
- `section_label(ui, label)` — 11 px MUTED all-caps + 4 px gap.
- `surface(ui, contents)` — white 14 px-radius 1 px-border 24 px-margin panel,
  width bounded to available.
- `primary(ui, text, enabled) -> Response` — 44 px min-height TEAL rounded
  button with white medium text.
- `empty_state(ui, title, body)` — surface with bold title and MUTED body.
- `handoff_ribbon(ui, stage: usize)` — four-phase ribbon; phases ≤ stage are
  completed/current, later phases quiet; drawn with Painter primitives + egui
  labels; no animation; no fake progress; stage 1 does not claim delivery.

---

## Phase 2 — Screen modules (Codex integration agent)

### T-03  Navigation shell [C]

[x] SOURCE IMPLEMENTED — CI and runtime validation pending

Replace `Screen` enum and top-tab navigation in `app.rs` with left-rail
navigation: Home, Send, Receive, Activity, People.
Rail: 192 px with explicit task labels. A compact icon-only rail is deferred to T-12; no second navigation mode was added in this candidate.
Background: RAIL color from theme.rs.
Move Demo to Help menu and empty-Home card.

### T-04  Home screen [C]

[x] SOURCE IMPLEMENTED — CI and runtime validation pending

Adaptive next-step card from local verified facts (see requirements §5).
Recent handoff rows below the prioritized next step. Failed/local-copy attention takes precedence without changing the recorded delivery phase.
"Try guided walkthrough" card when no deliveries.
No invented remote events; no chat/AI elements.

### T-05  Send screen redesign [C]

[x] SOURCE IMPLEMENTED — CI and runtime validation pending

Compose state → Prepared state → Ready-to-carry state.
Use `components::surface`, `components::primary`, `components::handoff_ribbon`.
Password sheet opens after `Prepare transfer` click.
Technical details (parcel ID, ciphertext, repair groups, paths) in collapsible
`Technical details` section, not on the task page.

### T-06  Receive screen redesign [C]

[x] SOURCE IMPLEMENTED — CI and runtime validation pending

Single `Choose transfer folder…` action; auto-run inspect_parcel on selection.
Route to: ready-to-receive panel, cannot-recover panel, or saved panel.
Shard grid in `Repair details` collapsible, not top-level.
Use plain-language result panels from design §7.3.

### T-07  Activity screen [C]

[x] SOURCE IMPLEMENTED — CI and runtime validation pending

List/detail layout with explicit To/From device names.
Detail pane: truthful handoff_ribbon + relevant action + local record details.
Codex scope amendment: the core exposes current phase and local timestamps, not a persistent event timeline; no historical timestamps or remote events are invented.
More actions section for cancel, discard, recover.
Collapsed technical record.

### T-08  People screen [C]

[x] SOURCE IMPLEMENTED — CI and runtime validation pending

This-device card with safety code.
Contact list with "Ready to exchange" / "Not checked" badges.
Add-contact flow with safety-code comparison sheet (full fingerprint shown,
instruction text from requirements §4.3, primary `The complete code matches`).
Both-devices trust requirement preserved; self-import refused.

### T-09  First-use setup wizard [C]

[x] SOURCE IMPLEMENTED — CI and runtime validation pending

Three-step wizard: (1) Set up this Mac, (2) Add the other computer,
(3) Check the safety code.
Route to Home after completion.
Password help text from requirements §4.1.

### T-10  Home waiting-for-confirmation prompt [C]

[x] SOURCE IMPLEMENTED — CI and runtime validation pending

When outgoing `WaitingReceipt` exists: show named prompt from requirements §8.
After ImportReceipt success: show named confirmation result.

---

## Phase 3 — Polish and validation

### T-11  Motion transitions [C]

[ ] DEFERRED — decorative step transitions are not implemented

140–180 ms fade when a task step is replaced by its verified result.
Spinner for background work only.
No animation for shards, remote progress or carrier movement.

### T-12  Minimum-width layout [C]

[ ] RUNTIME CHECK PENDING — fixed labelled rail implemented; compact rail deferred

Original proposed compact layout: at 880 px rail collapses to 64 px with tooltips; main content
keeps ≥ 32 px outer padding; handoff_ribbon remains legible.

### T-13  CI compilation [SHARED]

[ ] NOT YET VERIFIED — requires GitHub Actions run

theme.rs and components.rs must compile without errors or warnings under
`cargo check` and `cargo clippy` on the CI branch.

### T-14  First-time user validation [SHARED]

[ ] NOT YET VALIDATED

A nontechnical evaluator completes the full two-device workflow without author
guidance; correctly distinguishes "Copied to drive / Waiting for confirmation" from "Delivery confirmed"; does not need
technical details to proceed. See requirements §16.

---

## Ownership summary

| Task | Owner | Status |
|---|---|---|
| T-01 theme.rs | Kiro | [x] Source authored; CI pending |
| T-02 components.rs | Kiro | [x] Source authored; CI pending |
| T-03 Navigation shell | Codex | [x] Source implemented; CI/runtime pending |
| T-04 Home screen | Codex | [x] Source implemented; CI/runtime pending |
| T-05 Send redesign | Codex | [x] Source implemented; CI/runtime pending |
| T-06 Receive redesign | Codex | [x] Source implemented; CI/runtime pending |
| T-07 Activity screen | Codex | [x] Source implemented; CI/runtime pending |
| T-08 People screen | Codex | [x] Source implemented; CI/runtime pending |
| T-09 Setup wizard | Codex | [x] Source implemented; CI/runtime pending |
| T-10 Confirmation prompt | Codex | [x] Source implemented; CI/runtime pending |
| T-11 Motion transitions | Codex | [ ] Deferred |
| T-12 Min-width layout | Codex | [ ] Fixed labelled rail; runtime check pending, compact rail deferred |
| T-13 CI compilation | Shared | [ ] Not yet verified |
| T-14 User validation | Shared | [ ] Not yet validated |
