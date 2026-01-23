// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Channel strip UI component.

use crate::message::Message;
use crate::state::MixerChannel;
use crate::ui::theme::{self, *};
use iced::widget::{button, column, container, row, slider, text, vertical_slider, Space};
use iced::{Alignment, Background, Border, Element, Fill, Length, Theme};
use uuid::Uuid;

/// Create a channel strip widget for a mixer channel.
pub fn channel_strip(channel: &MixerChannel) -> Element<'static, Message> {
    let id = channel.id;
    let volume_db = channel.volume_db;
    let muted = channel.muted;
    let eq_enabled = channel.eq_enabled;
    let name = channel.name.clone();
    let assigned_apps = channel.assigned_apps.clone();

    // Channel name
    let name_text = text(&name)
        .size(14)
        .color(TEXT);

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
    .height(VOLUME_SLIDER_HEIGHT as u16)
    .on_release(Message::ChannelVolumeReleased(id))
    .style(move |theme: &Theme, status| {
        slider::Style {
            rail: slider::Rail {
                colors: (
                    theme::db_to_color(volume_db),
                    SLIDER_TRACK,
                ),
                width: 8.0,
                border_radius: 4.0.into(),
            },
            handle: slider::Handle {
                shape: slider::HandleShape::Rectangle {
                    width: 20,
                    border_radius: 3.0.into(),
                },
                color: if muted { MUTED_COLOR } else { TEXT },
                border_width: 0.0,
                border_color: Color::TRANSPARENT,
            },
        }
    });

    // Volume display
    let volume_text = text(theme::format_db(volume_db))
        .size(12)
        .color(if muted { TEXT_DIM } else { TEXT });

    // Mute button
    let mute_icon = if muted { "M" } else { "S" }; // M for muted, S for sound
    let mute_button = button(text(mute_icon).size(14))
        .padding([6, 10])
        .style(move |theme: &Theme, status| {
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

    // Assigned apps list
    let apps_list: Element<Message> = if assigned_apps.is_empty() {
        text("No apps")
            .size(10)
            .color(TEXT_DIM)
            .into()
    } else {
        let apps_text = assigned_apps
            .iter()
            .take(3)
            .map(|a| format!("• {}", truncate_string(a, 10)))
            .collect::<Vec<_>>()
            .join("\n");
        text(apps_text)
            .size(10)
            .color(TEXT_DIM)
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

    // Assemble the channel strip
    let content = column![
        // Header row with name and delete
        row![
            name_text,
            Space::with_width(Fill),
            delete_button,
        ]
        .align_y(Alignment::Center),
        Space::with_height(SPACING_SMALL),
        // EQ button
        eq_button,
        Space::with_height(SPACING),
        // Volume slider
        container(volume_slider)
            .center_x(Fill),
        Space::with_height(SPACING_SMALL),
        // Volume display
        volume_text,
        Space::with_height(SPACING_SMALL),
        // Mute button
        mute_button,
        Space::with_height(SPACING),
        // Apps list
        apps_list,
    ]
    .align_x(Alignment::Center)
    .padding(PADDING)
    .spacing(SPACING_SMALL);

    // Wrap in a styled container
    container(content)
        .width(CHANNEL_STRIP_WIDTH as u16)
        .style(|theme: &Theme| {
            container::Style {
                background: Some(Background::Color(SURFACE)),
                border: standard_border(),
                ..container::Style::default()
            }
        })
        .into()
}

/// Create a master channel strip widget.
pub fn master_strip(volume_db: f32, muted: bool, output_device: Option<&str>) -> Element<'static, Message> {
    // Title
    let title = text("Master")
        .size(14)
        .color(TEXT);

    // Volume slider
    let volume_slider = vertical_slider(-60.0..=12.0, volume_db, Message::MasterVolumeChanged)
        .step(0.5)
        .height(VOLUME_SLIDER_HEIGHT as u16)
        .on_release(Message::MasterVolumeReleased)
        .style(move |theme: &Theme, status| {
            slider::Style {
                rail: slider::Rail {
                    colors: (
                        theme::db_to_color(volume_db),
                        SLIDER_TRACK,
                    ),
                    width: 8.0,
                    border_radius: 4.0.into(),
                },
                handle: slider::Handle {
                    shape: slider::HandleShape::Rectangle {
                        width: 20,
                        border_radius: 3.0.into(),
                    },
                    color: if muted { MUTED_COLOR } else { PRIMARY },
                    border_width: 0.0,
                    border_color: Color::TRANSPARENT,
                },
            }
        });

    // Volume display
    let volume_text = text(theme::format_db(volume_db))
        .size(12)
        .color(if muted { TEXT_DIM } else { TEXT });

    // Mute button
    let mute_icon = if muted { "M" } else { "S" };
    let mute_button = button(text(mute_icon).size(14))
        .padding([6, 10])
        .style(move |theme: &Theme, status| {
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

    // Output device display
    let output_text = text(format!(
        "Output:\n{}",
        truncate_string(output_device.unwrap_or("Default"), 12)
    ))
    .size(10)
    .color(TEXT_DIM);

    // Assemble
    let content = column![
        title,
        Space::with_height(SPACING),
        container(volume_slider).center_x(Fill),
        Space::with_height(SPACING_SMALL),
        volume_text,
        Space::with_height(SPACING_SMALL),
        mute_button,
        Space::with_height(SPACING),
        output_text,
    ]
    .align_x(Alignment::Center)
    .padding(PADDING)
    .spacing(SPACING_SMALL);

    container(content)
        .width(CHANNEL_STRIP_WIDTH as u16)
        .style(|theme: &Theme| {
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
