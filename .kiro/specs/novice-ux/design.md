# RelayPost — Novice UX Redesign: Design

Status: Approved 2026-11-03. Source-authored; CI validation pending.

---

## 1  Design thesis

RelayPost should feel like a calm offline delivery assistant. A nontechnical
user should answer three questions without learning protocol vocabulary:

1. Who are these files going to?
2. What should I do next?
3. Has the other computer actually saved them yet?

The redesign uses one memorable product-specific element: the **handoff ribbon**.

```
[ This Mac ]  ──  [ On the drive ]  ──  [ Other Mac ]  ──  [ Confirmation back ]
  prepared             carry it             saved               delivered
```

Completed stages use verified facts only. The active stage contains the next
physical instruction. Future stages are quiet and never imply remote progress.

---

## 2  Layout

### 2.1  Window dimensions

- Design target: 1120 × 800 px.
- Minimum supported: 880 px wide.

### 2.2  Left rail

- Width: 184 px (normal); 64 px with icon + tooltip at ≤ 880 px.
- Background: `#16292B` (Rail).
- Product mark at top; navigation items in the middle; device name and Help at
  the bottom.
- No persistent footer. Network-independence messaging belongs in onboarding/Help.

### 2.3  Main content

- Max content width: 820 px, centered in remaining width.
- Outer horizontal margins: 40 px; vertical: 32 px.
- At minimum width: outer padding ≥ 32 px.
- Min useful main area target: ~600 px.

---

## 3  Color tokens

| Token | Hex | Use |
|---|---|---|
| INK | `#172326` | Primary text |
| MUTED | `#667377` | Supporting text, quiet labels |
| PAPER | `#F3F4EF` | Main background (canvas) |
| LINE | `#DDE2DF` | Quiet borders |
| RAIL | `#16292B` | Left navigation background |
| TEAL (primary) | `#087F72` | Main action button, current stage |
| PRIMARY_HOVER | `#06695F` | Hover / active on primary button |
| GREEN | Computed from teal family | Verified success, confirmed |
| AMBER | Amber family | Waiting / action needed |
| RED | Red family | Unrecoverable / destructive only |
| MINT_TINT | `#DFF3EC` | Verified success background |
| WAITING_TINT | `#FAEED7` | Waiting state background |
| DANGER_TINT | `#FBE7E2` | Error state background |

Rule: never use color alone; pair each with a heading or short sentence.

---

## 4  Typography

### 4.1  Font families

- **Proportional body**: Atkinson Hyperlegible Next Regular — all UI text.
- **Display/headings**: Atkinson Hyperlegible Next Medium — page titles,
  primary results.
- **Monospace**: IBM Plex Mono or platform fallback — fingerprints, parcel IDs,
  file hashes only.

Fonts are bundled as TTF assets. If font loading fails, fall back to the
platform proportional font. Never use monospaced text for navigation or status.

### 4.2  Scale

| Role | Size | Weight family |
|---|---|---|
| Page title | 30 px | Medium |
| Result title | 24 px | Medium |
| Section label | 11 px | Medium (all-caps) |
| Body | 16 px | Regular |
| Button | 15 px | Medium |
| Supporting | 13 px | Regular |

Content line length: ~68 characters. Do not stretch prose full-width.

---

## 5  Spacing and shape

- Base grid: 8 px.
- Page title → first content: 24 px gap.
- Between major groups: 24 px; between related controls: 8–12 px.
- Surface internal padding: 24 px (desktop), 18 px at minimum width.
- Surface corner radius: 14 px.
- Surface border: 1 px, LINE color.
- Primary button: 44 px min height, 16 px horizontal padding, 10 px corner
  radius, TEAL fill, white medium text.
- Form fields: 42 px min height, labels above, help below only when actionable.

---

## 6  Handoff ribbon

### 6.1  Phases

| Index | Label | Verified fact |
|---|---|---|
| 0 | Prepared | Parcel sealed locally (Ready) |
| 1 | Copied to drive | Carrier export recorded (WaitingReceipt) |
| 2 | Files saved | Signed receipt from recipient exists |
| 3 | Confirmed | Signed receipt verified by sender |

Phase 1 ("Copied to drive") does not claim the destination has received
anything. "Confirmed" requires the signed receipt to be verified.

### 6.2  Visual treatment

- Completed phases: TEAL/GREEN fill, solid circle connector.
- Current active phase: TEAL border, bold label.
- Future phases: LINE border, MUTED label.
- Connectors between phases: LINE color line; TEAL for completed segments.
- Drawn with `Painter` primitives (circles and lines); labels via normal egui
  labels so accessibility tree is intact.
- No animation. No numeric fake progress.
- Minimum width target: 600 px available. At narrower widths, phase labels may
  abbreviate or wrap; circles and connectors remain.

---

## 7  Surface component

- White fill.
- 1 px LINE border.
- 14 px corner radius.
- 24 px internal margin on all sides.
- Width bounded to available UI width (`ui.set_width(ui.available_width())`).

---

## 8  Primary button

- TEAL (`#087F72`) fill.
- White medium-weight text, 15 px.
- 44 px minimum height.
- 16 px horizontal padding.
- 10 px corner radius.
- Disabled state: muted fill, MUTED text.

---

## 9  Empty state

- Rendered inside a surface.
- Bold title at ~19 px.
- Supporting body in MUTED.
- 12 px vertical padding above and below.

---

## 10  Page title component

- Heading text: 30 px, medium, INK.
- Subtitle: 16 px, MUTED.
- 24 px gap below subtitle before first content.

---

## 11  Section label

- 11 px, medium weight, MUTED, all-caps.
- 4 px gap below before content.

---

## 12  Motion policy

- 140–180 ms fade/position transition when a task step is replaced by its
  verified result.
- Spinner for real background work only.
- Do not animate shard repair, remote-device progress or carrier movement.
- Respect truth over spectacle.

---

## 13  Code boundaries

The novice-UX Rust implementation introduces two new source files in
`crates/relay-desktop/src/`:

| File | Purpose |
|---|---|
| `theme.rs` | Color constants, font registration, style configuration |
| `components.rs` | Reusable primitives: page_title, section_label, surface, primary, empty_state, handoff_ribbon |

Additional screen modules (home, send, receive, activity, people) and
integration into `app.rs` are the responsibility of the Codex integration agent.

`theme.rs` and `components.rs` expose only a public API; they never mutate
`RelayApp` state, never call `relay-core` directly, and have no side effects
beyond rendering.

---

## 14  egui 0.31.1 API notes

- `Frame::new()` (not `Frame::default()`).
- `corner_radius` field on `Frame`; takes `impl Into<CornerRadius>` accepting
  scalar `i8`/`u8` values.
- `Margin::same(i8)`, `Margin::symmetric(i8, i8)`.
- `Painter::rect_stroke` requires four arguments: rect, rounding, stroke,
  `StrokeKind`.
- `FontDefinitions::font_data` values are `Arc<FontData>`.
- Font family name registration: `FontFamily::Name("RelayDisplay".into())`.

---

## 15  Limitations and pending work

[ ] NOT YET IMPLEMENTED — left-rail navigation shell and screen routing.
[ ] NOT YET IMPLEMENTED — Home adaptive next-step card.
[ ] NOT YET IMPLEMENTED — Send flow redesign (compose → prepared → carry).
[ ] NOT YET IMPLEMENTED — Receive flow redesign (auto-inspect → result pages).
[ ] NOT YET IMPLEMENTED — Activity screen with timeline.
[ ] NOT YET IMPLEMENTED — People screen with safety-code comparison flow.
[ ] NOT YET IMPLEMENTED — First-use setup wizard (3-step).
[ ] NOT YET IMPLEMENTED — "Waiting for confirmation" prompt on Home.
[ ] NOT YET IMPLEMENTED — Collapsible technical record in Activity detail.
[ ] NOT YET IMPLEMENTED — Motion transitions (state change fades).
[ ] NOT YET VERIFIED — CI compilation of theme.rs and components.rs.
[ ] NOT YET VALIDATED — First-time user acceptance checks (§16 of requirements).
