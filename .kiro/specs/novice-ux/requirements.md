> Historical Kiro input from the first UX iteration. The 2026-11-03 user-feedback revision supersedes its Home/People navigation and file-only primary flow; follow [the current product flow](../../../docs/UX_PRODUCT_FLOW.md). This annotation and the subsequent revision are by Codex.

# RelayPost — Novice UX Redesign: Requirements

Status: Approved 2026-11-03. Source-authored; CI validation pending.

---

## 1  Purpose

The current desktop UI exposes the implementation model before the user's task.
Numbered tabs, parcel IDs, shard blocks, ciphertext sizes and receipt
terminology make a rigorous tool look like an administrator console.

This redesign replaces that presentation with a calm, task-oriented flow
suitable for a first-time, nontechnical user while preserving every security and
repair invariant in `relay-core`.

---

## 2  Scope

- Owned files: `crates/relay-desktop/src/**` and `.kiro/specs/novice-ux/**`.
- Core library (`relay-core`), CLI, manifests, CI, scripts and release docs are
  out of scope and must not be modified for this work.
- No new crates; use existing `eframe`, `egui`, `rfd`, `relay-core`, `zeroize`.

---

## 3  Navigation structure

Replace `Send / Receive / History / Identity & Contacts / Demo` top tabs with a
fixed left rail. Rail width 184 px at normal size; collapses to 64 px with
icon+tooltip at ≤ 880 px window width.

| Destination | User-facing label | Purpose |
|---|---|---|
| Home | Home | Next-step card + in-progress handoffs |
| Send | Send | Compose, seal and export to carrier |
| Receive | Receive | Inspect, repair and save; return receipt |
| Activity | Activity | Timeline and recovery actions |
| People | People | This device, trusted contacts, add and verify |

Demo moves out of primary navigation to Help menu and empty-Home card.

---

## 4  First-use experience

### 4.1  Set up this Mac (step 1 of 3)

- Title: "Set up this Mac"
- Body: "RelayPost gives this Mac a private identity so another computer can
  send files only to it."
- Fields: device name, password, confirm password.
- Password help text: "This password unlocks RelayPost on this Mac. There is no
  online account or password reset."
- Primary action: `Create this Mac's identity`
- Do not lead with "key" or "cryptography" vocabulary in the task layer.

### 4.2  Add the other computer (step 2 of 3)

Show two equal visual actions:

- **Share this Mac** → `Save my connection card…`
- **Add another Mac** → `Open their connection card…`

Help text: "A connection card contains public information only. It cannot open
your files."

Do not say "export public keys" in this context.

### 4.3  Check the safety code (step 3 of 3)

After import, show the other device's name and the **complete fingerprint**
labelled as **Safety code** in grouped blocks.

Instruction: "Compare every group while you are together or on a call. If both
screens match, you know files will go to the intended computer."

Primary: `The complete code matches`
Secondary: `Not now`

A confirmation dialog repeats the name and full code before pinning.

**Security contract**: the full fingerprint must never be truncated or silently
omitted. Both devices must pin trust independently before exchange.

Status labels for contacts: "Not checked" (unverified), "Ready to exchange"
(verified/pinned). "UNVERIFIED" / "FINGERPRINT PINNED" remain in technical
details only.

---

## 5  Home: adaptive next-step card

Home derives a single recommended action from locally verified facts only.
No invented remote events. No chat, no assistant avatar, no AI label.

Priority order:

| Condition | Recommended action |
|---|---|
| No identity | `Set up this Mac` |
| No trusted contacts | `Add the other computer` |
| Outgoing `Ready` | `Copy prepared files to a drive` |
| Outgoing `WaitingReceipt` | `Add confirmation from the returned drive` |
| Incoming `SavedReceiptReady` | `Put confirmation on the drive` |
| Error / conflict present | `Review what needs attention` |
| Otherwise | Show Send and Receive task cards |

Below the next-step card: **In progress** handoff cards (concise). Below those:
recently completed handoffs (visually quiet).

---

## 6  Send flow

### 6.1  Compose state

- Title: "Send files"
- Subtitle: "Prepare a protected copy for another computer."
- Recipient picker: shows name + "Ready to exchange" badge; safety code hidden
  under "Connection details".
- File zone: large drop/select area; rows with name, human size, remove icon;
  one-line total.
- Summary: "N files · X MiB · only [Name]'s Mac can open them · includes damage
  recovery."
- Primary: `Prepare transfer` — opens password unlock sheet after click.

### 6.2  Prepared state

Outcome page (not appended results).

- Title: "Transfer ready"
- Body: "Choose a USB drive or folder."
- Primary: `Choose drive or folder…`

After export:

- Title: "Ready to carry"
- Instruction: "Eject the drive and take it to [Name]. This transfer is not
  delivered yet."
- Handoff ribbon active stage: **On the drive**
- Primary: `Done`; secondary: `Copy to another drive…`

Technical details (parcel ID, ciphertext size, repair groups, local path,
timestamps) available under `Technical details`. Not shown by default.

---

## 7  Receive flow

- Title: "Receive files"
- Subtitle: "Choose the RelayPost transfer on the drive."
- Start: `Choose transfer folder…` (directory picker with bounded discovery).
  After selection, run `inspect_parcel` automatically.

### 7.1  Ready to receive

Single status panel:

"From [Name]'s Mac — Ready to receive. RelayPost checked the sender and found
enough healthy pieces. N damaged pieces can be repaired automatically."

Primary: `Save files on this Mac…` → password sheet → existing receive
operation.

### 7.2  Cannot recover

"This copy is too damaged. Ask [Name] to copy the prepared transfer to the drive
again."

Primary: `Choose another copy…`; secondary: `See repair details`.

Group/shard grid stays in `Repair details`, not the top-level page.

### 7.3  Saved

"Files saved on this Mac. N files were checked byte for byte. RelayPost also
prepared a confirmation for [Name]."

Primary: `Put confirmation on the drive…`

Ribbon highlights **Confirmation back** without claiming sender verification.

---

## 8  Returning confirmation to sender

On Home when `WaitingReceipt`:

"Waiting for [Name]'s confirmation. If the drive has returned, add its
confirmation to finish this delivery."

Primary: `Add confirmation from drive…`

After verification: "[Name]'s Mac confirmed that all N files were saved
correctly."

Never say "receipt verified" or "confirmed" alone; name the device and fact.

---

## 9  State language

| Core state | Primary user label | Supporting sentence |
|---|---|---|
| `Ready` | Prepared | "The protected transfer is ready to copy to a drive." |
| `WaitingReceipt` | On its way | "Copied to a carrier. Waiting for [Name]'s Mac to confirm the save." |
| `SavedReceiptReady` | Saved here | "Files were verified and saved. The confirmation is ready to return." |
| `Confirmed` | Delivered | "[Name]'s Mac confirmed that the complete batch was saved." |
| `Failed` | Needs another copy | Specific corrective action first, internal exception under Details. |
| `Cancelled` | Stopped | "No more copies can be exported here. Copies already carried away still work." |

"Copying to a drive" is never described as "sent" or "delivered".

---

## 10  Activity

List/detail layout. Each row:

- Direction indicator (drawn with egui primitives, not emoji).
- Other device name.
- File count and size.
- Plain-language state.
- Relative local time.

Detail pane begins with handoff ribbon and the one relevant action. Below it:
a fact timeline (`Prepared on this Mac`, `Copied to carrier`, `Saved on
[Name]'s Mac`, `Confirmation verified here`).

Destructive actions (cancel, discard) in **More actions** section with
confirmation sheet.

Technical record (paths, IDs, fingerprints, timestamps) in a collapsed section.

---

## 11  People

- This device card: name, safety code, export card button.
- Trusted contacts: name, "Ready to exchange" / "Not checked" badge.
- Add flow: import card, then safety-code comparison (step 4.3 above).
- Both devices must pin trust independently. Self-import refused by core.

---

## 12  Visual / typography requirements

- Fonts: Atkinson Hyperlegible Next Regular (body) and Medium (display/headings),
  bundled as TTF assets under `crates/relay-desktop/assets/fonts/`.
- Body text: 16 px proportional. Buttons: 15 px. Page titles: 30 px in medium
  weight. Supporting text: 13 px.
- Monospace (IBM Plex Mono or platform fallback): fingerprints, parcel IDs,
  file hashes only.
- Colors: see design.md §3 for full token list.
- 8 px base grid; surfaces 24 px padding; primary button 44 px min height,
  10 px corner radius; task cards 14 px radius.
- Light canvas (`#F3F4EF`), dark rail (`#16292B`).

---

## 13  Handoff ribbon

Spatial, truthful, always-visible on task pages for active transfers.

Four phases, left to right:

1. Prepared
2. Copied to drive
3. Files saved
4. Confirmed

Only stages ≤ the current verified stage appear completed or current. Future
stages are visually quiet. No animated remote progress. No fake numeric progress.
Stage 1 ("Copied to drive") does not imply the destination received anything.

---

## 14  Information disclosure policy

Always visible: recipient identity, file count/size, truthful delivery state,
next physical action, failure recovery instruction.

Available on demand: full safety code / fingerprint (after trust), parcel ID,
ciphertext size, repair group detail, SHA-256 maps, filesystem paths, timestamps,
CLI evidence report path.

The full safety code is always shown during trust verification.

---

## 15  Security invariants (must not change)

- Full fingerprint comparison required; no shortening or silent omission.
- Both devices must pin trust before exchange.
- Only the pinned expected recipient's signed receipt can label a delivery
  Confirmed.
- "Copying to a drive" is never Confirmed or Delivered.
- Core `relay-core` API is unchanged; presentation only.

---

## 16  Acceptance checks

[ ] NOT YET VALIDATED — requires first-time user test.

A nontechnical user should, without author guidance:

1. Explain what the app does after the first screen.
2. Set up a device and add another without using protocol vocabulary.
3. Prepare and place files on a drive; correctly say "not delivered yet".
4. Receive a damaged-but-repairable transfer and know repair is automatic.
5. Put the confirmation on the drive and return it.
6. Distinguish "On its way" from "Delivered" on the sender.
7. Identify the one corrective action for an unrecoverable transfer without
   opening technical details.
