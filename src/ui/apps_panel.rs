// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Apps panel UI component - shows available audio applications.
//!
//! Displays running audio applications that can be routed to mixer channels.
//! Supports drag-and-drop assignment workflow.

use crate::message::Message;
use crate::state::{AppInfo, MixerChannel};
use crate::ui::theme::*;
use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Background, Border, Color, Element, Fill, Theme};
use std::collections::HashMap;

/// Create the apps panel showing available audio applications.
pub fn apps_panel<'a>(
    apps: &'a [AppInfo],
    channels: &'a [MixerChannel],
    dragging: Option<&(u32, String)>,
) -> Element<'a, Message> {
    let is_dragging = dragging.is_some();

    // === HEADER ===
    let title = text("Audio Apps").size(TEXT_BODY).color(TEXT);

    // Count unique apps (deduplicated)
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for app in apps {
        seen_ids.insert(app.identifier());
    }
    let unique_count = seen_ids.len();

    let status_text = if is_dragging {
        text("Select a channel").size(TEXT_SMALL).color(PRIMARY)
    } else {
        text(format!("{} active", unique_count))
            .size(TEXT_SMALL)
            .color(TEXT_DIM)
    };

    let header = row![title, Space::new().width(SPACING_SM), status_text,].align_y(Alignment::Center);

    // === APP ITEMS ===
    // Deduplicate apps by identifier - group all streams from the same app together
    let mut app_groups: HashMap<String, (Vec<&AppInfo>, usize)> = HashMap::new();
    for app in apps {
        let id = app.identifier();
        app_groups
            .entry(id)
            .or_insert_with(|| (Vec::new(), 0))
            .0
            .push(app);
    }
    for group in app_groups.values_mut() {
        group.1 = group.0.len();
    }

    // Convert to sorted list (by first occurrence order, roughly)
    let mut deduplicated: Vec<_> = app_groups.into_iter().collect();
    deduplicated.sort_by_key(|(_, (apps, _))| apps.first().map(|a| a.node_id).unwrap_or(0));

    let app_items: Vec<Element<Message>> = deduplicated
        .iter()
        .filter_map(|(_, (apps, stream_count))| {
            apps.first().map(|app| app_item(app, *stream_count, channels, dragging))
        })
        .collect();

    let apps_content: Element<Message> = if app_items.is_empty() {
        container(text("No audio apps playing").size(TEXT_SMALL).color(TEXT_DIM))
            .padding(PADDING)
            .center_x(Fill)
            .into()
    } else {
        scrollable(row(app_items).spacing(SPACING_SM))
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::default().width(4).scroller_width(4),
            ))
            .into()
    };

    // === CANCEL BUTTON (during drag) ===
    let cancel_area: Element<Message> = if is_dragging {
        button(text("Cancel").size(TEXT_SMALL).color(TEXT_DIM))
            .padding([SPACING_XS, SPACING_SM])
            .style(|_theme: &Theme, status| {
                let is_hovered =
                    matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: Some(Background::Color(if is_hovered {
                        SURFACE_LIGHT
                    } else {
                        SURFACE
                    })),
                    text_color: TEXT_DIM,
                    border: Border::default().rounded(RADIUS_SM),
                    ..button::Style::default()
                }
            })
            .on_press(Message::CancelDrag)
            .into()
    } else {
        Space::new().width(0).into()
    };

    // === ASSEMBLE ===
    let content = column![
        row![header, Space::new().width(Fill), cancel_area].align_y(Alignment::Center),
        Space::new().height(SPACING_SM),
        apps_content,
    ]
    .padding(PADDING);

    container(content)
        .width(Fill)
        .style(move |_theme: &Theme| container::Style {
            background: Some(Background::Color(SURFACE)),
            border: Border::default()
                .rounded(RADIUS)
                .color(if is_dragging {
                    PRIMARY
                } else {
                    SOOTMIX_DARK.border_subtle
                })
                .width(if is_dragging { 2.0 } else { 1.0 }),
            ..container::Style::default()
        })
        .into()
}

/// Create a single app item widget.
fn app_item<'a>(
    app: &'a AppInfo,
    stream_count: usize,
    channels: &'a [MixerChannel],
    dragging: Option<&(u32, String)>,
) -> Element<'a, Message> {
    let app_id = app.identifier();
    let node_id = app.node_id;
    let _ = stream_count;

    // Check if this app is currently being dragged
    let is_being_dragged = dragging.map(|(_, id)| id == &app_id).unwrap_or(false);

    // Check if app is assigned to any channel
    let assigned_channel = channels
        .iter()
        .find(|c| c.assigned_apps.iter().any(|a| a == &app_id));

    // Use media_name for display only for generic Chromium/Electron apps
    // (e.g., show "YouTube Music" instead of "Chromium").
    // For distinctive apps (Zen, Firefox, etc.), use the app name directly.
    let binary = app.binary.as_deref().unwrap_or("");
    let is_generic = crate::state::is_generic_app_identity(&app.name, binary);
    let raw_name = if is_generic {
        app.media_name
            .as_deref()
            .filter(|m| !m.is_empty() && !matches!(*m, "Playback" | "Audio Stream" | "audio-volume-change" | "AudioStream"))
            .unwrap_or(&app.name)
    } else {
        &app.name
    };
    let display_name = if app.stream_index > 0 {
        format!("{} #{}", clean_app_name(raw_name), app.stream_index)
    } else {
        clean_app_name(raw_name)
    };
    let is_assigned = assigned_channel.is_some();

    let name_text = text(display_name)
        .size(TEXT_SMALL)
        .color(if is_being_dragged { TEXT } else { TEXT });

    // Status indicator
    let status: Element<Message> = if is_being_dragged {
        text("Selected").size(TEXT_CAPTION).color(PRIMARY).into()
    } else if let Some(channel) = assigned_channel {
        row![
            text("\u{2192}").size(TEXT_CAPTION).color(SUCCESS), // arrow
            Space::new().width(SPACING_XS),
            text(truncate_string(&channel.name, 8))
                .size(TEXT_CAPTION)
                .color(SUCCESS),
        ]
        .align_y(Alignment::Center)
        .into()
    } else {
        text("Click to assign")
            .size(TEXT_CAPTION)
            .color(TEXT_DIM)
            .into()
    };

    let content = column![name_text, Space::new().height(SPACING_XS), status,]
        .align_x(Alignment::Center);

    let app_id_for_drop = app_id.clone();
    let app_id_for_click = app_id.clone();

    let tile = container(content)
        .padding([SPACING_SM, SPACING])
        .width(100)
        .style(move |_theme: &Theme| {
            container::Style {
                background: Some(Background::Color(if is_being_dragged {
                    PRIMARY
                } else if is_assigned {
                    SURFACE
                } else {
                    BACKGROUND
                })),
                text_color: if is_being_dragged {
                    SOOTMIX_DARK.canvas.into()
                } else {
                    TEXT.into()
                },
                border: Border::default()
                    .rounded(RADIUS_SM)
                    .color(if is_being_dragged {
                        PRIMARY
                    } else if is_assigned {
                        SUCCESS
                    } else {
                        SOOTMIX_DARK.border_subtle
                    })
                    .width(1.0),
                shadow: if is_being_dragged {
                    iced::Shadow {
                        color: Color { a: 0.3, ..PRIMARY },
                        offset: iced::Vector::new(0.0, 2.0),
                        blur_radius: 8.0,
                    }
                } else {
                    iced::Shadow::default()
                },
                ..container::Style::default()
            }
        });

    iced_drop::droppable(tile)
        .on_drop(move |point, rect| Message::DropApp(node_id, app_id_for_drop.clone(), point, rect))
        .on_click(Message::StartDraggingApp(node_id, app_id_for_click))
        .drag_overlay(true)
        .into()
}

/// Clean up app name for display.
fn clean_app_name(name: &str) -> String {
    let cleaned = name
        .replace(" (Virtual Sink)", "")
        .trim()
        .to_string();

    truncate_string(&cleaned, 16)
}

/// Truncate a string to max length with ellipsis.
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        s.chars()
            .take(max_len.saturating_sub(2))
            .collect::<String>()
            + ".."
    }
}
