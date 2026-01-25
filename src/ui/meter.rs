// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! VU meter widget for audio level display.

use crate::message::Message;
use crate::state::MeterDisplayState;
use iced::widget::canvas::{self, Frame, Geometry, Path};
use iced::{mouse, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

/// Width of a single meter bar in pixels.
const BAR_WIDTH: f32 = 4.0;
/// Gap between stereo bars.
const BAR_GAP: f32 = 2.0;
/// Total width of stereo meter (2 bars + gap).
pub const METER_WIDTH: f32 = BAR_WIDTH * 2.0 + BAR_GAP;
/// Peak hold indicator height.
const PEAK_INDICATOR_HEIGHT: f32 = 2.0;

/// Meter color thresholds in linear scale (0.0 to 1.0+).
/// These correspond to dB levels: -12dB, -6dB, 0dB
const YELLOW_THRESHOLD: f32 = 0.25; // -12dB
const ORANGE_THRESHOLD: f32 = 0.5;  // -6dB
const RED_THRESHOLD: f32 = 1.0;     // 0dB (clipping)

/// Color for the green (low level) portion of the meter.
const METER_GREEN: Color = Color::from_rgb(0.20, 0.70, 0.30);
/// Color for the yellow (-12dB to -6dB) portion.
const METER_YELLOW: Color = Color::from_rgb(0.85, 0.75, 0.15);
/// Color for the orange (-6dB to 0dB) portion.
const METER_ORANGE: Color = Color::from_rgb(0.90, 0.50, 0.10);
/// Color for the red (clipping) portion.
const METER_RED: Color = Color::from_rgb(0.90, 0.25, 0.20);
/// Color for the inactive/background portion.
const METER_BACKGROUND: Color = Color::from_rgb(0.15, 0.15, 0.17);
/// Color for peak hold indicator.
const PEAK_INDICATOR_COLOR: Color = Color::from_rgb(1.0, 1.0, 1.0);

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
}

impl VuMeter {
    /// Create a new VU meter from display state.
    pub fn from_state(state: &MeterDisplayState) -> Self {
        Self {
            level_left: state.level_left,
            level_right: state.level_right,
            peak_left: state.peak_hold_left,
            peak_right: state.peak_hold_right,
        }
    }

    /// Create an empty (silent) meter.
    pub fn silent() -> Self {
        Self {
            level_left: 0.0,
            level_right: 0.0,
            peak_left: 0.0,
            peak_right: 0.0,
        }
    }
}

impl Default for VuMeter {
    fn default() -> Self {
        Self::silent()
    }
}

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

        // Draw background for both bars
        let bg_path = Path::rectangle(Point::ORIGIN, Size::new(METER_WIDTH, height));
        frame.fill(&bg_path, METER_BACKGROUND);

        // Draw left bar
        self.draw_bar(&mut frame, 0.0, height, self.level_left, self.peak_left);

        // Draw right bar
        self.draw_bar(
            &mut frame,
            BAR_WIDTH + BAR_GAP,
            height,
            self.level_right,
            self.peak_right,
        );

        vec![frame.into_geometry()]
    }
}

impl VuMeter {
    /// Draw a single meter bar with gradient coloring and peak hold.
    fn draw_bar(&self, frame: &mut Frame, x: f32, height: f32, level: f32, peak: f32) {
        // Clamp level to reasonable range
        let level = level.clamp(0.0, 1.5);
        let peak = peak.clamp(0.0, 1.5);

        // Calculate filled height (level is linear, but display is linear too for simplicity)
        // Map 0.0-1.0 to bottom-top of meter, allow overflow for clipping
        let fill_ratio = (level / 1.2).min(1.0); // 1.2 allows some headroom display
        let fill_height = height * fill_ratio;

        if fill_height > 0.0 {
            // Draw the meter bar with gradient sections
            // We draw from bottom up with different colors for each zone

            let mut current_y = height;
            let mut remaining_fill = fill_height;

            // Green zone (0 to -12dB, which is 0.0 to 0.25 linear)
            let green_zone_height = height * (YELLOW_THRESHOLD / 1.2);
            let green_fill = remaining_fill.min(green_zone_height);
            if green_fill > 0.0 {
                let path = Path::rectangle(
                    Point::new(x, current_y - green_fill),
                    Size::new(BAR_WIDTH, green_fill),
                );
                frame.fill(&path, METER_GREEN);
                current_y -= green_fill;
                remaining_fill -= green_fill;
            }

            // Yellow zone (-12dB to -6dB, which is 0.25 to 0.5 linear)
            if remaining_fill > 0.0 {
                let yellow_zone_height = height * ((ORANGE_THRESHOLD - YELLOW_THRESHOLD) / 1.2);
                let yellow_fill = remaining_fill.min(yellow_zone_height);
                if yellow_fill > 0.0 {
                    let path = Path::rectangle(
                        Point::new(x, current_y - yellow_fill),
                        Size::new(BAR_WIDTH, yellow_fill),
                    );
                    frame.fill(&path, METER_YELLOW);
                    current_y -= yellow_fill;
                    remaining_fill -= yellow_fill;
                }
            }

            // Orange zone (-6dB to 0dB, which is 0.5 to 1.0 linear)
            if remaining_fill > 0.0 {
                let orange_zone_height = height * ((RED_THRESHOLD - ORANGE_THRESHOLD) / 1.2);
                let orange_fill = remaining_fill.min(orange_zone_height);
                if orange_fill > 0.0 {
                    let path = Path::rectangle(
                        Point::new(x, current_y - orange_fill),
                        Size::new(BAR_WIDTH, orange_fill),
                    );
                    frame.fill(&path, METER_ORANGE);
                    current_y -= orange_fill;
                    remaining_fill -= orange_fill;
                }
            }

            // Red zone (above 0dB - clipping)
            if remaining_fill > 0.0 {
                let path = Path::rectangle(
                    Point::new(x, current_y - remaining_fill),
                    Size::new(BAR_WIDTH, remaining_fill),
                );
                frame.fill(&path, METER_RED);
            }
        }

        // Draw peak hold indicator
        if peak > 0.01 {
            let peak_ratio = (peak / 1.2).min(1.0);
            let peak_y = height * (1.0 - peak_ratio);

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
}

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
