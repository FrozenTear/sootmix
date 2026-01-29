// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Theme system for SootMix.
//!
//! Design Philosophy: "Analog Warmth meets Digital Precision"
//! A professional audio mixer aesthetic that combines the tactile warmth
//! of vintage analog gear with modern digital clarity.
//!
//! The theme is structured for easy customization and future theming support
//! (nirify integration). All colors flow from a semantic palette that can
//! be swapped for different visual identities while maintaining consistency.

use iced::theme::Palette;
use iced::{Border, Color, Theme};

// ============================================================================
// SEMANTIC COLOR PALETTE
// ============================================================================
// Colors are organized semantically rather than by visual appearance.
// This enables coherent theming - swap the palette, keep the meaning.

/// Theme configuration - swap this struct for different themes
pub struct ThemeColors {
    // --- Canvas & Surfaces ---
    /// The deepest background - main app canvas
    pub canvas: Color,
    /// Primary surface for panels and cards
    pub surface: Color,
    /// Elevated surface for hover states and raised elements
    pub surface_raised: Color,
    /// Highest elevation - modals, dropdowns
    pub surface_overlay: Color,

    // --- Text Hierarchy ---
    /// Primary text - high contrast, main content
    pub text_primary: Color,
    /// Secondary text - labels, descriptions
    pub text_secondary: Color,
    /// Tertiary text - hints, disabled states
    pub text_muted: Color,

    // --- Accent Colors ---
    /// Primary accent - interactive elements, focus states
    pub accent_primary: Color,
    /// Secondary accent - complementary highlights
    pub accent_secondary: Color,
    /// Warm accent - for "active" or "engaged" states
    pub accent_warm: Color,

    // --- Semantic States ---
    /// Success/active - green family
    pub semantic_success: Color,
    /// Warning - caution without alarm
    pub semantic_warning: Color,
    /// Error/danger - immediate attention
    pub semantic_error: Color,

    // --- Audio-Specific ---
    /// Meter green zone (safe levels)
    pub meter_safe: Color,
    /// Meter yellow zone (-12dB to -6dB)
    pub meter_caution: Color,
    /// Meter orange zone (-6dB to 0dB)
    pub meter_hot: Color,
    /// Meter red zone (clipping)
    pub meter_clip: Color,
    /// Meter background/inactive
    pub meter_background: Color,

    // --- Borders & Dividers ---
    /// Subtle border for containers
    pub border_subtle: Color,
    /// Standard border for interactive elements
    pub border_default: Color,
    /// Emphasized border for focus/active states
    pub border_emphasis: Color,
}

// ============================================================================
// SOOTMIX DARK THEME - "Midnight Console"
// ============================================================================
// Deep, rich blacks with copper/amber warmth and cyan accents.
// Inspired by high-end studio gear and late-night mixing sessions.

pub const SOOTMIX_DARK: ThemeColors = ThemeColors {
    // Canvas: Deep blue-black, not pure black - adds depth
    canvas: Color::from_rgb(0.075, 0.082, 0.098),
    // Surface: Slightly elevated, warm undertone
    surface: Color::from_rgb(0.118, 0.125, 0.145),
    // Raised: Interactive hover state
    surface_raised: Color::from_rgb(0.165, 0.173, 0.196),
    // Overlay: Modal backgrounds
    surface_overlay: Color::from_rgb(0.098, 0.106, 0.125),

    // Text: Cream-white for warmth, not stark white
    text_primary: Color::from_rgb(0.945, 0.937, 0.918),
    text_secondary: Color::from_rgb(0.678, 0.667, 0.647),
    text_muted: Color::from_rgb(0.467, 0.459, 0.443),

    // Accents: Electric cyan + warm copper
    accent_primary: Color::from_rgb(0.259, 0.820, 0.878),    // Electric cyan
    accent_secondary: Color::from_rgb(0.878, 0.533, 0.259),  // Warm copper
    accent_warm: Color::from_rgb(0.957, 0.718, 0.298),       // Golden amber

    // Semantic: Industry-standard but refined
    semantic_success: Color::from_rgb(0.298, 0.788, 0.463),  // Mint green
    semantic_warning: Color::from_rgb(0.957, 0.757, 0.298),  // Amber
    semantic_error: Color::from_rgb(0.918, 0.337, 0.388),    // Coral red

    // Meters: Classic green-yellow-orange-red gradient
    meter_safe: Color::from_rgb(0.259, 0.757, 0.400),        // Vibrant green
    meter_caution: Color::from_rgb(0.878, 0.773, 0.200),     // Golden yellow
    meter_hot: Color::from_rgb(0.957, 0.549, 0.176),         // Hot orange
    meter_clip: Color::from_rgb(0.918, 0.278, 0.298),        // Danger red
    meter_background: Color::from_rgb(0.098, 0.106, 0.118),  // Near-black

    // Borders: Subtle blue-gray
    border_subtle: Color::from_rgb(0.188, 0.196, 0.224),
    border_default: Color::from_rgb(0.247, 0.255, 0.286),
    border_emphasis: Color::from_rgb(0.318, 0.329, 0.365),
};

// ============================================================================
// ACTIVE THEME SELECTION
// ============================================================================
// Change this to switch themes globally

const ACTIVE_THEME: &ThemeColors = &SOOTMIX_DARK;

// ============================================================================
// PUBLIC COLOR CONSTANTS (backwards compatible)
// ============================================================================
// These map semantic colors to the legacy names used throughout the codebase.

/// Main background color.
pub const BACKGROUND: Color = SOOTMIX_DARK.canvas;

/// Surface color for cards and panels.
pub const SURFACE: Color = SOOTMIX_DARK.surface;

/// Lighter surface for hover states.
pub const SURFACE_LIGHT: Color = SOOTMIX_DARK.surface_raised;

/// Primary accent color.
pub const PRIMARY: Color = SOOTMIX_DARK.accent_primary;

/// Secondary accent color.
pub const ACCENT: Color = SOOTMIX_DARK.accent_secondary;

/// Main text color.
pub const TEXT: Color = SOOTMIX_DARK.text_primary;

/// Dimmed text color.
pub const TEXT_DIM: Color = SOOTMIX_DARK.text_secondary;

/// Muted/error indicator (red).
pub const MUTED_COLOR: Color = SOOTMIX_DARK.semantic_error;

/// Success/active indicator (green).
pub const SUCCESS: Color = SOOTMIX_DARK.semantic_success;

/// Warning indicator.
pub const WARNING: Color = SOOTMIX_DARK.semantic_warning;

/// Slider track background.
pub const SLIDER_TRACK: Color = SOOTMIX_DARK.border_default;

/// Border color for UI elements.
pub const BORDER_COLOR: Color = SOOTMIX_DARK.border_default;

// ============================================================================
// Theme Palette (Iced integration)
// ============================================================================

/// Create the SootMix theme palette.
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
// SPACING & LAYOUT SYSTEM
// ============================================================================
// Based on an 4px grid for precise alignment

/// Base unit (4px)
pub const UNIT: f32 = 4.0;

/// Extra-small spacing (4px)
pub const SPACING_XS: f32 = UNIT;

/// Small spacing (8px)
pub const SPACING_SM: f32 = UNIT * 2.0;

/// Standard spacing (12px)
pub const SPACING: f32 = UNIT * 3.0;

/// Medium spacing (16px)
pub const SPACING_MD: f32 = UNIT * 4.0;

/// Large spacing (24px)
pub const SPACING_LG: f32 = UNIT * 6.0;

/// Extra-large spacing (32px)
pub const SPACING_XL: f32 = UNIT * 8.0;

// Legacy aliases
pub const SPACING_SMALL: f32 = SPACING_SM;
pub const SPACING_LARGE: f32 = SPACING_LG;

/// Standard padding for containers
pub const PADDING: f32 = SPACING_MD;

/// Compact padding for buttons/small elements
pub const PADDING_COMPACT: f32 = SPACING_SM;

// ============================================================================
// BORDER RADIUS SYSTEM
// ============================================================================

/// Small radius - buttons, inputs (4px)
pub const RADIUS_SM: f32 = 4.0;

/// Standard radius - cards, panels (8px)
pub const RADIUS: f32 = 8.0;

/// Large radius - modals, prominent containers (12px)
pub const RADIUS_LG: f32 = 12.0;

/// Full/pill radius for badges, tags
pub const RADIUS_FULL: f32 = 9999.0;

// Legacy aliases
pub const BORDER_RADIUS_SMALL: f32 = RADIUS_SM;
pub const BORDER_RADIUS: f32 = RADIUS;
pub const BORDER_RADIUS_LARGE: f32 = RADIUS_LG;

// ============================================================================
// MIXER-SPECIFIC DIMENSIONS
// ============================================================================

/// Channel strip width - optimized for readability
pub const CHANNEL_STRIP_WIDTH: f32 = 128.0;

/// Compact channel strip width (for many channels)
pub const CHANNEL_STRIP_WIDTH_COMPACT: f32 = 96.0;

/// Channel strip height (sized to fit input channels with device picker + sidetone controls)
pub const CHANNEL_STRIP_HEIGHT: f32 = 480.0;

/// Volume slider height (vertical fader)
pub const VOLUME_SLIDER_HEIGHT: f32 = 200.0;

/// Meter bar width (individual channel)
pub const METER_BAR_WIDTH: f32 = 5.0;

/// Meter bar gap (stereo separation)
pub const METER_BAR_GAP: f32 = 2.0;

// ============================================================================
// TYPOGRAPHY
// ============================================================================

/// Title text size (app name, section headers)
pub const TEXT_TITLE: f32 = 22.0;

/// Heading text size (panel titles)
pub const TEXT_HEADING: f32 = 16.0;

/// Body text size (labels, descriptions)
pub const TEXT_BODY: f32 = 13.0;

/// Small text size (secondary info)
pub const TEXT_SMALL: f32 = 11.0;

/// Caption text size (hints, metadata)
pub const TEXT_CAPTION: f32 = 10.0;

// ============================================================================
// ANIMATION DURATIONS (for future use)
// ============================================================================

/// Fast transition (hover states)
pub const DURATION_FAST: u64 = 100;

/// Normal transition (panel open/close)
pub const DURATION_NORMAL: u64 = 200;

/// Slow transition (page transitions)
pub const DURATION_SLOW: u64 = 300;

// ============================================================================
// STYLE HELPERS
// ============================================================================

/// Create a standard border with default styling.
pub fn standard_border() -> Border {
    Border::default()
        .rounded(RADIUS)
        .color(ACTIVE_THEME.border_default)
        .width(1.0)
}

/// Create an emphasized border for focus/active states.
pub fn emphasis_border() -> Border {
    Border::default()
        .rounded(RADIUS)
        .color(ACTIVE_THEME.accent_primary)
        .width(2.0)
}

/// Create a subtle border for containers.
pub fn subtle_border() -> Border {
    Border::default()
        .rounded(RADIUS)
        .color(ACTIVE_THEME.border_subtle)
        .width(1.0)
}

// ============================================================================
// AUDIO VISUALIZATION HELPERS
// ============================================================================

/// Convert dB level to a color for volume meters.
/// Follows professional audio conventions:
/// - Green: Safe levels (below -12dB)
/// - Yellow: Caution (-12dB to -6dB)
/// - Orange: Hot (-6dB to 0dB)
/// - Red: Clipping (above 0dB)
pub fn db_to_color(db: f32) -> Color {
    if db > 0.0 {
        // Clipping - danger!
        ACTIVE_THEME.meter_clip
    } else if db > -6.0 {
        // Hot zone - blend orange to red
        let t = (db + 6.0) / 6.0;
        blend_colors(ACTIVE_THEME.meter_hot, ACTIVE_THEME.meter_clip, t)
    } else if db > -12.0 {
        // Caution zone - blend yellow to orange
        let t = (db + 12.0) / 6.0;
        blend_colors(ACTIVE_THEME.meter_caution, ACTIVE_THEME.meter_hot, t)
    } else if db > -24.0 {
        // Safe but visible - blend green to yellow
        let t = (db + 24.0) / 12.0;
        blend_colors(ACTIVE_THEME.meter_safe, ACTIVE_THEME.meter_caution, t)
    } else {
        // Low level - solid green
        ACTIVE_THEME.meter_safe
    }
}

/// Blend two colors by a factor t (0.0 = a, 1.0 = b).
fn blend_colors(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color::from_rgb(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
    )
}

/// Format volume in dB for display.
pub fn format_db(db: f32) -> String {
    if db <= -60.0 {
        "-\u{221E}".to_string() // -infinity symbol
    } else if db >= 0.0 {
        format!("+{:.1}", db)
    } else {
        format!("{:.1}", db)
    }
}

/// Get a color for the slider fill based on level and mute state.
pub fn slider_fill_color(db: f32, muted: bool) -> Color {
    if muted {
        ACTIVE_THEME.text_muted
    } else {
        db_to_color(db)
    }
}

// ============================================================================
// METER COLORS (for direct use in meter rendering)
// ============================================================================

/// Meter green zone color
pub const METER_GREEN: Color = SOOTMIX_DARK.meter_safe;

/// Meter yellow zone color
pub const METER_YELLOW: Color = SOOTMIX_DARK.meter_caution;

/// Meter orange zone color
pub const METER_ORANGE: Color = SOOTMIX_DARK.meter_hot;

/// Meter red zone color
pub const METER_RED: Color = SOOTMIX_DARK.meter_clip;

/// Meter background color
pub const METER_BACKGROUND: Color = SOOTMIX_DARK.meter_background;

// ============================================================================
// WIDGET STYLE PRESETS
// ============================================================================

/// Button style variants
pub mod button_style {
    use super::*;
    use iced::widget::button;
    use iced::{Background, Theme};

    /// Primary action button style
    pub fn primary(_theme: &Theme, status: button::Status) -> button::Style {
        let is_active = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if is_active {
                lighten(ACTIVE_THEME.accent_primary, 0.1)
            } else {
                ACTIVE_THEME.accent_primary
            })),
            text_color: ACTIVE_THEME.canvas,
            border: Border::default().rounded(RADIUS_SM),
            ..button::Style::default()
        }
    }

    /// Secondary/ghost button style
    pub fn secondary(_theme: &Theme, status: button::Status) -> button::Style {
        let is_active = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if is_active {
                ACTIVE_THEME.surface_raised
            } else {
                ACTIVE_THEME.surface
            })),
            text_color: ACTIVE_THEME.text_primary,
            border: Border::default()
                .rounded(RADIUS_SM)
                .color(ACTIVE_THEME.border_default)
                .width(1.0),
            ..button::Style::default()
        }
    }

    /// Danger/destructive action button style
    pub fn danger(_theme: &Theme, status: button::Status) -> button::Style {
        let is_active = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if is_active {
                lighten(ACTIVE_THEME.semantic_error, 0.1)
            } else {
                ACTIVE_THEME.semantic_error
            })),
            text_color: ACTIVE_THEME.text_primary,
            border: Border::default().rounded(RADIUS_SM),
            ..button::Style::default()
        }
    }

    /// Ghost/transparent button style
    pub fn ghost(_theme: &Theme, status: button::Status) -> button::Style {
        let is_active = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if is_active {
                ACTIVE_THEME.surface_raised
            } else {
                Color::TRANSPARENT
            })),
            text_color: ACTIVE_THEME.text_secondary,
            border: Border::default().rounded(RADIUS_SM),
            ..button::Style::default()
        }
    }
}

/// Container style variants
pub mod container_style {
    use super::*;
    use iced::widget::container;
    use iced::{Background, Theme};

    /// Panel container (cards, sidebars)
    pub fn panel(_theme: &Theme) -> container::Style {
        container::Style {
            background: Some(Background::Color(ACTIVE_THEME.surface)),
            border: Border::default()
                .rounded(RADIUS)
                .color(ACTIVE_THEME.border_subtle)
                .width(1.0),
            ..container::Style::default()
        }
    }

    /// Elevated panel (modals, dropdowns)
    pub fn elevated(_theme: &Theme) -> container::Style {
        container::Style {
            background: Some(Background::Color(ACTIVE_THEME.surface_overlay)),
            border: Border::default()
                .rounded(RADIUS_LG)
                .color(ACTIVE_THEME.accent_primary)
                .width(1.0),
            shadow: iced::Shadow {
                color: Color { a: 0.4, ..Color::BLACK },
                offset: iced::Vector::new(0.0, 8.0),
                blur_radius: 24.0,
            },
            ..container::Style::default()
        }
    }

    /// Channel strip container
    pub fn channel_strip(_theme: &Theme, is_active: bool) -> container::Style {
        container::Style {
            background: Some(Background::Color(ACTIVE_THEME.surface)),
            border: Border::default()
                .rounded(RADIUS)
                .color(if is_active {
                    ACTIVE_THEME.accent_primary
                } else {
                    ACTIVE_THEME.border_subtle
                })
                .width(if is_active { 2.0 } else { 1.0 }),
            ..container::Style::default()
        }
    }

    /// Master strip container (emphasized)
    pub fn master_strip(_theme: &Theme) -> container::Style {
        container::Style {
            background: Some(Background::Color(ACTIVE_THEME.surface)),
            border: Border::default()
                .rounded(RADIUS)
                .color(ACTIVE_THEME.accent_warm)
                .width(2.0),
            ..container::Style::default()
        }
    }
}

/// Lighten a color by a factor (0.0-1.0)
fn lighten(color: Color, factor: f32) -> Color {
    Color::from_rgb(
        (color.r + (1.0 - color.r) * factor).min(1.0),
        (color.g + (1.0 - color.g) * factor).min(1.0),
        (color.b + (1.0 - color.b) * factor).min(1.0),
    )
}

// ============================================================================
// THEME PRESETS FOR FUTURE USE
// ============================================================================
// Additional themes can be defined here and selected at runtime

/// Light theme variant (for future implementation)
#[allow(dead_code)]
pub const SOOTMIX_LIGHT: ThemeColors = ThemeColors {
    canvas: Color::from_rgb(0.965, 0.961, 0.953),
    surface: Color::from_rgb(1.0, 1.0, 1.0),
    surface_raised: Color::from_rgb(0.976, 0.973, 0.969),
    surface_overlay: Color::from_rgb(1.0, 1.0, 1.0),

    text_primary: Color::from_rgb(0.118, 0.125, 0.145),
    text_secondary: Color::from_rgb(0.400, 0.408, 0.431),
    text_muted: Color::from_rgb(0.600, 0.608, 0.627),

    accent_primary: Color::from_rgb(0.157, 0.624, 0.698),
    accent_secondary: Color::from_rgb(0.788, 0.443, 0.169),
    accent_warm: Color::from_rgb(0.878, 0.627, 0.208),

    semantic_success: Color::from_rgb(0.208, 0.698, 0.373),
    semantic_warning: Color::from_rgb(0.878, 0.667, 0.208),
    semantic_error: Color::from_rgb(0.839, 0.247, 0.298),

    meter_safe: Color::from_rgb(0.208, 0.678, 0.341),
    meter_caution: Color::from_rgb(0.808, 0.694, 0.141),
    meter_hot: Color::from_rgb(0.878, 0.478, 0.118),
    meter_clip: Color::from_rgb(0.839, 0.208, 0.239),
    meter_background: Color::from_rgb(0.910, 0.906, 0.898),

    border_subtle: Color::from_rgb(0.910, 0.906, 0.898),
    border_default: Color::from_rgb(0.847, 0.843, 0.835),
    border_emphasis: Color::from_rgb(0.757, 0.753, 0.745),
};
