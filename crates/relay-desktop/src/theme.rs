// theme.rs — RelayPost novice-UX visual foundation.
//
// Provides public Color32 constants and a `configure` function that registers
// the bundled Atkinson Hyperlegible Next fonts and sets the egui style to the
// approved design-system values.
//
// Owner: Kiro (novice-ux redesign, 2026-11-03).
// Compilation and validation are pending GitHub Actions; no local Rust run.

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, TextStyle};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Color tokens
// ---------------------------------------------------------------------------

/// Primary text — near-black with a warm blue-green undertone.
pub const INK: Color32 = Color32::from_rgb(0x17, 0x23, 0x26);

/// Supporting text — medium-contrast muted blue-grey.
pub const MUTED: Color32 = Color32::from_rgb(0x66, 0x73, 0x77);

/// Main background canvas — warm off-white.
pub const PAPER: Color32 = Color32::from_rgb(0xF3, 0xF4, 0xEF);

/// Quiet border — pale cool-grey.
pub const LINE: Color32 = Color32::from_rgb(0xDD, 0xE2, 0xDF);

/// Primary action / current verified stage — deep teal.
pub const TEAL: Color32 = Color32::from_rgb(0x08, 0x7F, 0x72);

/// Verified success / confirmed — forest green.
pub const GREEN: Color32 = Color32::from_rgb(0x1A, 0x6B, 0x3A);

/// Waiting / action needed — warm amber.
pub const AMBER: Color32 = Color32::from_rgb(0xA7, 0x6A, 0x18);

/// Unrecoverable / destructive actions — deep red.
pub const RED: Color32 = Color32::from_rgb(0xB5, 0x45, 0x39);

/// Left navigation rail background — very dark teal-black.
pub const RAIL: Color32 = Color32::from_rgb(0x16, 0x29, 0x2B);

// ---------------------------------------------------------------------------
// Font name constant
// ---------------------------------------------------------------------------

/// The name of the display/medium font family used for headings and primary
/// buttons. Use as `FontFamily::Name(DISPLAY_FAMILY.into())`.
pub const DISPLAY_FAMILY: &str = "RelayDisplay";

// ---------------------------------------------------------------------------
// Font bytes — compiled into the binary at build time.
// ---------------------------------------------------------------------------

static REGULAR_BYTES: &[u8] =
    include_bytes!("../assets/fonts/AtkinsonHyperlegibleNext-Regular.ttf");

static MEDIUM_BYTES: &[u8] = include_bytes!("../assets/fonts/AtkinsonHyperlegibleNext-Medium.ttf");

// ---------------------------------------------------------------------------
// Public configuration entry point
// ---------------------------------------------------------------------------

/// Configure the egui context with the RelayPost design system.
///
/// Call once from `RelayApp::new` (via `eframe::CreationContext`) before the
/// first frame. Calling it again replaces the previous configuration.
///
/// Effects:
/// - Sets light visuals with INK override text colour and teal selection.
/// - Registers AtkinsonHyperlegibleNext-Regular as the leading proportional
///   font (emoji/monospace egui defaults remain as fallback).
/// - Registers AtkinsonHyperlegibleNext-Medium as `FontFamily::Name("RelayDisplay")`.
/// - Sets text styles: Body 16 px, Button 15 px (display), Heading 30 px (display),
///   Small 13 px; Monospace unchanged.
/// - Sets interact_size.y = 44, 8-px-based spacing and padding.
///
/// No new crates are required; all APIs are from eframe/egui 0.31.1.
pub fn configure(context: &egui::Context) {
    // --- Visuals -----------------------------------------------------------
    // Start from the egui light preset, then override specific fields.
    // Matches the pattern used by the existing configure_style in app.rs.
    context.set_visuals(egui::Visuals::light());

    let mut style = (*context.style()).clone();
    style.visuals.override_text_color = Some(INK);
    style.visuals.selection.bg_fill = Color32::from_rgb(0xD0, 0xE8, 0xE0);
    style.visuals.selection.stroke = egui::Stroke::new(1.0_f32, TEAL);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(0xD0, 0xE8, 0xE0);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(0xE5, 0xF0, 0xE9);
    style.visuals.panel_fill = PAPER;
    style.visuals.window_fill = Color32::WHITE;
    style.visuals.window_stroke = egui::Stroke::new(1.0_f32, LINE);
    for widget in [
        &mut style.visuals.widgets.noninteractive,
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::same(10);
    }

    // --- Fonts ------------------------------------------------------------
    let mut fonts = FontDefinitions::default();

    // Register Atkinson Regular — prepended to Proportional below.
    fonts.font_data.insert(
        "AtkinsonRegular".to_owned(),
        Arc::new(FontData::from_static(REGULAR_BYTES)),
    );

    // Register Atkinson Medium — used for the display family.
    fonts.font_data.insert(
        "AtkinsonMedium".to_owned(),
        Arc::new(FontData::from_static(MEDIUM_BYTES)),
    );

    // Prepend Regular to the Proportional family. Existing entries (NotoEmoji,
    // etc.) remain as fallback for full Unicode coverage.
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "AtkinsonRegular".to_owned());

    // Build the display family: Medium first, then Regular, then the rest of
    // the Proportional stack as further fallback.
    let mut display_stack = vec!["AtkinsonMedium".to_owned(), "AtkinsonRegular".to_owned()];
    if let Some(prop) = fonts.families.get(&FontFamily::Proportional).cloned() {
        for name in prop {
            if name != "AtkinsonMedium" && name != "AtkinsonRegular" {
                display_stack.push(name);
            }
        }
    }
    fonts
        .families
        .insert(FontFamily::Name(DISPLAY_FAMILY.into()), display_stack);

    context.set_fonts(fonts);

    // --- Style ------------------------------------------------------------
    // Text styles.
    let display = FontFamily::Name(DISPLAY_FAMILY.into());
    style
        .text_styles
        .insert(TextStyle::Body, FontId::new(16.0, FontFamily::Proportional));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::new(15.0, display.clone()));
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::new(30.0, display));
    style.text_styles.insert(
        TextStyle::Small,
        FontId::new(13.0, FontFamily::Proportional),
    );
    // Monospace is left at the egui default; used only for fingerprints/hashes.

    // Spacing — 8 px base grid, generous touch targets.
    style.spacing.item_spacing = egui::vec2(10.0, 9.0);
    style.spacing.button_padding = egui::vec2(16.0, 10.0);
    style.spacing.interact_size.y = 44.0;
    style.spacing.indent = 16.0;
    style.spacing.window_margin = egui::Margin::same(16_i8);

    context.set_style(style);
}
