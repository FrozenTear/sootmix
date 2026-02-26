// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Settings panel UI components.
//!
//! Provides a modal UI for daemon service controls and application settings.

use crate::message::Message;
use crate::ui::theme::*;
use iced::widget::{button, checkbox, column, container, row, text, Space};
use iced::{Alignment, Background, Border, Color, Element, Fill, Length, Theme};

/// Create the settings panel modal.
pub fn settings_panel<'a>(
    daemon_connected: bool,
    daemon_autostart: bool,
    daemon_action_pending: bool,
) -> Element<'a, Message> {
    // Header with title and close button
    let header = row![
        text("Settings").size(TEXT_HEADING).color(TEXT),
        Space::new().width(Fill),
        button(text("\u{00D7}").size(TEXT_HEADING))
            .padding([SPACING_XS, SPACING_SM])
            .style(|_theme: &Theme, status| {
                let is_hovered =
                    matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: Some(Background::Color(if is_hovered {
                        Color { a: 0.15, ..MUTED_COLOR }
                    } else {
                        Color::TRANSPARENT
                    })),
                    text_color: TEXT_DIM,
                    border: Border::default().rounded(RADIUS_SM),
                    ..button::Style::default()
                }
            })
            .on_press(Message::CloseSettings),
    ]
    .align_y(Alignment::Center);

    // Divider
    let divider = container(Space::new().height(1))
        .width(Length::Fill)
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(SOOTMIX_DARK.border_subtle)),
            ..container::Style::default()
        });

    // --- Daemon section ---
    let section_label = text("Daemon Service")
        .size(TEXT_BODY)
        .color(TEXT);

    // Status indicator
    let (status_text, status_color) = if daemon_connected {
        ("Running", SUCCESS)
    } else {
        ("Stopped", MUTED_COLOR)
    };

    let status_dot = container(Space::new().width(8).height(8))
        .style(move |_theme: &Theme| container::Style {
            background: Some(Background::Color(status_color)),
            border: Border::default().rounded(4.0),
            ..container::Style::default()
        });

    let status_row = row![
        status_dot,
        Space::new().width(SPACING_XS),
        text(status_text).size(TEXT_SMALL).color(status_color),
    ]
    .align_y(Alignment::Center);

    // Action buttons
    let start_btn = action_button("Start", daemon_action_pending || daemon_connected, || {
        Message::DaemonStart
    });
    let stop_btn = action_button("Stop", daemon_action_pending || !daemon_connected, || {
        Message::DaemonStop
    });
    let restart_btn = action_button("Restart", daemon_action_pending, || Message::DaemonRestart);

    let buttons_row = row![start_btn, stop_btn, restart_btn,]
        .spacing(SPACING_SM)
        .align_y(Alignment::Center);

    // Autostart toggle
    let autostart_check = checkbox(daemon_autostart)
        .on_toggle(Message::DaemonToggleAutostart)
        .size(14)
        .style(|_theme: &Theme, status| {
            let is_checked = matches!(
                status,
                checkbox::Status::Active { is_checked: true }
                    | checkbox::Status::Hovered { is_checked: true }
            );
            checkbox::Style {
                background: Background::Color(if is_checked { PRIMARY } else { SURFACE }),
                icon_color: SOOTMIX_DARK.canvas,
                border: Border::default()
                    .rounded(RADIUS_SM)
                    .color(if is_checked {
                        PRIMARY
                    } else {
                        SOOTMIX_DARK.border_default
                    })
                    .width(1.0),
                text_color: Some(TEXT_DIM),
            }
        });
    let autostart_toggle = row![
        autostart_check,
        Space::new().width(SPACING_XS),
        text("Start on login").size(TEXT_SMALL).color(TEXT_DIM),
    ]
    .align_y(Alignment::Center);

    // Pending indicator
    let pending_indicator: Element<Message> = if daemon_action_pending {
        text("Working...")
            .size(TEXT_CAPTION)
            .color(PRIMARY)
            .into()
    } else {
        Space::new().height(0).into()
    };

    // Main content
    let content = column![
        header,
        Space::new().height(SPACING_SM),
        divider,
        Space::new().height(SPACING_SM),
        section_label,
        Space::new().height(SPACING_SM),
        status_row,
        Space::new().height(SPACING_SM),
        buttons_row,
        Space::new().height(SPACING_XS),
        pending_indicator,
        Space::new().height(SPACING_SM),
        autostart_toggle,
    ]
    .padding(PADDING)
    .spacing(SPACING_XS);

    container(content)
        .width(Length::Fixed(380.0))
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(BACKGROUND)),
            border: Border::default()
                .rounded(RADIUS_LG)
                .color(PRIMARY)
                .width(2.0),
            shadow: iced::Shadow {
                color: Color { a: 0.4, ..Color::BLACK },
                offset: iced::Vector::new(0.0, 8.0),
                blur_radius: 24.0,
            },
            ..container::Style::default()
        })
        .into()
}

/// Create a styled action button.
fn action_button<'a>(
    label: &str,
    disabled: bool,
    on_press: impl Fn() -> Message + 'a,
) -> Element<'a, Message> {
    let label = label.to_string();
    let mut btn = button(
        text(label)
            .size(TEXT_SMALL)
            .color(if disabled { TEXT_DIM } else { TEXT }),
    )
    .padding([SPACING_XS, SPACING_SM])
    .style(move |_theme: &Theme, status| {
        let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if disabled {
                SURFACE
            } else if is_hovered {
                SURFACE_LIGHT
            } else {
                SURFACE
            })),
            text_color: if disabled { TEXT_DIM } else { TEXT },
            border: Border::default()
                .rounded(RADIUS_SM)
                .color(if disabled {
                    SOOTMIX_DARK.border_subtle
                } else {
                    SOOTMIX_DARK.border_default
                })
                .width(1.0),
            ..button::Style::default()
        }
    });

    if !disabled {
        btn = btn.on_press(on_press());
    }

    btn.into()
}
