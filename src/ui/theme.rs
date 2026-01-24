// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Theme constants and styling for SootMix.

use iced::color;
use iced::theme::Palette;
use iced::{Border, Color, Theme};

// ============================================================================
// Color Constants (Dark Theme)
// ============================================================================

/// Main background color.
pub const BACKGROUND: Color = Color::from_rgb(0.12, 0.12, 0.14);

/// Surface color for cards and panels.
pub const SURFACE: Color = Color::from_rgb(0.18, 0.18, 0.20);

/// Lighter surface for hover states.
pub const SURFACE_LIGHT: Color = Color::from_rgb(0.24, 0.24, 0.26);

/// Primary accent color (blue).
pub const PRIMARY: Color = Color::from_rgb(0.40, 0.65, 0.95);

/// Secondary accent color (orange).
pub const ACCENT: Color = Color::from_rgb(0.95, 0.60, 0.20);

/// Main text color.
pub const TEXT: Color = Color::from_rgb(0.90, 0.90, 0.92);

/// Dimmed text color.
pub const TEXT_DIM: Color = Color::from_rgb(0.60, 0.60, 0.65);

/// Muted/error indicator (red).
pub const MUTED_COLOR: Color = Color::from_rgb(0.85, 0.30, 0.30);

/// Success/active indicator (green).
pub const SUCCESS: Color = Color::from_rgb(0.40, 0.75, 0.40);

/// Warning indicator (yellow).
pub const WARNING: Color = Color::from_rgb(0.90, 0.75, 0.20);

/// Slider track background.
pub const SLIDER_TRACK: Color = Color::from_rgb(0.30, 0.30, 0.32);

/// Slider fill color (green gradient from dark to light based on level).
pub const SLIDER_FILL: Color = Color::from_rgb(0.40, 0.75, 0.40);

/// Slider fill when clipping (red).
pub const SLIDER_CLIP: Color = Color::from_rgb(0.90, 0.30, 0.30);

// ============================================================================
// Theme Palette
// ============================================================================

/// Create the SootMix dark theme palette.
pub const THEME_PALETTE: Palette = Palette {
    background: BACKGROUND,
    text: TEXT,
    primary: PRIMARY,
    success: SUCCESS,
    danger: MUTED_COLOR,
    warning: WARNING,
};

/// Get the SootMix custom theme.
pub fn sootmix_theme() -> Theme {
    Theme::custom("SootMix Dark".to_string(), THEME_PALETTE)
}

// ============================================================================
// Style Helpers
// ============================================================================

/// Standard border radius for UI elements.
pub const BORDER_RADIUS: f32 = 6.0;

/// Small border radius.
pub const BORDER_RADIUS_SMALL: f32 = 4.0;

/// Large border radius.
pub const BORDER_RADIUS_LARGE: f32 = 10.0;

/// Standard spacing between elements.
pub const SPACING: f32 = 10.0;

/// Small spacing.
pub const SPACING_SMALL: f32 = 5.0;

/// Large spacing.
pub const SPACING_LARGE: f32 = 20.0;

/// Standard padding.
pub const PADDING: f32 = 15.0;

/// Channel strip width.
pub const CHANNEL_STRIP_WIDTH: f32 = 120.0;

/// Channel strip height.
pub const CHANNEL_STRIP_HEIGHT: f32 = 400.0;

/// Volume slider height.
pub const VOLUME_SLIDER_HEIGHT: f32 = 200.0;

/// Create a standard border.
pub fn standard_border() -> Border {
    Border::default()
        .rounded(BORDER_RADIUS)
        .color(SURFACE_LIGHT)
        .width(1.0)
}

/// Convert dB to a color for the volume meter.
pub fn db_to_color(db: f32) -> Color {
    if db > 0.0 {
        // Clipping range - red
        SLIDER_CLIP
    } else if db > -6.0 {
        // High level - yellow to green
        let t = (db + 6.0) / 6.0;
        Color::from_rgb(
            WARNING.r * t + SUCCESS.r * (1.0 - t),
            WARNING.g * t + SUCCESS.g * (1.0 - t),
            WARNING.b * t + SUCCESS.b * (1.0 - t),
        )
    } else {
        // Normal level - green
        SUCCESS
    }
}

/// Format volume in dB for display.
pub fn format_db(db: f32) -> String {
    if db <= -60.0 {
        "-inf".to_string()
    } else if db >= 0.0 {
        format!("+{:.1}", db)
    } else {
        format!("{:.1}", db)
    }
}
