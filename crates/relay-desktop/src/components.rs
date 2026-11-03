// components.rs — RelayPost novice-UX reusable UI primitives.
//
// All functions are purely presentational: they accept `&mut Ui` and data,
// render to the egui frame, and return at most an `egui::Response`. They never
// read or write `RelayApp` state, never call relay-core, and have no side
// effects beyond painting.
//
// Owner: Kiro (novice-ux redesign, 2026-11-03).
// Compilation and validation are pending GitHub Actions; no local Rust run.

use crate::theme::{DISPLAY_FAMILY, INK, LINE, MUTED, PAPER, TEAL};
use eframe::egui::{self, Color32, FontFamily, RichText, Stroke, Ui, Vec2};

// ---------------------------------------------------------------------------
// Page title
// ---------------------------------------------------------------------------

/// Renders a large page heading with a supporting subtitle, followed by a
/// 24 px gap before the first content item.
///
/// `title` uses the Heading text style (30 px display family, set by
/// `theme::configure`).
/// `subtitle` uses the Body text style in MUTED colour.
pub fn page_title(ui: &mut Ui, title: &str, subtitle: &str) {
    ui.heading(title);
    ui.label(RichText::new(subtitle).color(MUTED));
    ui.add_space(24.0);
}

// ---------------------------------------------------------------------------
// Section label
// ---------------------------------------------------------------------------

/// Renders an all-caps section label in a smaller MUTED style, followed by a
/// 4 px gap before the section's contents.
pub fn section_label(ui: &mut Ui, label: &str) {
    ui.label(RichText::new(label).size(11.0).strong().color(MUTED));
    ui.add_space(4.0);
}

// ---------------------------------------------------------------------------
// Surface
// ---------------------------------------------------------------------------

/// Renders a white rounded panel with a quiet 1 px border, 24 px inner margin,
/// and width bounded to the available UI width.
///
/// Use `surface` for every major task card, identity panel, or status block.
pub fn surface(ui: &mut Ui, contents: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(Color32::WHITE)
        .stroke(Stroke::new(1.0_f32, LINE))
        .corner_radius(14_u8)
        .inner_margin(egui::Margin::same(24_i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            contents(ui);
        });
}

// ---------------------------------------------------------------------------
// Primary button
// ---------------------------------------------------------------------------

/// Renders the primary action button: TEAL fill, white medium text, 44 px min
/// height.
///
/// Corner rounding is inherited from the widget style set by `theme::configure`.
/// Pass `enabled = false` to disable; the button still renders but ignores
/// clicks.
///
/// Returns the `egui::Response` so callers can check `.clicked()`.
pub fn primary(ui: &mut Ui, text: &str, enabled: bool) -> egui::Response {
    let fill = if enabled {
        TEAL
    } else {
        Color32::from_rgb(0xB0, 0xBD, 0xBA)
    };

    let label = RichText::new(text)
        .family(FontFamily::Name(DISPLAY_FAMILY.into()))
        .size(15.0)
        .color(Color32::WHITE);

    let button = egui::Button::new(label)
        .wrap()
        .fill(fill)
        .corner_radius(10)
        .min_size(Vec2::new(0.0, 44.0));

    ui.add_enabled(enabled, button)
}

// ---------------------------------------------------------------------------
// Empty state
// ---------------------------------------------------------------------------

/// Renders an empty-state placeholder inside a `surface`: bold title at 19 px,
/// supporting body in MUTED, 12 px vertical padding above and below.
pub fn empty_state(ui: &mut Ui, title: &str, body: &str) {
    surface(ui, |ui| {
        ui.add_space(12.0);
        ui.label(RichText::new(title).size(19.0).strong());
        ui.label(RichText::new(body).color(MUTED));
        ui.add_space(12.0);
    });
}

// ---------------------------------------------------------------------------
// Handoff ribbon
// ---------------------------------------------------------------------------

/// Labels for the four phases of a RelayPost offline handoff.
const PHASE_LABELS: [&str; 4] = ["Prepared", "Copied to drive", "Files saved", "Confirmed"];

/// Renders the handoff ribbon: four phases connected by horizontal lines.
///
/// `stage` is 0-based:
/// - Phases with index < `stage` are **completed** — solid TEAL circle with a
///   simple checkmark.
/// - The phase with index == `stage` is **current**, unless it is the final,
///   verified confirmation. That terminal phase is complete too.
/// - Phases with index > `stage` are **future** — hollow LINE circle, MUTED
///   label.
///
/// Circles and connector lines are drawn with `Painter` primitives so that
/// rendering stays crisp at any DPI.  Phase labels are rendered as normal egui
/// labels so the accessibility tree remains intact.
///
/// **Truthfulness contract:**
/// - No animation, no numeric fake progress.
/// - Stage 1 ("Copied to drive") does not claim the destination received
///   anything.
/// - Stage 3 ("Confirmed") is only reached when the caller passes `stage = 3`,
///   which must be driven by a verified `DeliveryState::Confirmed` fact.
pub fn handoff_ribbon(ui: &mut Ui, stage: usize) {
    let stage = stage.min(PHASE_LABELS.len().saturating_sub(1));
    let n = PHASE_LABELS.len();
    let complete = stage == n - 1;

    // Distribute phases evenly across the available width.
    let available_width = ui.available_width().max(0.0);
    let phase_width = available_width / n as f32;

    // Layout measurements.
    let circle_r = 9.0_f32;
    let circle_row_height = 36.0_f32;
    let label_row_height = 42.0_f32;
    let total_height = circle_row_height + label_row_height + 4.0;

    // Allocate a single rectangle for the whole ribbon so the caller's layout
    // advances past it in one step.
    let (ribbon_rect, _) = ui.allocate_exact_size(
        Vec2::new(available_width, total_height),
        egui::Sense::hover(),
    );

    let painter = ui.painter();
    let circle_y = ribbon_rect.top() + circle_row_height / 2.0;

    // ── Connector lines (drawn first, behind circles) ─────────────────────
    for i in 0..(n - 1) {
        let left_cx = ribbon_rect.left() + phase_width * (i as f32 + 0.5);
        let right_cx = ribbon_rect.left() + phase_width * (i as f32 + 1.5);
        // A segment is teal when the left phase is complete (i < stage).
        let color = if i < stage { TEAL } else { LINE };
        painter.line_segment(
            [
                egui::pos2(left_cx + circle_r + 1.0, circle_y),
                egui::pos2(right_cx - circle_r - 1.0, circle_y),
            ],
            Stroke::new(2.0_f32, color),
        );
    }

    // ── Circles ───────────────────────────────────────────────────────────
    for i in 0..n {
        let cx = ribbon_rect.left() + phase_width * (i as f32 + 0.5);
        let center = egui::pos2(cx, circle_y);

        if i < stage || complete {
            // Completed — solid TEAL with a white checkmark.
            painter.circle_filled(center, circle_r, TEAL);
            let arm = circle_r * 0.33;
            // Left arm of check (down-right).
            painter.line_segment(
                [
                    egui::pos2(cx - arm, circle_y),
                    egui::pos2(cx - arm * 0.1, circle_y + arm * 0.9),
                ],
                Stroke::new(2.0_f32, Color32::WHITE),
            );
            // Right arm of check (up-right).
            painter.line_segment(
                [
                    egui::pos2(cx - arm * 0.1, circle_y + arm * 0.9),
                    egui::pos2(cx + arm, circle_y - arm * 0.6),
                ],
                Stroke::new(2.0_f32, Color32::WHITE),
            );
        } else if i == stage {
            // Current — solid TEAL with a faint outer ring.
            painter.circle_filled(center, circle_r, TEAL);
            painter.circle_stroke(
                center,
                circle_r + 3.0,
                Stroke::new(
                    1.5_f32,
                    Color32::from_rgba_unmultiplied(0x08, 0x7F, 0x72, 80),
                ),
            );
        } else {
            // Future — hollow circle.
            painter.circle_filled(center, circle_r, PAPER);
            painter.circle_stroke(center, circle_r, Stroke::new(1.5_f32, LINE));
        }
    }

    // ── Phase labels (normal egui labels for accessibility) ───────────────
    for (i, label) in PHASE_LABELS.iter().enumerate() {
        let is_current = i == stage && !complete;
        let is_future = i > stage;

        let text_color = if is_future {
            MUTED
        } else if is_current {
            INK
        } else {
            TEAL
        };
        let text_size = if is_current { 13.5_f32 } else { 12.5_f32 };

        let col_rect = egui::Rect::from_min_size(
            egui::pos2(
                ribbon_rect.left() + phase_width * i as f32,
                ribbon_rect.top() + circle_row_height,
            ),
            Vec2::new(phase_width, label_row_height + 4.0),
        );

        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(col_rect)
                .layout(egui::Layout::top_down(egui::Align::Center)),
        );

        let rich = RichText::new(*label).size(text_size).color(text_color);
        let rich = if is_current { rich.strong() } else { rich };
        child.label(rich);
        child.label(
            RichText::new(if complete && i == stage {
                "Complete"
            } else if i < stage {
                "Done"
            } else if is_current {
                "Current step"
            } else {
                "Not yet"
            })
            .size(12.0)
            .color(text_color),
        );
    }

    ui.add_space(6.0);
}
