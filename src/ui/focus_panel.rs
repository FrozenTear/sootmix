// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Focus panel UI component - detailed view of selected channel.
//!
//! Displays comprehensive channel information in a side panel following
//! the Harrison Mixbus "Focus Channel" pattern. Shows signal flow,
//! EQ, plugins, and routing in an expanded horizontal layout.

#![allow(dead_code)]

use crate::audio::types::OutputDevice;
use crate::message::Message;
use crate::state::MixerChannel;
use crate::ui::theme::*;
use crate::state::ChannelKind;
use iced::widget::{
    button, column, container, row, scrollable, slider, text, Space,
};
use iced::{Alignment, Background, Border, Color, Element, Fill, Length, Theme};
use uuid::Uuid;

/// Width of the focus panel.
pub const FOCUS_PANEL_WIDTH: f32 = 320.0;

/// Plugin info for display in the focus panel.
pub struct FocusPluginInfo {
    pub instance_id: Uuid,
    pub name: String,
    pub bypassed: bool,
}

/// Create the focus panel for a selected channel.
pub fn focus_panel<'a>(
    channel: &'a MixerChannel,
    available_outputs: &'a [OutputDevice],
    plugin_chain: Vec<FocusPluginInfo>,
) -> Element<'a, Message> {
    let id = channel.id;

    // === HEADER ===
    let header = focus_header(channel);

    // === SIGNAL FLOW SECTION ===
    let signal_flow = signal_flow_section(channel);

    // === INPUT SOURCES ===
    let inputs = input_sources_section(channel);

    // === NOISE SUPPRESSION SECTION (input channels only) ===
    let noise_section: Element<Message> = if channel.kind == ChannelKind::Input {
        noise_suppression_section(channel)
    } else {
        Space::new().width(0).height(0).into()
    };

    // === PLUGIN CHAIN SECTION ===
    let plugins = plugin_chain_section(id, plugin_chain);

    // === OUTPUT ROUTING ===
    let output = output_section(channel, available_outputs);

    // === QUICK ACTIONS ===
    let actions = quick_actions_section(channel);

    // Horizontal rule as styled container
    let divider: Element<Message> = container(Space::new().width(Fill).height(1))
        .style(|_| container::Style {
            background: Some(Background::Color(SOOTMIX_DARK.border_subtle)),
            ..container::Style::default()
        })
        .into();

    // Assemble all sections
    let content = column![
        header,
        Space::new().height(SPACING_SM),
        divider,
        Space::new().height(SPACING),
        signal_flow,
        Space::new().height(SPACING),
        inputs,
        Space::new().height(SPACING),
        noise_section,
        Space::new().height(SPACING),
        plugins,
        Space::new().height(SPACING),
        output,
        Space::new().height(SPACING_MD),
        actions,
    ]
    .padding(PADDING);

    let scrollable_content = scrollable(content)
        .direction(scrollable::Direction::Vertical(
            scrollable::Scrollbar::default().width(4).scroller_width(4),
        ));

    container(scrollable_content)
        .width(Length::Fixed(FOCUS_PANEL_WIDTH))
        .height(Fill)
        .style(|_| container::Style {
            background: Some(Background::Color(SURFACE)),
            border: Border::default()
                .rounded(RADIUS)
                .color(SOOTMIX_DARK.border_subtle)
                .width(1.0),
            ..container::Style::default()
        })
        .into()
}

/// Focus panel header with channel name and close button.
fn focus_header<'a>(channel: &'a MixerChannel) -> Element<'a, Message> {
    let title = text(&channel.name)
        .size(TEXT_HEADING)
        .color(TEXT);

    let subtitle = text("Channel Detail")
        .size(TEXT_CAPTION)
        .color(TEXT_DIM);

    let close_btn = button(text("x").size(TEXT_BODY).color(TEXT_DIM))
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

    row![
        column![title, subtitle].spacing(2),
        Space::new().width(Fill),
        close_btn,
    ]
    .align_y(Alignment::Center)
    .into()
}

/// Signal flow indicator showing the processing chain visually.
fn signal_flow_section<'a>(channel: &'a MixerChannel) -> Element<'a, Message> {
    let section_title = text("Signal Flow")
        .size(TEXT_SMALL)
        .color(TEXT_DIM);

    // Signal flow: Input -> EQ -> Plugins -> Fader -> Output
    let input_badge = flow_badge("IN", SOOTMIX_DARK.accent_primary);
    let eq_badge = flow_badge(
        "EQ",
        if channel.eq_enabled {
            SOOTMIX_DARK.semantic_success
        } else {
            SOOTMIX_DARK.text_muted
        },
    );
    let fx_badge = flow_badge(
        "FX",
        if !channel.plugin_chain.is_empty() {
            SOOTMIX_DARK.semantic_warning
        } else {
            SOOTMIX_DARK.text_muted
        },
    );
    let fader_badge = flow_badge("VOL", SOOTMIX_DARK.accent_warm);
    let out_badge = flow_badge("OUT", SOOTMIX_DARK.accent_secondary);

    // Create arrow separators (can't clone text widgets)
    let arrow1 = text(">").size(TEXT_SMALL).color(TEXT_DIM);
    let arrow2 = text(">").size(TEXT_SMALL).color(TEXT_DIM);
    let arrow3 = text(">").size(TEXT_SMALL).color(TEXT_DIM);
    let arrow4 = text(">").size(TEXT_SMALL).color(TEXT_DIM);

    let flow_row = row![
        input_badge,
        arrow1,
        eq_badge,
        arrow2,
        fx_badge,
        arrow3,
        fader_badge,
        arrow4,
        out_badge,
    ]
    .spacing(SPACING_XS)
    .align_y(Alignment::Center);

    column![section_title, Space::new().height(SPACING_XS), flow_row,].into()
}

/// Create a signal flow badge.
fn flow_badge<'a>(label: &'a str, color: Color) -> Element<'a, Message> {
    container(text(label).size(TEXT_CAPTION).color(TEXT))
        .padding([2, 6])
        .style(move |_| container::Style {
            background: Some(Background::Color(color.scale_alpha(0.3))),
            border: Border::default().rounded(RADIUS_SM).color(color).width(1.0),
            ..container::Style::default()
        })
        .into()
}

/// Input sources section showing assigned apps.
fn input_sources_section<'a>(channel: &'a MixerChannel) -> Element<'a, Message> {
    let section_title = text("Input Sources")
        .size(TEXT_SMALL)
        .color(TEXT_DIM);

    let apps_content: Element<Message> = if channel.assigned_apps.is_empty() {
        text("No apps assigned")
            .size(TEXT_SMALL)
            .color(TEXT_DIM)
            .into()
    } else {
        let app_badges: Vec<Element<Message>> = channel
            .assigned_apps
            .iter()
            .map(|app| {
                let id = channel.id;
                let app_name = app.clone();
                container(
                    row![
                        text(app).size(TEXT_SMALL).color(TEXT),
                        Space::new().width(SPACING_XS),
                        button(text("x").size(9).color(TEXT_DIM))
                            .padding([1, 4])
                            .style(|_: &Theme, status| {
                                let is_hovered = matches!(status, button::Status::Hovered);
                                button::Style {
                                    background: Some(Background::Color(if is_hovered {
                                        SOOTMIX_DARK.semantic_error.scale_alpha(0.3)
                                    } else {
                                        Color::TRANSPARENT
                                    })),
                                    text_color: TEXT_DIM,
                                    border: Border::default().rounded(2.0),
                                    ..button::Style::default()
                                }
                            })
                            .on_press(Message::AppUnassigned(id, app_name)),
                    ]
                    .align_y(Alignment::Center),
                )
                .padding([SPACING_XS, SPACING_SM])
                .style(|_| container::Style {
                    background: Some(Background::Color(SURFACE_LIGHT)),
                    border: Border::default()
                        .rounded(RADIUS_SM)
                        .color(SOOTMIX_DARK.border_subtle)
                        .width(1.0),
                    ..container::Style::default()
                })
                .into()
            })
            .collect();

        column(app_badges).spacing(SPACING_XS).into()
    };

    column![
        section_title,
        Space::new().height(SPACING_XS),
        apps_content,
    ]
    .into()
}

/// EQ section with toggle and preset.
fn eq_section<'a>(channel: &'a MixerChannel) -> Element<'a, Message> {
    let id = channel.id;
    let section_title = text("Equalizer")
        .size(TEXT_SMALL)
        .color(TEXT_DIM);

    let eq_toggle = button(
        text(if channel.eq_enabled { "ON" } else { "OFF" })
            .size(TEXT_SMALL)
            .color(if channel.eq_enabled { TEXT } else { TEXT_DIM }),
    )
    .padding([SPACING_XS, SPACING_SM])
    .style(move |_: &Theme, status| {
        let is_hovered = matches!(status, button::Status::Hovered);
        let bg = if channel.eq_enabled {
            if is_hovered {
                SOOTMIX_DARK.semantic_success
            } else {
                SOOTMIX_DARK.semantic_success.scale_alpha(0.7)
            }
        } else if is_hovered {
            SURFACE_LIGHT
        } else {
            SURFACE
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: if channel.eq_enabled { TEXT } else { TEXT_DIM },
            border: Border::default()
                .rounded(RADIUS_SM)
                .color(if channel.eq_enabled {
                    SOOTMIX_DARK.semantic_success
                } else {
                    SOOTMIX_DARK.border_subtle
                })
                .width(1.0),
            ..button::Style::default()
        }
    })
    .on_press(Message::ChannelEqToggled(id));

    let preset_label = text(format!("Preset: {}", channel.eq_preset))
        .size(TEXT_SMALL)
        .color(TEXT);

    // Placeholder for EQ curve visualization
    let eq_visual = container(Space::new().width(Fill).height(60))
        .style(|_| container::Style {
            background: Some(Background::Color(BACKGROUND)),
            border: Border::default()
                .rounded(RADIUS_SM)
                .color(SOOTMIX_DARK.border_subtle)
                .width(1.0),
            ..container::Style::default()
        });

    column![
        row![section_title, Space::new().width(Fill), eq_toggle,].align_y(Alignment::Center),
        Space::new().height(SPACING_XS),
        preset_label,
        Space::new().height(SPACING_SM),
        eq_visual,
    ]
    .into()
}

/// Noise suppression section for input channels.
fn noise_suppression_section<'a>(channel: &'a MixerChannel) -> Element<'a, Message> {
    let id = channel.id;
    let ns_enabled = channel.noise_suppression_enabled;
    let vad_threshold = channel.vad_threshold;

    let section_title = text("Noise Suppression")
        .size(TEXT_SMALL)
        .color(TEXT_DIM);

    // NS toggle button
    let ns_toggle = button(
        text(if ns_enabled { "ON" } else { "OFF" })
            .size(TEXT_SMALL)
            .color(if ns_enabled { TEXT } else { TEXT_DIM }),
    )
    .padding([SPACING_XS, SPACING_SM])
    .style(move |_: &Theme, status| {
        let is_hovered = matches!(status, button::Status::Hovered);
        let bg = if ns_enabled {
            if is_hovered {
                PRIMARY
            } else {
                PRIMARY.scale_alpha(0.7)
            }
        } else if is_hovered {
            SURFACE_LIGHT
        } else {
            SURFACE
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: if ns_enabled { TEXT } else { TEXT_DIM },
            border: Border::default()
                .rounded(RADIUS_SM)
                .color(if ns_enabled { PRIMARY } else { SOOTMIX_DARK.border_subtle })
                .width(1.0),
            ..button::Style::default()
        }
    })
    .on_press(Message::ChannelNoiseSuppressionToggled(id));

    // VAD threshold slider (only show when NS is enabled)
    let vad_section: Element<Message> = if ns_enabled {
        let vad_label = text("Voice Threshold")
            .size(TEXT_SMALL)
            .color(TEXT_DIM);

        let vad_value = text(format!("{}%", vad_threshold as i32))
            .size(TEXT_SMALL)
            .color(TEXT);

        let vad_slider = slider(0.0..=100.0, vad_threshold, move |v| {
            Message::ChannelVADThresholdChanged(id, v)
        })
        .step(1.0)
        .width(Length::Fill)
        .style(|_theme: &Theme, _status| slider::Style {
            rail: slider::Rail {
                backgrounds: (
                    Background::Color(PRIMARY),
                    Background::Color(SLIDER_TRACK),
                ),
                width: 4.0,
                border: Border::default().rounded(2.0),
            },
            handle: slider::Handle {
                shape: slider::HandleShape::Rectangle {
                    width: 12,
                    border_radius: RADIUS_SM.into(),
                },
                background: Background::Color(TEXT),
                border_width: 0.0,
                border_color: Color::TRANSPARENT,
            },
        });

        let help_text = text("Higher = more aggressive noise filtering")
            .size(TEXT_CAPTION)
            .color(TEXT_DIM);

        column![
            Space::new().height(SPACING_SM),
            row![vad_label, Space::new().width(Fill), vad_value,].align_y(Alignment::Center),
            Space::new().height(SPACING_XS),
            vad_slider,
            Space::new().height(SPACING_XS),
            help_text,
        ]
        .into()
    } else {
        column![
            Space::new().height(SPACING_XS),
            text("Enable to adjust voice threshold")
                .size(TEXT_CAPTION)
                .color(TEXT_DIM),
        ]
        .into()
    };

    column![
        row![section_title, Space::new().width(Fill), ns_toggle,].align_y(Alignment::Center),
        vad_section,
    ]
    .into()
}

/// Plugin chain section with expanded plugin list.
fn plugin_chain_section<'a>(
    channel_id: Uuid,
    plugins: Vec<FocusPluginInfo>,
) -> Element<'a, Message> {
    let section_title = text("Plugin Chain")
        .size(TEXT_SMALL)
        .color(TEXT_DIM);

    let add_btn = button(text("+").size(TEXT_BODY).color(PRIMARY))
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
        .on_press(Message::OpenPluginBrowser(channel_id));

    let plugins_content: Element<Message> = if plugins.is_empty() {
        container(
            text("No plugins - click + to add")
                .size(TEXT_SMALL)
                .color(TEXT_DIM),
        )
        .padding(SPACING)
        .center_x(Fill)
        .into()
    } else {
        let plugin_rows: Vec<Element<Message>> = plugins
            .into_iter()
            .enumerate()
            .map(|(idx, plugin)| {
                focus_plugin_row(channel_id, idx, plugin)
            })
            .collect();

        column(plugin_rows).spacing(SPACING_XS).into()
    };

    column![
        row![section_title, Space::new().width(Fill), add_btn,].align_y(Alignment::Center),
        Space::new().height(SPACING_SM),
        plugins_content,
    ]
    .into()
}

/// Single plugin row in focus panel.
fn focus_plugin_row<'a>(
    channel_id: Uuid,
    idx: usize,
    plugin: FocusPluginInfo,
) -> Element<'a, Message> {
    let instance_id = plugin.instance_id;

    // Index number
    let index_label = text(format!("{}.", idx + 1))
        .size(TEXT_CAPTION)
        .color(TEXT_DIM);

    // Plugin name (consume the owned string)
    let name_label = text(plugin.name)
        .size(TEXT_SMALL)
        .color(if plugin.bypassed { TEXT_DIM } else { TEXT });

    // Bypass indicator
    let bypass_indicator: Element<Message> = if plugin.bypassed {
        text("BYP")
            .size(TEXT_CAPTION)
            .color(SOOTMIX_DARK.semantic_warning)
            .into()
    } else {
        Space::new().width(0).into()
    };

    // Edit button
    let edit_btn = button(text("Edit").size(TEXT_CAPTION).color(TEXT_DIM))
        .padding([2, 6])
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
        .on_press(Message::OpenPluginEditor(channel_id, instance_id));

    // Bypass toggle button
    let bypass_btn = button(text("B").size(TEXT_CAPTION))
        .padding([2, 6])
        .style(move |_: &Theme, status| {
            let is_hovered = matches!(status, button::Status::Hovered);
            let bg = if plugin.bypassed {
                SOOTMIX_DARK.semantic_warning.scale_alpha(0.3)
            } else if is_hovered {
                SURFACE_LIGHT
            } else {
                Color::TRANSPARENT
            };
            button::Style {
                background: Some(Background::Color(bg)),
                text_color: if plugin.bypassed {
                    SOOTMIX_DARK.semantic_warning
                } else {
                    TEXT_DIM
                },
                border: Border::default().rounded(RADIUS_SM),
                ..button::Style::default()
            }
        })
        .on_press(Message::TogglePluginBypass(channel_id, instance_id));

    // Remove button
    let remove_btn = button(text("x").size(TEXT_CAPTION).color(TEXT_DIM))
        .padding([2, 6])
        .style(|_: &Theme, status| {
            let is_hovered = matches!(status, button::Status::Hovered);
            button::Style {
                background: Some(Background::Color(if is_hovered {
                    SOOTMIX_DARK.semantic_error.scale_alpha(0.3)
                } else {
                    Color::TRANSPARENT
                })),
                text_color: if is_hovered {
                    SOOTMIX_DARK.semantic_error
                } else {
                    TEXT_DIM
                },
                border: Border::default().rounded(RADIUS_SM),
                ..button::Style::default()
            }
        })
        .on_press(Message::RemovePluginFromChannel(channel_id, instance_id));

    container(
        row![
            index_label,
            Space::new().width(SPACING_XS),
            name_label,
            Space::new().width(SPACING_XS),
            bypass_indicator,
            Space::new().width(Fill),
            edit_btn,
            Space::new().width(2),
            bypass_btn,
            Space::new().width(2),
            remove_btn,
        ]
        .align_y(Alignment::Center),
    )
    .padding([SPACING_XS, SPACING_SM])
    .style(|_| container::Style {
        background: Some(Background::Color(BACKGROUND)),
        border: Border::default()
            .rounded(RADIUS_SM)
            .color(SOOTMIX_DARK.border_subtle)
            .width(1.0),
        ..container::Style::default()
    })
    .into()
}

/// Output routing section.
fn output_section<'a>(
    channel: &'a MixerChannel,
    _available_outputs: &'a [OutputDevice],
) -> Element<'a, Message> {
    let section_title = text("Output")
        .size(TEXT_SMALL)
        .color(TEXT_DIM);

    let current_output = channel
        .output_device_name
        .as_deref()
        .unwrap_or("Default");

    let output_label = text(format!("Routing to: {}", current_output))
        .size(TEXT_SMALL)
        .color(TEXT);

    // Volume indicator
    let vol_text = text(format!("{:+.1} dB", channel.volume_db))
        .size(TEXT_BODY)
        .color(if channel.muted {
            SOOTMIX_DARK.semantic_error
        } else {
            SOOTMIX_DARK.accent_warm
        });

    let mute_status = if channel.muted { " (MUTED)" } else { "" };
    let mute_text = text(mute_status)
        .size(TEXT_SMALL)
        .color(SOOTMIX_DARK.semantic_error);

    column![
        section_title,
        Space::new().height(SPACING_XS),
        output_label,
        Space::new().height(SPACING_XS),
        row![vol_text, mute_text,].align_y(Alignment::Center),
    ]
    .into()
}

/// Quick actions section.
fn quick_actions_section<'a>(channel: &'a MixerChannel) -> Element<'a, Message> {
    let id = channel.id;

    // Mute button
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
    .width(Fill)
    .style(move |_: &Theme, status| {
        let is_hovered = matches!(status, button::Status::Hovered);
        let bg = if channel.muted {
            if is_hovered {
                SOOTMIX_DARK.semantic_error
            } else {
                SOOTMIX_DARK.semantic_error.scale_alpha(0.3)
            }
        } else if is_hovered {
            SURFACE_LIGHT
        } else {
            SURFACE
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: if channel.muted {
                TEXT
            } else {
                TEXT
            },
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

    // Delete button
    let delete_btn = button(
        text("Delete Channel")
            .size(TEXT_SMALL)
            .color(TEXT_DIM),
    )
    .padding([SPACING_SM, SPACING])
    .width(Fill)
    .style(|_: &Theme, status| {
        let is_hovered = matches!(status, button::Status::Hovered);
        button::Style {
            background: Some(Background::Color(if is_hovered {
                SOOTMIX_DARK.semantic_error.scale_alpha(0.2)
            } else {
                Color::TRANSPARENT
            })),
            text_color: if is_hovered {
                SOOTMIX_DARK.semantic_error
            } else {
                TEXT_DIM
            },
            border: Border::default()
                .rounded(RADIUS)
                .color(SOOTMIX_DARK.border_subtle)
                .width(1.0),
            ..button::Style::default()
        }
    })
    .on_press(Message::ChannelDeleted(id));

    column![
        mute_btn,
        Space::new().height(SPACING_SM),
        delete_btn,
    ]
    .into()
}

/// Create an empty state for when no channel is selected.
pub fn focus_panel_empty<'a>() -> Element<'a, Message> {
    let content = column![
        Space::new().height(Fill),
        text("Select a channel")
            .size(TEXT_BODY)
            .color(TEXT_DIM),
        text("to view details")
            .size(TEXT_SMALL)
            .color(TEXT_DIM),
        Space::new().height(Fill),
    ]
    .align_x(Alignment::Center)
    .padding(PADDING);

    container(content)
        .width(Length::Fixed(FOCUS_PANEL_WIDTH))
        .height(Fill)
        .center_x(Fill)
        .style(|_| container::Style {
            background: Some(Background::Color(SURFACE)),
            border: Border::default()
                .rounded(RADIUS)
                .color(SOOTMIX_DARK.border_subtle)
                .width(1.0),
            ..container::Style::default()
        })
        .into()
}
