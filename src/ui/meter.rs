// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! VU meter widget for audio level display.
//!
//! A professional-grade meter visualization with:
//! - Stereo bar display with precise level indication
//! - Four-zone gradient coloring (green/yellow/orange/red)
//! - Peak hold indicators with color-coded warnings
//! - Smooth visual transitions

#![allow(dead_code)]

use crate::message::Message;
use crate::state::MeterDisplayState;
use crate::ui::theme::{METER_BACKGROUND, METER_GREEN, METER_ORANGE, METER_RED, METER_YELLOW};
use iced::widget::canvas::{self, Frame, Geometry, Path};
use iced::{mouse, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

// ============================================================================
// METER DIMENSIONS
// ============================================================================

/// Width of a single meter bar in pixels.
const BAR_WIDTH: f32 = 5.0;

/// Gap between stereo bars.
const BAR_GAP: f32 = 2.0;

/// Total width of stereo meter (2 bars + gap).
pub const METER_WIDTH: f32 = BAR_WIDTH * 2.0 + BAR_GAP;

/// Peak hold indicator height.
const PEAK_INDICATOR_HEIGHT: f32 = 2.0;

/// Segment gap for the "segmented LED" look (0 = solid, >0 = segmented).
const SEGMENT_GAP: f32 = 1.0;

/// Segment height (including gap).
const SEGMENT_HEIGHT: f32 = 4.0;

// ============================================================================
// LEVEL THRESHOLDS
// ============================================================================
// Thresholds in linear scale (0.0 to 1.0+).
// These correspond to dB levels: -12dB, -6dB, 0dB

/// Yellow threshold (-12dB in linear).
const YELLOW_THRESHOLD: f32 = 0.25;

/// Orange threshold (-6dB in linear).
const ORANGE_THRESHOLD: f32 = 0.5;

/// Red/clipping threshold (0dB in linear).
const RED_THRESHOLD: f32 = 1.0;

/// Peak indicator base color.
const PEAK_INDICATOR_COLOR: Color = Color::from_rgb(1.0, 1.0, 1.0);

// ============================================================================
// VU METER STATE
// ============================================================================

/// Height of the clip indicator at the top of the meter.
const CLIP_INDICATOR_HEIGHT: f32 = 4.0;

/// VU meter state for canvas rendering.
#[derive(Debug, Clone)]
pub struct VuMeter {
    /// Left channel level (0.0 to 1.0+).
    pub level_left: f32,
    /// Right channel level (0.0 to 1.0+).
    pub level_right: f32,
    /// Left channel peak hold (0.0 to 1.0+).
    pub peak_left: f32,
    /// Right channel peak hold (0.0 to 1.0+).
    pub peak_right: f32,
    /// Left channel has clipped.
    pub clipped_left: bool,
    /// Right channel has clipped.
    pub clipped_right: bool,
}

impl VuMeter {
    /// Create a new VU meter from display state.
    pub fn from_state(state: &MeterDisplayState) -> Self {
        Self {
            level_left: state.level_left,
            level_right: state.level_right,
            peak_left: state.peak_hold_left,
            peak_right: state.peak_hold_right,
            clipped_left: state.clipped_left,
            clipped_right: state.clipped_right,
        }
    }

    /// Create an empty (silent) meter.
    pub fn silent() -> Self {
        Self {
            level_left: 0.0,
            level_right: 0.0,
            peak_left: 0.0,
            peak_right: 0.0,
            clipped_left: false,
            clipped_right: false,
        }
    }
}

impl Default for VuMeter {
    fn default() -> Self {
        Self::silent()
    }
}

// ============================================================================
// CANVAS RENDERING
// ============================================================================

impl<Message> canvas::Program<Message> for VuMeter {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let height = bounds.height;

        // Draw background for both bars with rounded corners effect
        let bg_path = Path::rectangle(Point::ORIGIN, Size::new(METER_WIDTH, height));
        frame.fill(&bg_path, METER_BACKGROUND);

        // Reserve space for clip indicator at top
        let meter_height = height - CLIP_INDICATOR_HEIGHT - 2.0; // 2px gap

        // Draw clip indicators at top
        self.draw_clip_indicator(&mut frame, 0.0, self.clipped_left);
        self.draw_clip_indicator(&mut frame, BAR_WIDTH + BAR_GAP, self.clipped_right);

        // Draw left bar (offset by clip indicator height)
        self.draw_bar(
            &mut frame,
            0.0,
            CLIP_INDICATOR_HEIGHT + 2.0, // Start Y
            meter_height,
            self.level_left,
            self.peak_left,
        );

        // Draw right bar
        self.draw_bar(
            &mut frame,
            BAR_WIDTH + BAR_GAP,
            CLIP_INDICATOR_HEIGHT + 2.0, // Start Y
            meter_height,
            self.level_right,
            self.peak_right,
        );

        vec![frame.into_geometry()]
    }
}

impl VuMeter {
    /// Draw the clip indicator at the top of the meter.
    fn draw_clip_indicator(&self, frame: &mut Frame, x: f32, clipped: bool) {
        let color = if clipped {
            METER_RED
        } else {
            Color::from_rgba(0.2, 0.2, 0.2, 0.5) // Dim when not clipping
        };

        let path = Path::rectangle(
            Point::new(x, 0.0),
            Size::new(BAR_WIDTH, CLIP_INDICATOR_HEIGHT),
        );
        frame.fill(&path, color);
    }

    /// Draw a single meter bar with gradient coloring and peak hold.
    fn draw_bar(&self, frame: &mut Frame, x: f32, start_y: f32, height: f32, level: f32, peak: f32) {
        // Clamp level to reasonable range
        let level = level.clamp(0.0, 1.5);
        let peak = peak.clamp(0.0, 1.5);

        // Calculate filled height
        // Map 0.0-1.0 to bottom-top of meter, allow overflow for clipping
        let fill_ratio = (level / 1.2).min(1.0); // 1.2 allows some headroom display
        let fill_height = height * fill_ratio;

        if fill_height > 0.0 {
            // Draw the meter bar with gradient sections
            // We draw from bottom up with different colors for each zone
            self.draw_segmented_bar(frame, x, start_y, height, fill_height);
        }

        // Draw peak hold indicator
        if peak > 0.01 {
            let peak_ratio = (peak / 1.2).min(1.0);
            let peak_y = start_y + height * (1.0 - peak_ratio);

            // Choose peak indicator color based on level
            let peak_color = if peak >= RED_THRESHOLD {
                METER_RED
            } else if peak >= ORANGE_THRESHOLD {
                METER_ORANGE
            } else if peak >= YELLOW_THRESHOLD {
                METER_YELLOW
            } else {
                PEAK_INDICATOR_COLOR
            };

            let path = Path::rectangle(
                Point::new(x, peak_y),
                Size::new(BAR_WIDTH, PEAK_INDICATOR_HEIGHT),
            );
            frame.fill(&path, peak_color);
        }
    }

    /// Draw the segmented/gradient meter bar.
    fn draw_segmented_bar(&self, frame: &mut Frame, x: f32, start_y: f32, height: f32, fill_height: f32) {
        let mut current_y = start_y + height; // Bottom of meter
        let mut remaining_fill = fill_height;

        // Green zone (0 to -12dB, which is 0.0 to 0.25 linear)
        let green_zone_height = height * (YELLOW_THRESHOLD / 1.2);
        let green_fill = remaining_fill.min(green_zone_height);
        if green_fill > 0.0 {
            self.draw_zone_segments(frame, x, current_y - green_fill, green_fill, METER_GREEN);
            current_y -= green_fill;
            remaining_fill -= green_fill;
        }

        // Yellow zone (-12dB to -6dB, which is 0.25 to 0.5 linear)
        if remaining_fill > 0.0 {
            let yellow_zone_height = height * ((ORANGE_THRESHOLD - YELLOW_THRESHOLD) / 1.2);
            let yellow_fill = remaining_fill.min(yellow_zone_height);
            if yellow_fill > 0.0 {
                self.draw_zone_segments(
                    frame,
                    x,
                    current_y - yellow_fill,
                    yellow_fill,
                    METER_YELLOW,
                );
                current_y -= yellow_fill;
                remaining_fill -= yellow_fill;
            }
        }

        // Orange zone (-6dB to 0dB, which is 0.5 to 1.0 linear)
        if remaining_fill > 0.0 {
            let orange_zone_height = height * ((RED_THRESHOLD - ORANGE_THRESHOLD) / 1.2);
            let orange_fill = remaining_fill.min(orange_zone_height);
            if orange_fill > 0.0 {
                self.draw_zone_segments(
                    frame,
                    x,
                    current_y - orange_fill,
                    orange_fill,
                    METER_ORANGE,
                );
                current_y -= orange_fill;
                remaining_fill -= orange_fill;
            }
        }

        // Red zone (above 0dB - clipping)
        if remaining_fill > 0.0 {
            self.draw_zone_segments(
                frame,
                x,
                current_y - remaining_fill,
                remaining_fill,
                METER_RED,
            );
        }
    }

    /// Draw a zone with optional segmentation for LED-style look.
    fn draw_zone_segments(&self, frame: &mut Frame, x: f32, y: f32, height: f32, color: Color) {
        if SEGMENT_GAP <= 0.0 {
            // Solid fill
            let path = Path::rectangle(Point::new(x, y), Size::new(BAR_WIDTH, height));
            frame.fill(&path, color);
        } else {
            // Segmented LED-style fill
            let segment_fill_height = SEGMENT_HEIGHT - SEGMENT_GAP;
            let mut seg_y = y + height;

            while seg_y > y {
                let seg_height = (segment_fill_height).min(seg_y - y);
                if seg_height > 0.0 {
                    let path = Path::rectangle(
                        Point::new(x, seg_y - seg_height),
                        Size::new(BAR_WIDTH, seg_height),
                    );
                    frame.fill(&path, color);
                }
                seg_y -= SEGMENT_HEIGHT;
            }
        }
    }
}

// ============================================================================
// PUBLIC API
// ============================================================================

/// Create a VU meter widget element.
pub fn vu_meter<'a>(meter_state: &MeterDisplayState, height: f32) -> Element<'a, Message> {
    canvas::Canvas::new(VuMeter::from_state(meter_state))
        .width(Length::Fixed(METER_WIDTH))
        .height(Length::Fixed(height))
        .into()
}

/// Create an empty VU meter (for when no audio is playing).
pub fn vu_meter_silent<'a>(height: f32) -> Element<'a, Message> {
    canvas::Canvas::new(VuMeter::silent())
        .width(Length::Fixed(METER_WIDTH))
        .height(Length::Fixed(height))
        .into()
}
