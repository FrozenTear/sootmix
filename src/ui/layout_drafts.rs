// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Layout draft implementations for comparison.
//!
//! Two approaches based on professional DAW research:
//!
//! ## Option 1: Bottom Panel (Ableton/FL Studio style)
//! - Channel strips span full width at top
//! - Collapsible bottom drawer for detail view
//! - Resizable via drag handle
//! - Based on Ableton's "Detail View" pattern
//!
//! ## Option 2: Overlay Panel (Logic Pro / Floating Inspector style)
//! - Channel strips span full width
//! - Floating overlay panel appears on channel selection
//! - Semi-modal with backdrop dimming
//! - Can be pinned or auto-dismiss
//!
//! References:
//! - Ableton Live Detail View: https://help.ableton.com/hc/en-us/articles/12243771208092
//! - Logic Pro Inspector: https://support.apple.com/guide/logicpro/inspector-interface-lgcpe9cc3b1d
//! - PatternFly Drawer: https://www.patternfly.org/components/drawer/design-guidelines/
//! - Adobe Slide-out Panels: https://developer.adobe.com/commerce/admin-developer/pattern-library/containers/slideouts-modals-overlays/

use crate::audio::types::OutputDevice;
use crate::message::Message;
use crate::state::{AppState, MixerChannel};
use crate::ui::channel_strip::{channel_strip, master_strip};
use crate::ui::focus_panel::{FocusPluginInfo, FOCUS_PANEL_WIDTH};
use crate::ui::theme::*;
use iced::widget::{
    button, column, container, mouse_area, opaque, row, scrollable, stack, text, Space,
};
use iced::{Alignment, Background, Border, Color, Element, Fill, Length, Theme};
use uuid::Uuid;

// ============================================================================
// OPTION 1: BOTTOM PANEL LAYOUT (Ableton/FL Studio style)
// ============================================================================
//
// Layout Structure:
// ┌─────────────────────────────────────────────────────────────────────────┐
// │ Header                                                                   │
// ├─────────────────────────────────────────────────────────────────────────┤
// │                                                                         │
// │  [Ch1] [Ch2] [Ch3] [Ch4] [Ch5] ... [Master]   ← Full width, scrollable  │
// │                                                                         │
// ├─────────────────────────────────────────────────────────────────────────┤
// │ ═══════════════════ Drag Handle ═══════════════════                     │
// ├─────────────────────────────────────────────────────────────────────────┤
// │                                                                         │
// │  Detail Panel (selected channel info, plugins, EQ, routing)             │
// │  - Can be collapsed (height = 0)                                        │
// │  - Can be resized by dragging the handle                                │
// │  - Shows context based on selection                                     │
// │                                                                         │
// └─────────────────────────────────────────────────────────────────────────┘
//
// Pros:
// - Maximizes horizontal space for channels
// - Familiar pattern from Ableton Live
// - Can be fully collapsed when not needed
// - Works well with horizontal scrolling
//
// Cons:
// - Reduces vertical space for channel strips when open
// - May need minimum height to be useful

/// Default height for the bottom panel when expanded.
pub const BOTTOM_PANEL_DEFAULT_HEIGHT: f32 = 200.0;

/// Minimum height for the bottom panel.
pub const BOTTOM_PANEL_MIN_HEIGHT: f32 = 120.0;

/// Maximum height for the bottom panel.
pub const BOTTOM_PANEL_MAX_HEIGHT: f32 = 400.0;

/// Height of the drag handle area.
pub const DRAG_HANDLE_HEIGHT: f32 = 8.0;

/// Create the bottom panel layout.
///
/// This is the main view function for Option 1.
pub fn bottom_panel_layout<'a>(
    state: &'a AppState,
    header: Element<'a, Message>,
    channel_strips: Element<'a, Message>,
    bottom_panel_height: f32,
    bottom_panel_expanded: bool,
) -> Element<'a, Message> {
    // === DRAG HANDLE ===
    let drag_handle = container(
        container(Space::new().width(60).height(4))
            .style(|_| container::Style {
                background: Some(Background::Color(SOOTMIX_DARK.border_emphasis)),
                border: Border::default().rounded(2.0),
                ..container::Style::default()
            }),
    )
    .width(Fill)
    .height(DRAG_HANDLE_HEIGHT)
    .center_x(Fill)
    .center_y(DRAG_HANDLE_HEIGHT)
    .style(|_| container::Style {
        background: Some(Background::Color(SOOTMIX_DARK.border_subtle)),
        ..container::Style::default()
    });

    // === BOTTOM PANEL CONTENT ===
    let bottom_content: Element<'a, Message> = if bottom_panel_expanded {
        if let Some(channel_id) = state.selected_channel {
            if let Some(channel) = state.channel(channel_id) {
                bottom_detail_panel(channel, &state.available_outputs)
            } else {
                bottom_panel_empty()
            }
        } else {
            bottom_panel_empty()
        }
    } else {
        Space::new().height(0).into()
    };

    // === BOTTOM PANEL CONTAINER ===
    let bottom_panel: Element<'a, Message> = if bottom_panel_expanded {
        column![
            drag_handle,
            container(bottom_content)
                .width(Fill)
                .height(Length::Fixed(bottom_panel_height))
                .style(|_| container::Style {
                    background: Some(Background::Color(SURFACE)),
                    border: Border::default()
                        .color(SOOTMIX_DARK.border_subtle)
                        .width(1.0),
                    ..container::Style::default()
                }),
        ]
        .into()
    } else {
        // Collapsed: just show a thin bar to expand
        button(
            container(text("▲ Show Detail").size(TEXT_CAPTION).color(TEXT_DIM))
                .center_x(Fill)
                .padding([SPACING_XS, 0.0]),
        )
        .width(Fill)
        .padding(0)
        .style(|_: &Theme, status| {
            let is_hovered = matches!(status, button::Status::Hovered);
            button::Style {
                background: Some(Background::Color(if is_hovered {
                    SURFACE_LIGHT
                } else {
                    SURFACE
                })),
                text_color: TEXT_DIM,
                border: Border::default()
                    .color(SOOTMIX_DARK.border_subtle)
                    .width(1.0),
                ..button::Style::default()
            }
        })
        .on_press(Message::ToggleBottomPanel)
        .into()
    };

    // === MAIN LAYOUT ===
    column![
        header,
        Space::new().height(SPACING),
        // Channel strips take remaining vertical space
        container(channel_strips).height(Fill),
        Space::new().height(SPACING_SM),
        // Bottom panel
        bottom_panel,
    ]
    .padding(PADDING)
    .into()
}

/// Bottom panel detail view for selected channel.
fn bottom_detail_panel<'a>(
    channel: &'a MixerChannel,
    available_outputs: &'a [OutputDevice],
) -> Element<'a, Message> {
    let id = channel.id;

    // === HEADER ROW ===
    let title = text(&channel.name)
        .size(TEXT_HEADING)
        .color(TEXT);

    let close_btn = button(text("▼ Hide").size(TEXT_CAPTION).color(TEXT_DIM))
        .padding([SPACING_XS, SPACING_SM])
        .style(|_: &Theme, status| {
            let is_hovered = matches!(status, button::Status::Hovered);
            button::Style {
                background: Some(Background::Color(if is_hovered {
                    SURFACE_LIGHT
                } else {
                    Color::TRANSPARENT
                })),
                text_color: TEXT_DIM,
                border: Border::default().rounded(RADIUS_SM),
                ..button::Style::default()
            }
        })
        .on_press(Message::ToggleBottomPanel);

    let header_row = row![
        title,
        Space::new().width(Fill),
        close_btn,
    ]
    .align_y(Alignment::Center);

    // === CONTENT SECTIONS (Horizontal layout for bottom panel) ===
    // Signal flow: EQ | Plugins | Sends | Output

    // EQ Section
    let eq_section = container(
        column![
            text("EQ").size(TEXT_SMALL).color(TEXT_DIM),
            Space::new().height(SPACING_XS),
            // EQ visualization placeholder
            container(Space::new().width(120).height(60))
                .style(|_| container::Style {
                    background: Some(Background::Color(BACKGROUND)),
                    border: Border::default()
                        .rounded(RADIUS_SM)
                        .color(SOOTMIX_DARK.border_subtle)
                        .width(1.0),
                    ..container::Style::default()
                }),
            Space::new().height(SPACING_XS),
            button(
                text(if channel.eq_enabled { "ON" } else { "OFF" })
                    .size(TEXT_CAPTION)
                    .color(if channel.eq_enabled { TEXT } else { TEXT_DIM }),
            )
            .padding([SPACING_XS, SPACING_SM])
            .style(move |_: &Theme, _| button::Style {
                background: Some(Background::Color(if channel.eq_enabled {
                    SOOTMIX_DARK.semantic_success.scale_alpha(0.3)
                } else {
                    SURFACE
                })),
                border: Border::default().rounded(RADIUS_SM),
                ..button::Style::default()
            })
            .on_press(Message::ChannelEqToggled(id)),
        ]
        .align_x(Alignment::Center),
    )
    .padding(SPACING)
    .style(|_| container::Style {
        background: Some(Background::Color(BACKGROUND)),
        border: Border::default()
            .rounded(RADIUS)
            .color(SOOTMIX_DARK.border_subtle)
            .width(1.0),
        ..container::Style::default()
    });

    // Plugins Section
    let plugin_count = channel.plugin_chain.len();
    let plugins_section = container(
        column![
            text("Plugins").size(TEXT_SMALL).color(TEXT_DIM),
            Space::new().height(SPACING_XS),
            text(format!("{} loaded", plugin_count))
                .size(TEXT_BODY)
                .color(TEXT),
            Space::new().height(SPACING_XS),
            button(text("Open Browser").size(TEXT_CAPTION).color(PRIMARY))
                .padding([SPACING_XS, SPACING_SM])
                .style(|_: &Theme, status| {
                    let is_hovered = matches!(status, button::Status::Hovered);
                    button::Style {
                        background: Some(Background::Color(if is_hovered {
                            PRIMARY.scale_alpha(0.2)
                        } else {
                            Color::TRANSPARENT
                        })),
                        text_color: PRIMARY,
                        border: Border::default()
                            .rounded(RADIUS_SM)
                            .color(PRIMARY)
                            .width(1.0),
                        ..button::Style::default()
                    }
                })
                .on_press(Message::OpenPluginBrowser(id)),
        ]
        .align_x(Alignment::Center),
    )
    .padding(SPACING)
    .width(Length::Fixed(140.0))
    .style(|_| container::Style {
        background: Some(Background::Color(BACKGROUND)),
        border: Border::default()
            .rounded(RADIUS)
            .color(SOOTMIX_DARK.border_subtle)
            .width(1.0),
        ..container::Style::default()
    });

    // Routing Section
    let output_name = channel
        .output_device_name
        .as_deref()
        .unwrap_or("Default");
    let routing_section = container(
        column![
            text("Output").size(TEXT_SMALL).color(TEXT_DIM),
            Space::new().height(SPACING_XS),
            text(output_name).size(TEXT_BODY).color(TEXT),
            Space::new().height(SPACING_XS),
            text(format!("{:+.1} dB", channel.volume_db))
                .size(TEXT_BODY)
                .color(SOOTMIX_DARK.accent_warm),
        ]
        .align_x(Alignment::Center),
    )
    .padding(SPACING)
    .width(Length::Fixed(120.0))
    .style(|_| container::Style {
        background: Some(Background::Color(BACKGROUND)),
        border: Border::default()
            .rounded(RADIUS)
            .color(SOOTMIX_DARK.border_subtle)
            .width(1.0),
        ..container::Style::default()
    });

    // Assigned Apps Section
    let apps_count = channel.assigned_apps.len();
    let apps_section = container(
        column![
            text("Sources").size(TEXT_SMALL).color(TEXT_DIM),
            Space::new().height(SPACING_XS),
            text(format!("{} app{}", apps_count, if apps_count == 1 { "" } else { "s" }))
                .size(TEXT_BODY)
                .color(TEXT),
        ]
        .align_x(Alignment::Center),
    )
    .padding(SPACING)
    .width(Length::Fixed(100.0))
    .style(|_| container::Style {
        background: Some(Background::Color(BACKGROUND)),
        border: Border::default()
            .rounded(RADIUS)
            .color(SOOTMIX_DARK.border_subtle)
            .width(1.0),
        ..container::Style::default()
    });

    // Quick Actions
    let mute_btn = button(
        text(if channel.muted { "UNMUTE" } else { "MUTE" })
            .size(TEXT_SMALL)
            .color(if channel.muted {
                SOOTMIX_DARK.semantic_error
            } else {
                TEXT
            }),
    )
    .padding([SPACING_SM, SPACING])
    .style(move |_: &Theme, status| {
        let is_hovered = matches!(status, button::Status::Hovered);
        button::Style {
            background: Some(Background::Color(if channel.muted {
                if is_hovered {
                    SOOTMIX_DARK.semantic_error
                } else {
                    SOOTMIX_DARK.semantic_error.scale_alpha(0.3)
                }
            } else if is_hovered {
                SURFACE_LIGHT
            } else {
                SURFACE
            })),
            border: Border::default()
                .rounded(RADIUS)
                .color(if channel.muted {
                    SOOTMIX_DARK.semantic_error
                } else {
                    SOOTMIX_DARK.border_subtle
                })
                .width(1.0),
            ..button::Style::default()
        }
    })
    .on_press(Message::ChannelMuteToggled(id));

    // === ASSEMBLE ===
    let content_row = row![
        eq_section,
        Space::new().width(SPACING),
        plugins_section,
        Space::new().width(SPACING),
        routing_section,
        Space::new().width(SPACING),
        apps_section,
        Space::new().width(Fill),
        mute_btn,
    ]
    .align_y(Alignment::Start);

    let scrollable_content = scrollable(content_row)
        .direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::default().width(4).scroller_width(4),
        ));

    column![
        header_row,
        Space::new().height(SPACING),
        scrollable_content,
    ]
    .padding(SPACING)
    .into()
}

/// Empty state for bottom panel when no channel is selected.
fn bottom_panel_empty<'a>() -> Element<'a, Message> {
    container(
        column![
            text("Select a channel to view details")
                .size(TEXT_BODY)
                .color(TEXT_DIM),
        ]
        .align_x(Alignment::Center),
    )
    .width(Fill)
    .height(Fill)
    .center_x(Fill)
    .center_y(Fill)
    .into()
}

// ============================================================================
// OPTION 2: OVERLAY PANEL LAYOUT (Logic Pro / Floating Inspector style)
// ============================================================================
//
// Layout Structure:
// ┌─────────────────────────────────────────────────────────────────────────┐
// │ Header                                                                   │
// ├─────────────────────────────────────────────────────────────────────────┤
// │                                                                         │
// │  [Ch1] [Ch2] [Ch3] [Ch4] [Ch5] ... [Master]   ← Full width, scrollable  │
// │                                                                         │
// │         ┌────────────────────┐                                          │
// │         │  Floating Panel    │  ← Appears near selected channel         │
// │         │  ─────────────────│                                          │
// │         │  Channel: "Music"  │                                          │
// │         │  EQ: ON            │                                          │
// │         │  Plugins: 2        │                                          │
// │         │  [Edit] [Close]    │                                          │
// │         └────────────────────┘                                          │
// │                                                                         │
// │  [Apps Panel - collapsed or side]                                       │
// └─────────────────────────────────────────────────────────────────────────┘
//
// Pros:
// - Doesn't reduce space for channels or consume permanent screen space
// - Familiar from Logic Pro's floating inspectors
// - Can be positioned near the relevant channel
// - Can be pinned open or set to auto-dismiss
//
// Cons:
// - Covers channel strips when open (though backdrop makes this clear)
// - May feel less "integrated" than a panel
// - Position needs to be managed

/// Create the overlay panel layout.
///
/// This is the main view function for Option 2.
/// Uses a stack to layer the overlay on top of the main content.
pub fn overlay_panel_layout<'a>(
    state: &'a AppState,
    header: Element<'a, Message>,
    channel_strips: Element<'a, Message>,
    apps_panel: Element<'a, Message>,
) -> Element<'a, Message> {
    // === BASE LAYER (Main content) ===
    let main_content = column![
        header,
        Space::new().height(SPACING),
        container(channel_strips).height(Fill),
        Space::new().height(SPACING),
        apps_panel,
    ]
    .padding(PADDING);

    // === OVERLAY LAYER ===
    let overlay: Element<'a, Message> = if let Some(channel_id) = state.selected_channel {
        if let Some(channel) = state.channel(channel_id) {
            // Create backdrop + floating panel
            let backdrop = mouse_area(
                container(Space::new().width(Fill).height(Fill))
                    .width(Fill)
                    .height(Fill)
                    .style(|_| container::Style {
                        background: Some(Background::Color(Color {
                            a: 0.5,
                            ..Color::BLACK
                        })),
                        ..container::Style::default()
                    }),
            )
            .on_press(Message::SelectChannel(None)); // Click backdrop to close

            let panel = floating_detail_panel(channel, &state.available_outputs);

            // Center the panel in the viewport
            let centered_panel = container(panel)
                .width(Fill)
                .height(Fill)
                .center_x(Fill)
                .center_y(Fill);

            // Stack backdrop behind panel
            stack![
                backdrop,
                opaque(centered_panel),
            ]
            .into()
        } else {
            Space::new().width(0).height(0).into()
        }
    } else {
        Space::new().width(0).height(0).into()
    };

    // === COMBINE WITH STACK ===
    if state.selected_channel.is_some() {
        stack![
            main_content,
            overlay,
        ]
        .into()
    } else {
        main_content.into()
    }
}

/// Floating detail panel for selected channel.
fn floating_detail_panel<'a>(
    channel: &'a MixerChannel,
    _available_outputs: &'a [OutputDevice],
) -> Element<'a, Message> {
    let id = channel.id;

    // === HEADER ===
    let title = text(&channel.name)
        .size(TEXT_HEADING)
        .color(TEXT);

    let subtitle = text("Channel Inspector")
        .size(TEXT_CAPTION)
        .color(TEXT_DIM);

    let close_btn = button(text("×").size(TEXT_HEADING).color(TEXT_DIM))
        .padding([SPACING_XS, SPACING_SM])
        .style(|_: &Theme, status| {
            let is_hovered = matches!(status, button::Status::Hovered);
            button::Style {
                background: Some(Background::Color(if is_hovered {
                    SURFACE_LIGHT
                } else {
                    Color::TRANSPARENT
                })),
                text_color: TEXT_DIM,
                border: Border::default().rounded(RADIUS_SM),
                ..button::Style::default()
            }
        })
        .on_press(Message::SelectChannel(None));

    let header = row![
        column![title, subtitle].spacing(2),
        Space::new().width(Fill),
        close_btn,
    ]
    .align_y(Alignment::Center);

    // === DIVIDER ===
    let divider = container(Space::new().width(Fill).height(1))
        .style(|_| container::Style {
            background: Some(Background::Color(SOOTMIX_DARK.border_subtle)),
            ..container::Style::default()
        });

    // === SIGNAL FLOW VISUALIZATION ===
    let flow_badges = row![
        flow_badge_overlay("IN", PRIMARY),
        text("→").size(TEXT_SMALL).color(TEXT_DIM),
        flow_badge_overlay(
            "EQ",
            if channel.eq_enabled {
                SOOTMIX_DARK.semantic_success
            } else {
                SOOTMIX_DARK.text_muted
            }
        ),
        text("→").size(TEXT_SMALL).color(TEXT_DIM),
        flow_badge_overlay(
            "FX",
            if !channel.plugin_chain.is_empty() {
                SOOTMIX_DARK.semantic_warning
            } else {
                SOOTMIX_DARK.text_muted
            }
        ),
        text("→").size(TEXT_SMALL).color(TEXT_DIM),
        flow_badge_overlay("OUT", SOOTMIX_DARK.accent_secondary),
    ]
    .spacing(SPACING_XS)
    .align_y(Alignment::Center);

    // === INFO GRID ===
    let info_rows = column![
        info_row("Volume", format!("{:+.1} dB", channel.volume_db)),
        info_row("Status", if channel.muted { "Muted".to_string() } else { "Active".to_string() }),
        info_row("EQ", if channel.eq_enabled { "Enabled".to_string() } else { "Disabled".to_string() }),
        info_row("Plugins", format!("{} loaded", channel.plugin_chain.len())),
        info_row("Sources", format!("{} apps", channel.assigned_apps.len())),
    ]
    .spacing(SPACING_XS);

    // === QUICK ACTIONS ===
    let mute_btn = button(
        text(if channel.muted { "Unmute" } else { "Mute" })
            .size(TEXT_SMALL),
    )
    .padding([SPACING_SM, SPACING])
    .width(Fill)
    .style(move |_: &Theme, status| {
        let is_hovered = matches!(status, button::Status::Hovered);
        button::Style {
            background: Some(Background::Color(if channel.muted {
                if is_hovered {
                    SOOTMIX_DARK.semantic_error
                } else {
                    SOOTMIX_DARK.semantic_error.scale_alpha(0.3)
                }
            } else if is_hovered {
                SURFACE_LIGHT
            } else {
                SURFACE
            })),
            text_color: if channel.muted { TEXT } else { TEXT },
            border: Border::default()
                .rounded(RADIUS)
                .color(if channel.muted {
                    SOOTMIX_DARK.semantic_error
                } else {
                    SOOTMIX_DARK.border_subtle
                })
                .width(1.0),
            ..button::Style::default()
        }
    })
    .on_press(Message::ChannelMuteToggled(id));

    let plugins_btn = button(text("Open Plugins").size(TEXT_SMALL))
        .padding([SPACING_SM, SPACING])
        .width(Fill)
        .style(|_: &Theme, status| {
            let is_hovered = matches!(status, button::Status::Hovered);
            button::Style {
                background: Some(Background::Color(if is_hovered {
                    PRIMARY.scale_alpha(0.2)
                } else {
                    Color::TRANSPARENT
                })),
                text_color: PRIMARY,
                border: Border::default()
                    .rounded(RADIUS)
                    .color(PRIMARY)
                    .width(1.0),
                ..button::Style::default()
            }
        })
        .on_press(Message::OpenPluginBrowser(id));

    let actions = column![mute_btn, plugins_btn,].spacing(SPACING_SM);

    // === ASSEMBLE PANEL ===
    let content = column![
        header,
        Space::new().height(SPACING),
        divider,
        Space::new().height(SPACING),
        flow_badges,
        Space::new().height(SPACING_MD),
        info_rows,
        Space::new().height(SPACING_MD),
        actions,
    ]
    .padding(SPACING_MD);

    container(content)
        .width(Length::Fixed(320.0))
        .style(|_| container::Style {
            background: Some(Background::Color(SURFACE)),
            border: Border::default()
                .rounded(RADIUS_LG)
                .color(SOOTMIX_DARK.border_emphasis)
                .width(1.0),
            shadow: iced::Shadow {
                color: Color {
                    a: 0.4,
                    ..Color::BLACK
                },
                offset: iced::Vector::new(0.0, 8.0),
                blur_radius: 24.0,
            },
            ..container::Style::default()
        })
        .into()
}

/// Helper: Create a flow badge for the overlay panel.
fn flow_badge_overlay<'a>(label: &'a str, color: Color) -> Element<'a, Message> {
    container(text(label).size(TEXT_CAPTION).color(TEXT))
        .padding([3, 8])
        .style(move |_| container::Style {
            background: Some(Background::Color(color.scale_alpha(0.3))),
            border: Border::default()
                .rounded(RADIUS_SM)
                .color(color)
                .width(1.0),
            ..container::Style::default()
        })
        .into()
}

/// Helper: Create an info row (label: value).
fn info_row(label: &'static str, value: String) -> Element<'static, Message> {
    row![
        text(label).size(TEXT_SMALL).color(TEXT_DIM),
        Space::new().width(Fill),
        text(value).size(TEXT_SMALL).color(TEXT),
    ]
    .into()
}

// ============================================================================
// COMPARISON SUMMARY
// ============================================================================
//
// | Feature                | Option 1: Bottom Panel    | Option 2: Overlay Panel   |
// |------------------------|---------------------------|---------------------------|
// | Channel strip space    | Full width always         | Full width always         |
// | Vertical space impact  | Reduces when open         | None (overlays content)   |
// | Familiarity           | Ableton, FL Studio        | Logic Pro, modal patterns |
// | Persistence           | Stays open until closed   | Dismisses on click outside|
// | Resizable             | Yes (drag handle)         | No (fixed size)           |
// | Context switching     | Low (always visible)      | Medium (appears/disappears)|
// | Implementation        | Column layout             | Stack with overlay        |
// | Mobile-friendly       | Better                    | Good                      |
//
// Recommendation:
// - Use Option 1 (Bottom Panel) if users frequently need to see channel details
//   while working with multiple channels
// - Use Option 2 (Overlay Panel) if channel details are accessed infrequently
//   and maximum channel visibility is priority
