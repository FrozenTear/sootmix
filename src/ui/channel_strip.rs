// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Channel strip UI component.
//!
//! Professional mixer channel strip with:
//! - Volume fader with dynamic level coloring
//! - Stereo VU meter
//! - Mute, EQ, and FX controls
//! - App assignment display
//! - Per-channel output routing
//! - Drop target for drag-and-drop routing

use crate::audio::types::{InputDevice, OutputDevice};
use crate::message::Message;
use crate::state::{ChannelKind, MeterDisplayState, MixerChannel};
use crate::ui::meter::vu_meter;
use crate::ui::plugin_chain::fx_button;
use crate::ui::theme::{self, *};
use iced::widget::{
    button, column, container, pick_list, row, slider, text, text_input, tooltip, vertical_slider,
    Space,
};
use iced::{Alignment, Background, Border, Color, Element, Fill, Length, Theme};
use uuid::Uuid;

// ============================================================================
// CHANNEL STRIP
// ============================================================================

/// Create a channel strip widget for a mixer channel.
///
/// # Arguments
/// * `channel` - The mixer channel data
/// * `dragging` - Currently dragged app (if any)
/// * `editing` - Channel being edited (id, current text)
/// * `has_active_snapshot` - Whether there's an active snapshot for save button
/// * `available_outputs` - List of available output devices
/// * `is_selected` - Whether this channel is currently selected for the focus panel
pub fn channel_strip<'a>(
    channel: &'a MixerChannel,
    editing: Option<&'a (Uuid, String)>,
    has_active_snapshot: bool,
    available_outputs: &'a [OutputDevice],
    available_inputs: &'a [InputDevice],
    is_selected: bool,
    is_first_in_group: bool,
    is_last_in_group: bool,
) -> Element<'a, Message> {
    let id = channel.id;
    let volume_db = channel.volume_db;
    let muted = channel.muted;
    let name = channel.name.clone();
    let output_device_name = channel.output_device_name.clone();
    let is_input = channel.kind == ChannelKind::Input;

    // Input channel accent color (cyan) vs output channel (default)
    let channel_accent = if is_input {
        SOOTMIX_DARK.accent_primary  // Cyan for inputs
    } else {
        TEXT  // Default for outputs
    };

    // Check if this channel is being edited
    let is_editing = editing.map(|(eid, _)| *eid == id).unwrap_or(false);

    // === CHANNEL NAME ===
    let name_element: Element<Message> = if is_editing {
        let edit_value = editing.map(|(_, v)| v.clone()).unwrap_or_default();
        text_input("Channel name", &edit_value)
            .on_input(Message::ChannelNameEditChanged)
            .on_submit(Message::ChannelRenamed(id, edit_value.clone()))
            .size(TEXT_BODY)
            .width(Length::Fill)
            .style(|_theme: &Theme, _status| text_input::Style {
                background: Background::Color(SURFACE_LIGHT),
                border: Border::default()
                    .rounded(RADIUS_SM)
                    .color(PRIMARY)
                    .width(2.0),
                icon: TEXT,
                placeholder: TEXT_DIM,
                value: TEXT,
                selection: PRIMARY,
            })
            .into()
    } else {
        button(text(name.clone()).size(TEXT_BODY).color(TEXT))
            .padding([SPACING_XS, SPACING_SM])
            .style(|_theme: &Theme, status| {
                let is_hovered = matches!(status, button::Status::Hovered);
                button::Style {
                    background: Some(Background::Color(if is_hovered {
                        SURFACE_LIGHT
                    } else {
                        Color::TRANSPARENT
                    })),
                    text_color: TEXT,
                    border: Border::default().rounded(RADIUS_SM),
                    ..button::Style::default()
                }
            })
            .on_press(Message::StartEditingChannelName(id))
            .into()
    };

    // === CHANNEL TYPE BADGE (for input channels) ===
    let type_badge: Element<Message> = if is_input {
        container(
            text("MIC").size(TEXT_CAPTION).color(SOOTMIX_DARK.canvas)
        )
        .padding([2.0, SPACING_XS])
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(SOOTMIX_DARK.accent_primary)),
            border: Border::default().rounded(RADIUS_SM),
            ..container::Style::default()
        })
        .into()
    } else {
        Space::new().width(0).height(0).into()
    };

    // === VOLUME SLIDER ===
    let volume_slider = vertical_slider(-60.0..=12.0, volume_db, move |v| {
        Message::ChannelVolumeChanged(id, v)
    })
    .step(0.5)
    .height(VOLUME_SLIDER_HEIGHT)
    .on_release(Message::ChannelVolumeReleased(id))
    .style(move |_theme: &Theme, _status| slider::Style {
        rail: slider::Rail {
            backgrounds: (
                Background::Color(theme::db_to_color(volume_db)),
                Background::Color(SLIDER_TRACK),
            ),
            width: 10.0,
            border: Border::default().rounded(5.0),
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Rectangle {
                width: 24,
                border_radius: RADIUS_SM.into(),
            },
            background: Background::Color(if muted { MUTED_COLOR } else { channel_accent }),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    });

    // === VU METER ===
    let meter = vu_meter(&channel.meter_display, VOLUME_SLIDER_HEIGHT);

    // === VOLUME DISPLAY ===
    let volume_text = container(
        text(theme::format_db(volume_db))
            .size(TEXT_SMALL)
            .color(if muted { TEXT_DIM } else { TEXT }),
    )
    .padding([SPACING_XS, SPACING_SM])
    .style(|_theme: &Theme| container::Style {
        background: Some(Background::Color(SURFACE)),
        border: Border::default().rounded(RADIUS_SM),
        ..container::Style::default()
    });

    // === MUTE BUTTON ===
    let mute_icon = if muted { "M" } else { "S" };
    let mute_button = button(text(mute_icon).size(TEXT_BODY))
        .padding([SPACING_SM, SPACING])
        .style(move |_theme: &Theme, status| {
            let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            let bg_color = if muted {
                if is_hovered {
                    lighten(MUTED_COLOR, 0.15)
                } else {
                    MUTED_COLOR
                }
            } else if is_hovered {
                SURFACE_LIGHT
            } else {
                SURFACE
            };
            button::Style {
                background: Some(Background::Color(bg_color)),
                text_color: TEXT,
                border: Border::default().rounded(RADIUS_SM),
                ..button::Style::default()
            }
        })
        .on_press(Message::ChannelMuteToggled(id));

    // === SAVE TO SNAPSHOT BUTTON ===
    let save_button: Element<Message> = if has_active_snapshot {
        button(text("\u{2713}").size(TEXT_SMALL)) // checkmark
            .padding([SPACING_XS, SPACING_SM])
            .style(|_theme: &Theme, status| {
                let is_hovered =
                    matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: Some(Background::Color(if is_hovered {
                        SUCCESS
                    } else {
                        SURFACE
                    })),
                    text_color: if is_hovered { TEXT } else { SUCCESS },
                    border: Border::default()
                        .rounded(RADIUS_SM)
                        .color(SUCCESS)
                        .width(1.0),
                    ..button::Style::default()
                }
            })
            .on_press(Message::SaveChannelToSnapshot(id))
            .into()
    } else {
        Space::new().width(0).height(0).into()
    };

    // === MOVE ARROW BUTTONS ===
    let arrow_style = |_theme: &Theme, status: button::Status| {
        let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if is_hovered {
                SURFACE_LIGHT
            } else {
                Color::TRANSPARENT
            })),
            text_color: if is_hovered { TEXT } else { TEXT_DIM },
            border: Border::default().rounded(RADIUS_SM),
            ..button::Style::default()
        }
    };

    let left_btn: Element<Message> = if !is_first_in_group {
        button(text("\u{25C0}").size(TEXT_SMALL).color(TEXT_DIM))
            .padding([SPACING_XS, SPACING_XS])
            .style(arrow_style)
            .on_press(Message::MoveChannelLeft(id))
            .into()
    } else {
        Space::new().width(0).height(0).into()
    };

    let right_btn: Element<Message> = if !is_last_in_group {
        button(text("\u{25B6}").size(TEXT_SMALL).color(TEXT_DIM))
            .padding([SPACING_XS, SPACING_XS])
            .style(arrow_style)
            .on_press(Message::MoveChannelRight(id))
            .into()
    } else {
        Space::new().width(0).height(0).into()
    };

    // === DELETE BUTTON ===
    let delete_button = button(
        text("\u{00D7}").size(TEXT_BODY + 2.0)
    )
        .padding([SPACING_XS, SPACING_SM])
        .style(|_theme: &Theme, status| {
            let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(Background::Color(if is_hovered {
                    Color {
                        a: 0.2,
                        ..MUTED_COLOR
                    }
                } else {
                    Color::TRANSPARENT
                })),
                text_color: if is_hovered { MUTED_COLOR } else { TEXT_DIM },
                border: Border::default().rounded(RADIUS_SM),
                ..button::Style::default()
            }
        })
        .on_press(Message::ChannelDeleted(id));

    // === SLIDER + METER ROW ===
    let slider_meter_row = row![volume_slider, Space::new().width(SPACING_SM), meter,]
        .align_y(Alignment::Center);

    // === FX BUTTON ===
    let plugin_count = channel.plugin_chain.len();
    let fx_btn = fx_button(id, plugin_count);

    // === DEVICE PICKER (Output for output channels, Input for input channels) ===
    let is_input = channel.kind == ChannelKind::Input;
    let max_display_chars = 12;

    let device_picker: Element<'a, Message> = if is_input {
        // Input channel: show input device picker + sidetone controls
        // Filter out the synthetic "system-default" entry - we'll add our own "Default" label
        let hw_inputs: Vec<_> = available_inputs
            .iter()
            .filter(|d| d.name != "system-default")
            .collect();

        let input_options: Vec<String> = std::iter::once("Default".to_string())
            .chain(hw_inputs.iter().map(|d| truncate_string(&d.description, max_display_chars)))
            .collect();

        // Map stored input_device_name to display label
        let selected_input = channel.input_device_name
            .clone()
            .map(|name| {
                if name == "system-default" {
                    "Default".to_string()
                } else {
                    hw_inputs
                        .iter()
                        .find(|d| d.description == name || d.name == name)
                        .map(|d| truncate_string(&d.description, max_display_chars))
                        .unwrap_or_else(|| truncate_string(&name, max_display_chars))
                }
            })
            .unwrap_or_else(|| "Default".to_string());

        let display_to_full: Vec<(String, String)> = hw_inputs
            .iter()
            .map(|d| (truncate_string(&d.description, max_display_chars), d.description.clone()))
            .collect();

        let sidetone_enabled = channel.sidetone_enabled;
        let sidetone_vol = channel.sidetone_volume_db;
        let noise_suppression_enabled = channel.noise_suppression_enabled;
        let input_gain = channel.input_gain_db;

        let input_picker = column![
            text("Input").size(TEXT_SMALL).color(TEXT_DIM),
            pick_list(input_options, Some(selected_input), move |selection: String| {
                let device = if selection == "Default" {
                    Some("system-default".to_string())
                } else {
                    let full_name = display_to_full
                        .iter()
                        .find(|(trunc, _)| *trunc == selection)
                        .map(|(_, full)| full.clone())
                        .unwrap_or(selection);
                    Some(full_name)
                };
                Message::ChannelInputDeviceChanged(id, device)
            })
                .text_size(TEXT_SMALL)
                .padding([SPACING_SM, SPACING_SM])
                .width(Length::Fixed(CHANNEL_STRIP_WIDTH - PADDING * 2.0))
                .style(|_theme: &Theme, _status| {
                    pick_list::Style {
                        text_color: TEXT,
                        placeholder_color: TEXT_DIM,
                        handle_color: SOOTMIX_DARK.text_muted,
                        background: Background::Color(SOOTMIX_DARK.surface_raised),
                        border: Border::default()
                            .rounded(RADIUS_SM)
                            .color(SOOTMIX_DARK.border_default)
                            .width(1.0),
                    }
                }),
        ]
        .spacing(SPACING_XS);

        // Mic Gain slider (hardware input level)
        let gain_label = format!("{:+.1}dB", input_gain);
        let mic_gain_section = column![
            text("Mic Gain").size(TEXT_SMALL).color(TEXT_DIM),
            row![
                slider(-12.0..=12.0, input_gain, move |v| {
                    Message::ChannelInputGainChanged(id, v)
                })
                .step(0.5)
                .width(Length::Fixed(CHANNEL_STRIP_WIDTH - PADDING * 2.0 - 50.0))
                .on_release(Message::ChannelInputGainReleased(id))
                .style(|_theme: &Theme, _status| slider::Style {
                    rail: slider::Rail {
                        backgrounds: (
                            Background::Color(SOOTMIX_DARK.accent_primary),
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
                }),
                Space::new().width(SPACING_XS),
                text(gain_label).size(TEXT_CAPTION).color(TEXT_DIM),
            ]
            .align_y(Alignment::Center),
        ]
        .spacing(SPACING_XS);

        // Sidetone toggle
        let sidetone_btn = button(text("MON").size(TEXT_SMALL))
            .padding([SPACING_XS, SPACING_SM])
            .style(move |_theme: &Theme, status| {
                let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                let bg_color = if sidetone_enabled {
                    if is_hovered { lighten(SUCCESS, 0.15) } else { SUCCESS }
                } else if is_hovered {
                    SURFACE_LIGHT
                } else {
                    SURFACE
                };
                button::Style {
                    background: Some(Background::Color(bg_color)),
                    text_color: if sidetone_enabled { SOOTMIX_DARK.canvas } else { TEXT },
                    border: Border::default()
                        .rounded(RADIUS_SM)
                        .color(if sidetone_enabled { SUCCESS } else { SURFACE_LIGHT })
                        .width(1.0),
                    ..button::Style::default()
                }
            })
            .on_press(Message::ChannelSidetoneToggled(id));

        // Noise suppression toggle (RNNoise)
        let ns_btn = button(text("NS").size(TEXT_SMALL))
            .padding([SPACING_XS, SPACING_SM])
            .style(move |_theme: &Theme, status| {
                let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                let bg_color = if noise_suppression_enabled {
                    if is_hovered { lighten(PRIMARY, 0.15) } else { PRIMARY }
                } else if is_hovered {
                    SURFACE_LIGHT
                } else {
                    SURFACE
                };
                button::Style {
                    background: Some(Background::Color(bg_color)),
                    text_color: if noise_suppression_enabled { SOOTMIX_DARK.canvas } else { TEXT },
                    border: Border::default()
                        .rounded(RADIUS_SM)
                        .color(if noise_suppression_enabled { PRIMARY } else { SURFACE_LIGHT })
                        .width(1.0),
                    ..button::Style::default()
                }
            })
            .on_press(Message::ChannelNoiseSuppressionToggled(id));

        let sidetone_row: Element<'a, Message> = if sidetone_enabled {
            let sidetone_slider = slider(-60.0..=0.0, sidetone_vol, move |v| {
                Message::ChannelSidetoneVolumeChanged(id, v)
            })
            .step(0.5)
            .width(Length::Fixed(CHANNEL_STRIP_WIDTH - PADDING * 2.0 - 90.0)) // Adjusted for NS button
            .style(|_theme: &Theme, _status| slider::Style {
                rail: slider::Rail {
                    backgrounds: (
                        Background::Color(SUCCESS),
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
            row![sidetone_btn, Space::new().width(SPACING_XS), ns_btn, Space::new().width(SPACING_SM), sidetone_slider]
                .align_y(Alignment::Center)
                .into()
        } else {
            row![sidetone_btn, Space::new().width(SPACING_XS), ns_btn]
                .align_y(Alignment::Center)
                .into()
        };

        column![
            input_picker,
            Space::new().height(SPACING_XS),
            mic_gain_section,
            Space::new().height(SPACING_XS),
            sidetone_row
        ]
            .into()
    } else {
        // Output channel: show output device picker
        let output_options: Vec<String> = std::iter::once("Default".to_string())
            .chain(
                available_outputs
                    .iter()
                    .filter(|d| d.name != "system-default")
                    .map(|d| truncate_string(&d.description, max_display_chars)),
            )
            .collect();

        let selected_output = output_device_name
            .clone()
            .map(|name| {
                if name == "system-default" {
                    "Default".to_string()
                } else {
                    available_outputs
                        .iter()
                        .find(|d| d.description == name || d.name == name)
                        .map(|d| truncate_string(&d.description, max_display_chars))
                        .unwrap_or_else(|| truncate_string(&name, max_display_chars))
                }
            })
            .or_else(|| Some("Default".to_string()));

        let has_hw_outputs = available_outputs.iter().any(|d| d.name != "system-default");
        if !has_hw_outputs {
            Space::new().width(0).height(0).into()
        } else {
            let display_to_full: Vec<(String, String)> = available_outputs
                .iter()
                .filter(|d| d.name != "system-default")
                .map(|d| (truncate_string(&d.description, max_display_chars), d.description.clone()))
                .collect();

            column![
                text("Output").size(TEXT_SMALL).color(TEXT_DIM),
                pick_list(output_options, selected_output, move |selection: String| {
                    let device = if selection == "Default" {
                        Some("system-default".to_string())
                    } else {
                        let full_name = display_to_full
                            .iter()
                            .find(|(trunc, _)| *trunc == selection)
                            .map(|(_, full)| full.clone())
                            .unwrap_or(selection);
                        Some(full_name)
                    };
                    Message::ChannelOutputDeviceChanged(id, device)
                },)
                    .placeholder("Default")
                    .text_size(TEXT_SMALL)
                    .padding([SPACING_SM, SPACING_SM])
                    .width(Length::Fixed(CHANNEL_STRIP_WIDTH - PADDING * 2.0))
                    .style(|_theme: &Theme, _status| {
                        pick_list::Style {
                            text_color: TEXT,
                            placeholder_color: TEXT,
                            handle_color: SOOTMIX_DARK.text_muted,
                            background: Background::Color(SOOTMIX_DARK.surface_raised),
                            border: Border::default()
                                .rounded(RADIUS_SM)
                                .color(SOOTMIX_DARK.border_default)
                                .width(1.0),
                        }
                    }),
            ]
            .spacing(SPACING_XS)
            .into()
        }
    };

    // === MOVE ARROWS ROW (centered, compact) ===
    let arrows_row: Element<Message> = if !is_first_in_group || !is_last_in_group {
        row![left_btn, right_btn]
            .spacing(SPACING_XS)
            .align_y(Alignment::Center)
            .into()
    } else {
        Space::new().width(0).height(0).into()
    };

    // === ASSEMBLE CHANNEL STRIP ===
    let content = column![
        // Header: type badge (if input) + name + delete
        row![type_badge, name_element, Space::new().width(Fill), delete_button,].align_y(Alignment::Center),
        // Move arrows (centered, own row)
        container(arrows_row).center_x(Fill),
        // Controls: EQ + FX
        fx_btn,
        Space::new().height(SPACING),
        // Fader section
        container(slider_meter_row).center_x(Fill),
        Space::new().height(SPACING_SM),
        // Volume readout
        container(volume_text).center_x(Fill),
        Space::new().height(SPACING_SM),
        // Mute + Save
        row![mute_button, Space::new().width(SPACING_SM), save_button,].align_y(Alignment::Center),
        Space::new().height(SPACING),
        // Device picker (output or input)
        device_picker,
    ]
    .align_x(Alignment::Center)
    .padding(PADDING)
    .spacing(SPACING_XS);

    // === CONTAINER STYLING ===
    let border_color = if is_selected {
        PRIMARY
    } else {
        SOOTMIX_DARK.border_subtle
    };
    let border_width = if is_selected { 2.0 } else { 1.0 };

    let strip_container = container(content)
        .width(CHANNEL_STRIP_WIDTH)
        .height(CHANNEL_STRIP_HEIGHT)
        .style(move |_theme: &Theme| container::Style {
            background: Some(Background::Color(if is_selected {
                // Slightly lighter background when selected
                SURFACE_LIGHT
            } else {
                SURFACE
            })),
            border: Border::default()
                .rounded(RADIUS)
                .color(border_color)
                .width(border_width),
            ..container::Style::default()
        });

    strip_container.into()
}

// ============================================================================
// MASTER STRIP
// ============================================================================

/// Create a master channel strip widget.
///
/// The master strip has a distinctive styling (golden border) and includes
/// the output device selector.
pub fn master_strip<'a>(
    volume_db: f32,
    muted: bool,
    available_outputs: &'a [OutputDevice],
    selected_output: Option<&'a str>,
    meter_display: &'a MeterDisplayState,
    recording_enabled: bool,
) -> Element<'a, Message> {
    // === TITLE ===
    let title = container(
        text("MASTER")
            .size(TEXT_BODY)
            .color(SOOTMIX_DARK.canvas),
    )
    .padding([SPACING_XS, SPACING_SM])
    .style(|_theme: &Theme| container::Style {
        background: Some(Background::Color(SOOTMIX_DARK.accent_warm)),
        border: Border::default().rounded(RADIUS_SM),
        ..container::Style::default()
    });

    // === VOLUME SLIDER ===
    let volume_slider = vertical_slider(-60.0..=12.0, volume_db, Message::MasterVolumeChanged)
        .step(0.5)
        .height(VOLUME_SLIDER_HEIGHT)
        .on_release(Message::MasterVolumeReleased)
        .style(move |_theme: &Theme, _status| slider::Style {
            rail: slider::Rail {
                backgrounds: (
                    Background::Color(theme::db_to_color(volume_db)),
                    Background::Color(SLIDER_TRACK),
                ),
                width: 10.0,
                border: Border::default().rounded(5.0),
            },
            handle: slider::Handle {
                shape: slider::HandleShape::Rectangle {
                    width: 24,
                    border_radius: RADIUS_SM.into(),
                },
                background: Background::Color(if muted {
                    MUTED_COLOR
                } else {
                    SOOTMIX_DARK.accent_warm
                }),
                border_width: 0.0,
                border_color: Color::TRANSPARENT,
            },
        });

    // === VU METER ===
    let meter = vu_meter(meter_display, VOLUME_SLIDER_HEIGHT);

    // === SLIDER + METER ROW ===
    let slider_meter_row = row![volume_slider, Space::new().width(SPACING_SM), meter,]
        .align_y(Alignment::Center);

    // === VOLUME DISPLAY ===
    let volume_text = container(
        text(theme::format_db(volume_db))
            .size(TEXT_SMALL)
            .color(if muted { TEXT_DIM } else { TEXT }),
    )
    .padding([SPACING_XS, SPACING_SM])
    .style(|_theme: &Theme| container::Style {
        background: Some(Background::Color(SURFACE)),
        border: Border::default().rounded(RADIUS_SM),
        ..container::Style::default()
    });

    // === MUTE BUTTON ===
    let mute_icon = if muted { "M" } else { "S" };
    let mute_button = button(text(mute_icon).size(TEXT_BODY))
        .padding([SPACING_SM, SPACING])
        .style(move |_theme: &Theme, status| {
            let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            let bg_color = if muted {
                if is_hovered {
                    lighten(MUTED_COLOR, 0.15)
                } else {
                    MUTED_COLOR
                }
            } else if is_hovered {
                lighten(SOOTMIX_DARK.accent_warm, 0.1)
            } else {
                SOOTMIX_DARK.accent_warm
            };
            button::Style {
                background: Some(Background::Color(bg_color)),
                text_color: if muted { TEXT } else { SOOTMIX_DARK.canvas },
                border: Border::default().rounded(RADIUS_SM),
                ..button::Style::default()
            }
        })
        .on_press(Message::MasterMuteToggled);

    // === RECORDING TOGGLE ===
    let rec_label = if recording_enabled { "REC" } else { "rec" };
    let recording_button = button(text(rec_label).size(TEXT_SMALL))
        .padding([SPACING_XS, SPACING_SM])
        .style(move |_theme: &Theme, status| {
            let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            let bg_color = if recording_enabled {
                if is_hovered {
                    lighten(MUTED_COLOR, 0.15)
                } else {
                    MUTED_COLOR
                }
            } else if is_hovered {
                SURFACE_LIGHT
            } else {
                SURFACE
            };
            button::Style {
                background: Some(Background::Color(bg_color)),
                text_color: if recording_enabled { TEXT } else { TEXT_DIM },
                border: Border::default()
                    .rounded(RADIUS_SM)
                    .color(if recording_enabled { MUTED_COLOR } else { SURFACE_LIGHT })
                    .width(1.0),
                ..button::Style::default()
            }
        })
        .on_press(Message::ToggleMasterRecording);

    // === OUTPUT DEVICE PICKER ===
    // Build display labels and a mapping to the name/sentinel to send.
    // Filter out the synthetic "system-default" entry since we add it manually.
    let max_display_chars = 12;
    let hw_outputs: Vec<&OutputDevice> = available_outputs
        .iter()
        .filter(|d| d.name != "system-default")
        .collect();

    let output_options: Vec<String> = std::iter::once("System Default".to_string())
        .chain(hw_outputs.iter().map(|d| truncate_string(&d.description, max_display_chars)))
        .collect();

    // Map the selected_output (stored as name/sentinel) to the display label
    let selected = selected_output.map(|s| {
        if s == "system-default" {
            "System Default".to_string()
        } else {
            // Match by name or description to find the display label (truncated)
            hw_outputs
                .iter()
                .find(|d| d.name == s || d.description == s)
                .map(|d| truncate_string(&d.description, max_display_chars))
                .unwrap_or_else(|| truncate_string(s, max_display_chars))
        }
    });

    let has_any_outputs = !hw_outputs.is_empty();
    let output_picker: Element<'a, Message> = if !has_any_outputs && selected.is_none() {
        text("No outputs").size(TEXT_SMALL).color(TEXT_DIM).into()
    } else {
        let outputs_for_closure: Vec<(String, String)> = hw_outputs
            .iter()
            .map(|d| (truncate_string(&d.description, max_display_chars), d.name.clone()))
            .collect();
        column![
            text("Output").size(TEXT_SMALL).color(TEXT_DIM),
            pick_list(
                output_options,
                selected,
                move |selection: String| {
                    if selection == "System Default" {
                        Message::OutputDeviceChanged("system-default".to_string())
                    } else {
                        // Map truncated display name back to device name
                        let device_name = outputs_for_closure
                            .iter()
                            .find(|(trunc, _)| *trunc == selection)
                            .map(|(_, name)| name.clone())
                            .unwrap_or(selection);
                        Message::OutputDeviceChanged(device_name)
                    }
                },
            )
            .placeholder("Select...")
            .text_size(TEXT_SMALL)
            .padding([SPACING_SM, SPACING_SM])
            .width(Length::Fixed(CHANNEL_STRIP_WIDTH - PADDING * 2.0))
            .style(|_theme: &Theme, _status| {
                pick_list::Style {
                    text_color: TEXT,
                    placeholder_color: TEXT_DIM,
                    handle_color: SOOTMIX_DARK.text_muted,
                    background: Background::Color(SOOTMIX_DARK.surface_raised),
                    border: Border::default()
                        .rounded(RADIUS_SM)
                        .color(SOOTMIX_DARK.border_default)
                        .width(1.0),
                }
            }),
        ]
        .spacing(SPACING_XS)
        .into()
    };

    // === ASSEMBLE ===
    let content = column![
        container(title).center_x(Fill),
        Space::new().height(SPACING),
        container(slider_meter_row).center_x(Fill),
        Space::new().height(SPACING_SM),
        container(volume_text).center_x(Fill),
        Space::new().height(SPACING_SM),
        container(row![mute_button, Space::new().width(SPACING_SM), recording_button].align_y(Alignment::Center))
            .center_x(Fill),
        Space::new().height(SPACING),
        output_picker,
    ]
    .align_x(Alignment::Center)
    .padding(PADDING)
    .spacing(SPACING_XS);

    container(content)
        .width(CHANNEL_STRIP_WIDTH)
        .height(CHANNEL_STRIP_HEIGHT)
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(SURFACE)),
            border: Border::default()
                .rounded(RADIUS)
                .color(SOOTMIX_DARK.accent_warm)
                .width(2.0),
            ..container::Style::default()
        })
        .into()
}

// ============================================================================
// APP CARD
// ============================================================================

/// Render a compact icon grid showing which apps are assigned to this channel.
///
/// Displayed below the channel strip, aligned to the same width.
/// Each app is shown as a small colored tile with initials; hover for full name,
/// click to unassign.
/// Height of the app card when empty (text + padding).
const APP_CARD_MIN_HEIGHT: f32 = 48.0;
/// Tile size for app icons.
const APP_TILE_SIZE: f32 = 32.0;
/// Number of app icon tiles per row.
const APP_ICONS_PER_ROW: usize = 3;


pub fn app_card(channel: &MixerChannel) -> Element<'_, Message> {
    let id = channel.id;
    let assigned_apps = &channel.assigned_apps;

    // Input channels show recording destinations instead of apps
    if channel.kind == ChannelKind::Input {
        return input_channel_card(channel);
    }

    let (content, card_height): (Element<Message>, f32) = if assigned_apps.is_empty() {
        (
            text("No apps")
                .size(TEXT_CAPTION)
                .color(TEXT_DIM)
                .into(),
            APP_CARD_MIN_HEIGHT,
        )
    } else {
        let total = assigned_apps.len();
        let num_rows = (total + APP_ICONS_PER_ROW - 1) / APP_ICONS_PER_ROW;
        let mut grid_rows: Vec<Element<Message>> = Vec::new();
        let mut i = 0;
        while i < total {
            let end = (i + APP_ICONS_PER_ROW).min(total);
            let row_tiles: Vec<Element<Message>> = assigned_apps[i..end]
                .iter()
                .map(|app_id| app_icon_tile(id, app_id))
                .collect();
            grid_rows.push(row(row_tiles).spacing(SPACING_XS).into());
            i = end;
        }

        // tile rows + spacing between rows + padding top/bottom
        let h = (num_rows as f32 * APP_TILE_SIZE)
            + ((num_rows.saturating_sub(1)) as f32 * SPACING_XS)
            + PADDING_COMPACT * 2.0;

        (column(grid_rows).spacing(SPACING_XS).into(), h)
    };

    container(
        column![content]
            .padding(PADDING_COMPACT)
            .spacing(SPACING_XS),
    )
    .width(CHANNEL_STRIP_WIDTH)
    .height(card_height)
    .style(|_theme: &Theme| container::Style {
        background: Some(Background::Color(SURFACE)),
        border: Border::default()
            .rounded(RADIUS_SM)
            .color(SOOTMIX_DARK.border_subtle)
            .width(1.0),
        ..container::Style::default()
    })
    .into()
}

/// Card shown below input channels (instead of app list).
/// Shows input status and recording destinations.
fn input_channel_card(channel: &MixerChannel) -> Element<'static, Message> {
    let input_status = if let Some(ref device_name) = channel.input_device_name {
        // Truncate long device names
        let display_name = if device_name.len() > 20 {
            format!("{}...", &device_name[..17])
        } else {
            device_name.clone()
        };
        column![
            text("🎤").size(16.0),
            text(display_name)
                .size(TEXT_CAPTION)
                .color(TEXT_DIM),
        ]
        .spacing(SPACING_XS)
        .align_x(Alignment::Center)
    } else {
        column![
            text("🎤").size(16.0).color(TEXT_DIM),
            text("No input")
                .size(TEXT_CAPTION)
                .color(TEXT_DIM),
        ]
        .spacing(SPACING_XS)
        .align_x(Alignment::Center)
    };

    container(
        column![input_status]
            .padding(PADDING_COMPACT)
            .align_x(Alignment::Center),
    )
    .width(CHANNEL_STRIP_WIDTH)
    .height(APP_CARD_MIN_HEIGHT)
    .center_x(Length::Fill)
    .center_y(APP_CARD_MIN_HEIGHT)
    .style(|_theme: &Theme| container::Style {
        background: Some(Background::Color(SURFACE)),
        border: Border::default()
            .rounded(RADIUS_SM)
            .color(SOOTMIX_DARK.border_subtle)
            .width(1.0),
        ..container::Style::default()
    })
    .into()
}

/// A single app icon tile: colored square with 2-char initials.
fn app_icon_tile(channel_id: Uuid, app_id: &str) -> Element<'_, Message> {
    let initials = app_initials(app_id);
    let color = app_color(app_id);
    let app_id_owned = app_id.to_string();
    let display_name = app_id.to_string();

    let tile_size: f32 = 32.0;

    let icon = button(
        container(
            text(initials)
                .size(TEXT_CAPTION)
                .color(TEXT)
                .center(),
        )
        .center(tile_size),
    )
    .width(tile_size)
    .height(tile_size)
    .padding(0)
    .style(move |_theme: &Theme, status| {
        let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if is_hovered {
                Color {
                    a: 0.5,
                    ..MUTED_COLOR
                }
            } else {
                color
            })),
            text_color: TEXT,
            border: Border::default().rounded(RADIUS_SM),
            ..button::Style::default()
        }
    })
    .on_press(Message::AppUnassigned(channel_id, app_id_owned));

    tooltip(
        icon,
        container(
            text(display_name).size(TEXT_CAPTION).color(TEXT),
        )
        .padding([SPACING_XS, SPACING_SM])
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(SOOTMIX_DARK.surface_overlay)),
            border: Border::default()
                .rounded(RADIUS_SM)
                .color(SOOTMIX_DARK.border_default)
                .width(1.0),
            ..container::Style::default()
        }),
        tooltip::Position::Top,
    )
    .gap(4)
    .into()
}

/// Get 2-character initials from an app identifier.
fn app_initials(app_id: &str) -> String {
    let cleaned = app_id
        .replace('-', " ")
        .replace('_', " ")
        .replace('.', " ");
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    match words.len() {
        0 => "??".to_string(),
        1 => {
            let w = words[0];
            let mut chars = w.chars();
            let first = chars.next().unwrap_or('?');
            let second = chars.next().unwrap_or(' ');
            format!("{}{}", first, second).to_uppercase()
        }
        _ => {
            let a = words[0].chars().next().unwrap_or('?');
            let b = words[1].chars().next().unwrap_or('?');
            format!("{}{}", a, b).to_uppercase()
        }
    }
}

/// Generate a deterministic muted color from an app identifier.
fn app_color(app_id: &str) -> Color {
    let hash: u32 = app_id.bytes().fold(5381u32, |h, b| h.wrapping_mul(33).wrapping_add(b as u32));
    let hue = (hash % 360) as f32;
    // Convert HSL (hue, 0.35 saturation, 0.25 lightness) to RGB for a muted dark tone
    hsl_to_color(hue, 0.35, 0.25)
}

/// Convert HSL to iced Color.
fn hsl_to_color(h: f32, s: f32, l: f32) -> Color {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h2 = h / 60.0;
    let x = c * (1.0 - (h2 % 2.0 - 1.0).abs());
    let (r1, g1, b1) = if h2 < 1.0 {
        (c, x, 0.0)
    } else if h2 < 2.0 {
        (x, c, 0.0)
    } else if h2 < 3.0 {
        (0.0, c, x)
    } else if h2 < 4.0 {
        (0.0, x, c)
    } else if h2 < 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    let m = l - c / 2.0;
    Color::from_rgb(r1 + m, g1 + m, b1 + m)
}

// ============================================================================
// HELPERS
// ============================================================================

/// Truncate a string to max length with ellipsis.
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

/// Lighten a color by a factor (0.0-1.0).
fn lighten(color: Color, factor: f32) -> Color {
    Color::from_rgb(
        (color.r + (1.0 - color.r) * factor).min(1.0),
        (color.g + (1.0 - color.g) * factor).min(1.0),
        (color.b + (1.0 - color.b) * factor).min(1.0),
    )
}
