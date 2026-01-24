// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Apps panel UI component - shows available audio applications.

use crate::message::Message;
use crate::state::{AppInfo, MixerChannel};
use crate::ui::theme::*;
use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Background, Border, Element, Fill, Theme};

/// Create the apps panel showing available audio applications.
pub fn apps_panel<'a>(
    apps: &'a [AppInfo],
    channels: &'a [MixerChannel],
    dragging: Option<&(u32, String)>,
) -> Element<'a, Message> {
    let is_dragging = dragging.is_some();

    let title = text("Audio Apps")
        .size(13)
        .color(TEXT);

    let status_text = if is_dragging {
        text("Select a channel")
            .size(11)
            .color(PRIMARY)
    } else {
        text(format!("{} active", apps.len()))
            .size(11)
            .color(TEXT_DIM)
    };

    let header = row![
        title,
        Space::new().width(8),
        status_text,
    ]
    .align_y(Alignment::Center);

    // Build app items
    let app_items: Vec<Element<Message>> = apps
        .iter()
        .map(|app| app_item(app, channels, dragging))
        .collect();

    let apps_content: Element<Message> = if app_items.is_empty() {
        container(
            text("No audio apps playing")
                .size(12)
                .color(TEXT_DIM),
        )
        .padding(PADDING)
        .center_x(Fill)
        .into()
    } else {
        scrollable(
            row(app_items)
                .spacing(SPACING_SMALL),
        )
        .direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::default()
                .width(4)
                .scroller_width(4),
        ))
        .into()
    };

    // Cancel drag button when dragging
    let cancel_area: Element<Message> = if is_dragging {
        button(
            text("Cancel")
                .size(11)
                .color(TEXT_DIM)
        )
        .padding([4, 8])
        .style(|_theme: &Theme, _status| button::Style {
            background: Some(Background::Color(SURFACE_LIGHT)),
            text_color: TEXT_DIM,
            border: Border::default().rounded(BORDER_RADIUS_SMALL),
            ..button::Style::default()
        })
        .on_press(Message::CancelDrag)
        .into()
    } else {
        Space::new().width(0).into()
    };

    let content = column![
        row![header, Space::new().width(Fill), cancel_area].align_y(Alignment::Center),
        Space::new().height(SPACING_SMALL),
        apps_content,
    ]
    .padding(PADDING);

    container(content)
        .width(Fill)
        .style(move |_theme: &Theme| container::Style {
            background: Some(Background::Color(SURFACE)),
            border: Border::default()
                .rounded(BORDER_RADIUS)
                .color(if is_dragging { PRIMARY } else { SURFACE_LIGHT })
                .width(if is_dragging { 2.0 } else { 1.0 }),
            ..container::Style::default()
        })
        .into()
}

/// Create a single app item widget.
fn app_item<'a>(
    app: &'a AppInfo,
    channels: &'a [MixerChannel],
    dragging: Option<&(u32, String)>,
) -> Element<'a, Message> {
    let app_id = app.identifier().to_string();
    let node_id = app.node_id;

    // Check if this app is currently being dragged
    let is_being_dragged = dragging
        .map(|(_, id)| id == &app_id)
        .unwrap_or(false);

    // Check if app is assigned to any channel
    let assigned_channel = channels.iter().find(|c| {
        c.assigned_apps.iter().any(|a| a == &app_id)
    });

    let display_name = clean_app_name(&app.name);
    let is_assigned = assigned_channel.is_some();

    let name_text = text(display_name)
        .size(12)
        .color(if is_being_dragged { PRIMARY } else { TEXT });

    // Show assigned channel indicator
    let status: Element<Message> = if is_being_dragged {
        text("Selected")
            .size(10)
            .color(PRIMARY)
            .into()
    } else if let Some(channel) = assigned_channel {
        text(format!("-> {}", truncate_string(&channel.name, 8)))
            .size(10)
            .color(SUCCESS)
            .into()
    } else {
        text("Click to assign")
            .size(10)
            .color(TEXT_DIM)
            .into()
    };

    let content = column![
        name_text,
        Space::new().height(2),
        status,
    ]
    .align_x(Alignment::Center);

    let app_id_clone = app_id.clone();

    button(content)
        .padding([6, 12])
        .width(95)
        .style(move |_theme: &Theme, status| {
            let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(Background::Color(
                    if is_being_dragged {
                        PRIMARY
                    } else if is_hovered {
                        SURFACE_LIGHT
                    } else if is_assigned {
                        SURFACE_LIGHT
                    } else {
                        BACKGROUND
                    }
                )),
                text_color: if is_being_dragged { BACKGROUND } else { TEXT },
                border: Border::default()
                    .rounded(BORDER_RADIUS_SMALL)
                    .color(if is_being_dragged {
                        PRIMARY
                    } else if is_assigned {
                        SUCCESS
                    } else if is_hovered {
                        TEXT_DIM
                    } else {
                        SURFACE_LIGHT
                    })
                    .width(1.0),
                ..button::Style::default()
            }
        })
        .on_press(Message::StartDraggingApp(node_id, app_id_clone))
        .into()
}

/// Clean up app name for display.
fn clean_app_name(name: &str) -> String {
    let cleaned = name
        .replace(" (Virtual Sink)", "")
        .replace("Audio", "")
        .trim()
        .to_string();

    truncate_string(&cleaned, 12)
}

/// Truncate a string to max length with ellipsis.
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        s.chars().take(max_len.saturating_sub(2)).collect::<String>() + ".."
    }
}
