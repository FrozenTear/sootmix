// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Channel strip UI component.

use crate::audio::types::OutputDevice;
use crate::message::Message;
use crate::state::{MeterDisplayState, MixerChannel};
use crate::ui::meter::vu_meter;
use crate::ui::plugin_chain::fx_button;
use crate::ui::theme::{self, *};
use iced::widget::{button, column, container, pick_list, row, slider, text, text_input, vertical_slider, Space};
use iced::{Alignment, Background, Border, Color, Element, Fill, Length, Theme};
use uuid::Uuid;

/// Create a channel strip widget for a mixer channel.
/// `editing` is Some((channel_id, current_text)) if this channel's name is being edited.
/// `has_active_snapshot` indicates whether there's an active snapshot to save to.
pub fn channel_strip<'a>(
    channel: &'a MixerChannel,
    dragging: Option<&'a (u32, String)>,
    editing: Option<&'a (Uuid, String)>,
    has_active_snapshot: bool,
) -> Element<'a, Message> {
    let id = channel.id;
    let volume_db = channel.volume_db;
    let muted = channel.muted;
    let eq_enabled = channel.eq_enabled;
    let name = channel.name.clone();
    let assigned_apps = channel.assigned_apps.clone();
    let is_drop_target = dragging.is_some();

    // Check if this channel is being edited
    let is_editing = editing.map(|(eid, _)| *eid == id).unwrap_or(false);

    // Channel name - either text input (editing) or clickable text
    let name_element: Element<Message> = if is_editing {
        let edit_value = editing.map(|(_, v)| v.clone()).unwrap_or_default();
        text_input("Channel name", &edit_value)
            .on_input(Message::ChannelNameEditChanged)
            .on_submit(Message::ChannelRenamed(id, edit_value.clone()))
            .size(13)
            .width(Length::Fill)
            .style(|_theme: &Theme, _status| text_input::Style {
                background: Background::Color(SURFACE_LIGHT),
                border: Border::default().rounded(BORDER_RADIUS_SMALL).color(PRIMARY).width(1.0),
                icon: TEXT,
                placeholder: TEXT_DIM,
                value: TEXT,
                selection: PRIMARY,
            })
            .into()
    } else {
        button(text(name.clone()).size(14).color(TEXT))
            .padding([2, 4])
            .style(|_theme: &Theme, status| {
                let is_hovered = matches!(status, button::Status::Hovered);
                button::Style {
                    background: Some(Background::Color(if is_hovered { SURFACE_LIGHT } else { Color::TRANSPARENT })),
                    text_color: TEXT,
                    border: Border::default().rounded(BORDER_RADIUS_SMALL),
                    ..button::Style::default()
                }
            })
            .on_press(Message::StartEditingChannelName(id))
            .into()
    };

    // EQ button
    let eq_button = button(text("EQ").size(11))
        .padding([4, 8])
        .style(move |theme: &Theme, status| {
            let bg_color = if eq_enabled {
                PRIMARY
            } else {
                SURFACE_LIGHT
            };
            button::Style {
                background: Some(Background::Color(bg_color)),
                text_color: TEXT,
                border: Border::default().rounded(BORDER_RADIUS_SMALL),
                ..button::Style::default()
            }
        })
        .on_press(Message::ChannelEqToggled(id));

    // Volume slider (vertical)
    let volume_slider = vertical_slider(-60.0..=12.0, volume_db, move |v| {
        Message::ChannelVolumeChanged(id, v)
    })
    .step(0.5)
    .height(VOLUME_SLIDER_HEIGHT)
    .on_release(Message::ChannelVolumeReleased(id))
    .style(move |theme: &Theme, status| {
        slider::Style {
            rail: slider::Rail {
                backgrounds: (
                    Background::Color(theme::db_to_color(volume_db)),
                    Background::Color(SLIDER_TRACK),
                ),
                width: 8.0,
                border: Border::default().rounded(4.0),
            },
            handle: slider::Handle {
                shape: slider::HandleShape::Rectangle {
                    width: 20,
                    border_radius: 3.0.into(),
                },
                background: Background::Color(if muted { MUTED_COLOR } else { TEXT }),
                border_width: 0.0,
                border_color: Color::TRANSPARENT,
            },
        }
    });

    // VU meter
    let meter = vu_meter(&channel.meter_display, VOLUME_SLIDER_HEIGHT);

    // Volume display
    let volume_text = text(theme::format_db(volume_db))
        .size(12)
        .color(if muted { TEXT_DIM } else { TEXT });

    // Mute button
    let mute_icon = if muted { "M" } else { "S" }; // M for muted, S for sound
    let mute_button = button(text(mute_icon).size(14))
        .padding([6, 10])
        .style(move |_theme: &Theme, _status| {
            let bg_color = if muted {
                MUTED_COLOR
            } else {
                SURFACE_LIGHT
            };
            button::Style {
                background: Some(Background::Color(bg_color)),
                text_color: TEXT,
                border: Border::default().rounded(BORDER_RADIUS_SMALL),
                ..button::Style::default()
            }
        })
        .on_press(Message::ChannelMuteToggled(id));

    // Save to snapshot button (checkmark) - only shown when there's an active snapshot
    let save_button: Element<Message> = if has_active_snapshot {
        button(text("✓").size(12))
            .padding([4, 8])
            .style(|_theme: &Theme, status| {
                let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: Some(Background::Color(if is_hovered { SUCCESS } else { SURFACE_LIGHT })),
                    text_color: if is_hovered { TEXT } else { SUCCESS },
                    border: Border::default().rounded(BORDER_RADIUS_SMALL),
                    ..button::Style::default()
                }
            })
            .on_press(Message::SaveChannelToSnapshot(id))
            .into()
    } else {
        Space::new().width(0).height(0).into()
    };

    // Assigned apps list or drop indicator
    let apps_list: Element<Message> = if is_drop_target {
        // Show assignment indicator when in assign mode
        column![
            text("+ Assign")
                .size(12)
                .color(ACCENT),
        ]
        .align_x(Alignment::Center)
        .into()
    } else if assigned_apps.is_empty() {
        text("No apps")
            .size(10)
            .color(TEXT_DIM)
            .into()
    } else {
        // Create clickable buttons for each assigned app
        let app_buttons: Vec<Element<Message>> = assigned_apps
            .iter()
            .take(3)
            .map(|app_id| {
                let app_id_clone = app_id.clone();
                button(
                    text(format!("× {}", truncate_string(app_id, 8)))
                        .size(9)
                )
                .padding([2, 4])
                .style(|_theme: &Theme, status| {
                    let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                    button::Style {
                        background: Some(Background::Color(if is_hovered { MUTED_COLOR } else { SURFACE_LIGHT })),
                        text_color: if is_hovered { TEXT } else { TEXT_DIM },
                        border: Border::default().rounded(BORDER_RADIUS_SMALL),
                        ..button::Style::default()
                    }
                })
                .on_press(Message::AppUnassigned(id, app_id_clone))
                .into()
            })
            .collect();

        column(app_buttons)
            .spacing(2)
            .align_x(Alignment::Center)
            .into()
    };

    // Delete button
    let delete_button = button(text("×").size(14))
        .padding([2, 6])
        .style(|theme: &Theme, status| {
            button::Style {
                background: Some(Background::Color(Color::TRANSPARENT)),
                text_color: TEXT_DIM,
                border: Border::default(),
                ..button::Style::default()
            }
        })
        .on_press(Message::ChannelDeleted(id));

    // Slider and meter row
    let slider_meter_row = row![
        volume_slider,
        Space::new().width(SPACING_SMALL),
        meter,
    ]
    .align_y(Alignment::Center);

    // FX button (plugin chain)
    let plugin_count = channel.plugin_chain.len();
    let fx_btn = fx_button(id, plugin_count);

    // Assemble the channel strip
    let content = column![
        // Header row with name and delete
        row![
            name_element,
            Space::new().width(Fill),
            delete_button,
        ]
        .align_y(Alignment::Center),
        Space::new().height(SPACING_SMALL),
        // EQ and FX buttons row
        row![
            eq_button,
            Space::new().width(SPACING_SMALL),
            fx_btn,
        ]
        .align_y(Alignment::Center),
        Space::new().height(SPACING),
        // Volume slider with meter
        container(slider_meter_row)
            .center_x(Fill),
        Space::new().height(SPACING_SMALL),
        // Volume display
        volume_text,
        Space::new().height(SPACING_SMALL),
        // Mute and save buttons row
        row![
            mute_button,
            Space::new().width(SPACING_SMALL),
            save_button,
        ]
        .align_y(Alignment::Center),
        Space::new().height(SPACING),
        // Apps list
        apps_list,
    ]
    .align_x(Alignment::Center)
    .padding(PADDING)
    .spacing(SPACING_SMALL);

    // Wrap in a styled container
    let strip_container = container(content)
        .width(CHANNEL_STRIP_WIDTH)
        .style(move |_theme: &Theme| {
            container::Style {
                background: Some(Background::Color(SURFACE)),
                border: Border::default()
                    .rounded(BORDER_RADIUS)
                    .color(if is_drop_target { PRIMARY } else { SURFACE_LIGHT })
                    .width(if is_drop_target { 2.0 } else { 1.0 }),
                ..container::Style::default()
            }
        });

    // When dragging an app, wrap in a button to accept drops
    if is_drop_target {
        button(strip_container)
            .padding(0)
            .style(|_theme: &Theme, status| {
                let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: Some(Background::Color(Color::TRANSPARENT)),
                    border: Border::default()
                        .rounded(BORDER_RADIUS)
                        .color(if is_hovered { ACCENT } else { PRIMARY })
                        .width(if is_hovered { 3.0 } else { 2.0 }),
                    shadow: if is_hovered {
                        iced::Shadow {
                            color: Color { a: 0.3, ..ACCENT },
                            offset: iced::Vector::new(0.0, 0.0),
                            blur_radius: 8.0,
                        }
                    } else {
                        iced::Shadow::default()
                    },
                    ..button::Style::default()
                }
            })
            .on_press(Message::DropAppOnChannel(id))
            .into()
    } else {
        strip_container.into()
    }
}

/// Create a master channel strip widget.
pub fn master_strip<'a>(
    volume_db: f32,
    muted: bool,
    available_outputs: &'a [OutputDevice],
    selected_output: Option<&'a str>,
    meter_display: &'a MeterDisplayState,
) -> Element<'a, Message> {
    // Title
    let title = text("Master")
        .size(14)
        .color(TEXT);

    // Volume slider
    let volume_slider = vertical_slider(-60.0..=12.0, volume_db, Message::MasterVolumeChanged)
        .step(0.5)
        .height(VOLUME_SLIDER_HEIGHT)
        .on_release(Message::MasterVolumeReleased)
        .style(move |_theme: &Theme, _status| {
            slider::Style {
                rail: slider::Rail {
                    backgrounds: (
                        Background::Color(theme::db_to_color(volume_db)),
                        Background::Color(SLIDER_TRACK),
                    ),
                    width: 8.0,
                    border: Border::default().rounded(4.0),
                },
                handle: slider::Handle {
                    shape: slider::HandleShape::Rectangle {
                        width: 20,
                        border_radius: 3.0.into(),
                    },
                    background: Background::Color(if muted { MUTED_COLOR } else { PRIMARY }),
                    border_width: 0.0,
                    border_color: Color::TRANSPARENT,
                },
            }
        });

    // VU meter
    let meter = vu_meter(meter_display, VOLUME_SLIDER_HEIGHT);

    // Slider and meter row
    let slider_meter_row = row![
        volume_slider,
        Space::new().width(SPACING_SMALL),
        meter,
    ]
    .align_y(Alignment::Center);

    // Volume display
    let volume_text = text(theme::format_db(volume_db))
        .size(12)
        .color(if muted { TEXT_DIM } else { TEXT });

    // Mute button
    let mute_icon = if muted { "M" } else { "S" };
    let mute_button = button(text(mute_icon).size(14))
        .padding([6, 10])
        .style(move |_theme: &Theme, _status| {
            let bg_color = if muted {
                MUTED_COLOR
            } else {
                PRIMARY
            };
            button::Style {
                background: Some(Background::Color(bg_color)),
                text_color: TEXT,
                border: Border::default().rounded(BORDER_RADIUS_SMALL),
                ..button::Style::default()
            }
        })
        .on_press(Message::MasterMuteToggled);

    // Output device picker
    let output_options: Vec<String> = available_outputs
        .iter()
        .map(|d| d.description.clone())
        .collect();

    let output_picker: Element<'a, Message> = if output_options.is_empty() {
        text("No outputs")
            .size(10)
            .color(TEXT_DIM)
            .into()
    } else {
        let selected = selected_output.map(|s| s.to_string());
        column![
            text("Output").size(10).color(TEXT_DIM),
            pick_list(
                output_options,
                selected,
                Message::OutputDeviceChanged,
            )
            .placeholder("Select...")
            .text_size(11)
            .padding([4, 8])
            .width(Length::Fixed(CHANNEL_STRIP_WIDTH - PADDING * 2.0))
            .style(|_theme: &Theme, _status| {
                pick_list::Style {
                    text_color: TEXT,
                    placeholder_color: TEXT_DIM,
                    handle_color: TEXT_DIM,
                    background: Background::Color(SURFACE_LIGHT),
                    border: Border::default()
                        .rounded(BORDER_RADIUS_SMALL)
                        .color(SURFACE_LIGHT)
                        .width(1.0),
                }
            }),
        ]
        .spacing(2)
        .into()
    };

    // Assemble
    let content = column![
        title,
        Space::new().height(SPACING),
        container(slider_meter_row).center_x(Fill),
        Space::new().height(SPACING_SMALL),
        volume_text,
        Space::new().height(SPACING_SMALL),
        mute_button,
        Space::new().height(SPACING),
        output_picker,
    ]
    .align_x(Alignment::Center)
    .padding(PADDING)
    .spacing(SPACING_SMALL);

    container(content)
        .width(CHANNEL_STRIP_WIDTH)
        .style(|_theme: &Theme| {
            container::Style {
                background: Some(Background::Color(SURFACE)),
                border: Border::default()
                    .rounded(BORDER_RADIUS)
                    .color(PRIMARY)
                    .width(2.0),
                ..container::Style::default()
            }
        })
        .into()
}

/// Truncate a string to max length with ellipsis.
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}
