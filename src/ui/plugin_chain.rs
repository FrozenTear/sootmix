// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Plugin chain UI components.
//!
//! Provides UI for managing per-channel plugin chains:
//! - Plugin browser for discovering and adding plugins
//! - Plugin chain strip showing active plugins on a channel
//! - Plugin slot for individual plugin controls (bypass, remove, edit)
//! - Plugin editor for adjusting parameters

use crate::message::Message;
use crate::plugins::{PluginMetadata, PluginType};
use crate::ui::theme::*;
use iced::widget::{button, column, container, row, scrollable, slider, text, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};
use uuid::Uuid;

// ============================================================================
// FX BUTTON
// ============================================================================

/// Create an "FX" button for opening the plugin chain panel.
/// Shows the number of active plugins if any.
pub fn fx_button<'a>(channel_id: Uuid, plugin_count: usize) -> Element<'a, Message> {
    let label = if plugin_count > 0 {
        format!("FX ({})", plugin_count)
    } else {
        "FX".to_string()
    };

    let has_plugins = plugin_count > 0;

    button(text(label).size(TEXT_SMALL))
        .padding([SPACING_XS, SPACING_SM])
        .style(move |_theme: &Theme, status| {
            let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            let bg_color = if has_plugins {
                if is_hovered {
                    lighten(ACCENT, 0.15)
                } else {
                    ACCENT
                }
            } else if is_hovered {
                SURFACE_LIGHT
            } else {
                SURFACE
            };
            button::Style {
                background: Some(Background::Color(bg_color)),
                text_color: if has_plugins {
                    SOOTMIX_DARK.canvas
                } else {
                    TEXT
                },
                border: Border::default()
                    .rounded(RADIUS_SM)
                    .color(if has_plugins { ACCENT } else { SURFACE_LIGHT })
                    .width(1.0),
                ..button::Style::default()
            }
        })
        .on_press(Message::OpenPluginBrowser(channel_id))
        .into()
}

// ============================================================================
// PLUGIN SLOT
// ============================================================================

/// Create a plugin slot widget showing a single plugin in the chain.
pub fn plugin_slot<'a>(
    channel_id: Uuid,
    instance_id: Uuid,
    plugin_name: &str,
    bypassed: bool,
    slot_index: usize,
    total_slots: usize,
) -> Element<'a, Message> {
    // Plugin name button (opens editor)
    let name_button = button(
        text(truncate_name(plugin_name, 14))
            .size(TEXT_SMALL)
            .color(if bypassed { TEXT_DIM } else { TEXT }),
    )
    .padding([SPACING_XS, SPACING_SM])
    .style(move |_theme: &Theme, status| {
        let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if is_hovered {
                SURFACE_LIGHT
            } else {
                Color::TRANSPARENT
            })),
            text_color: if bypassed { TEXT_DIM } else { TEXT },
            border: Border::default().rounded(RADIUS_SM),
            ..button::Style::default()
        }
    })
    .on_press(Message::OpenPluginEditor(channel_id, instance_id));

    // Bypass button
    let bypass_button = button(text(if bypassed { "B" } else { "\u{2022}" }).size(TEXT_CAPTION))
        .padding([SPACING_XS, SPACING_SM])
        .style(move |_theme: &Theme, status| {
            let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(Background::Color(if bypassed {
                    MUTED_COLOR
                } else if is_hovered {
                    SURFACE_LIGHT
                } else {
                    Color::TRANSPARENT
                })),
                text_color: if bypassed { TEXT } else { TEXT_DIM },
                border: Border::default().rounded(RADIUS_SM),
                ..button::Style::default()
            }
        })
        .on_press(Message::TogglePluginBypass(channel_id, instance_id));

    // Move up button
    let up_button: Element<'a, Message> = if slot_index > 0 {
        button(text("\u{2191}").size(TEXT_CAPTION)) // up arrow
            .padding([SPACING_XS, SPACING_XS])
            .style(|_theme: &Theme, status| {
                let is_hovered =
                    matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: Some(Background::Color(if is_hovered {
                        SURFACE_LIGHT
                    } else {
                        Color::TRANSPARENT
                    })),
                    text_color: TEXT_DIM,
                    border: Border::default(),
                    ..button::Style::default()
                }
            })
            .on_press(Message::MovePluginInChain(channel_id, instance_id, -1))
            .into()
    } else {
        Space::new().width(20).into()
    };

    // Move down button
    let down_button: Element<'a, Message> = if slot_index < total_slots - 1 {
        button(text("\u{2193}").size(TEXT_CAPTION)) // down arrow
            .padding([SPACING_XS, SPACING_XS])
            .style(|_theme: &Theme, status| {
                let is_hovered =
                    matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: Some(Background::Color(if is_hovered {
                        SURFACE_LIGHT
                    } else {
                        Color::TRANSPARENT
                    })),
                    text_color: TEXT_DIM,
                    border: Border::default(),
                    ..button::Style::default()
                }
            })
            .on_press(Message::MovePluginInChain(channel_id, instance_id, 1))
            .into()
    } else {
        Space::new().width(20).into()
    };

    // Remove button
    let remove_button = button(text("\u{00D7}").size(TEXT_CAPTION))
        .padding([SPACING_XS, SPACING_XS])
        .style(|_theme: &Theme, status| {
            let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(Background::Color(if is_hovered {
                    Color { a: 0.2, ..MUTED_COLOR }
                } else {
                    Color::TRANSPARENT
                })),
                text_color: if is_hovered { MUTED_COLOR } else { TEXT_DIM },
                border: Border::default(),
                ..button::Style::default()
            }
        })
        .on_press(Message::RemovePluginFromChannel(channel_id, instance_id));

    let slot_content = row![
        bypass_button,
        name_button,
        Space::new().width(Length::Fill),
        up_button,
        down_button,
        remove_button,
    ]
    .spacing(SPACING_XS)
    .align_y(Alignment::Center);

    container(slot_content)
        .padding([SPACING_XS, SPACING_SM])
        .style(move |_theme: &Theme| container::Style {
            background: Some(Background::Color(if bypassed { SURFACE } else { SURFACE_LIGHT })),
            border: Border::default()
                .rounded(RADIUS_SM)
                .color(if bypassed {
                    SOOTMIX_DARK.border_subtle
                } else {
                    PRIMARY
                })
                .width(1.0),
            ..container::Style::default()
        })
        .into()
}

// ============================================================================
// PLUGIN CHAIN PANEL
// ============================================================================

/// Create the plugin chain panel showing all plugins for a channel.
pub fn plugin_chain_panel(
    channel_id: Uuid,
    channel_name: &str,
    plugins: Vec<(Uuid, String, bool)>, // (instance_id, name, bypassed)
) -> Element<'static, Message> {
    // Header
    let header = row![
        text(format!("{} \u{2014} FX Chain", channel_name))
            .size(TEXT_HEADING)
            .color(TEXT),
        Space::new().width(Length::Fill),
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
            .on_press(Message::ClosePluginBrowser),
    ]
    .align_y(Alignment::Center);

    // Plugin slots
    let total = plugins.len();
    let plugin_slots: Vec<Element<Message>> = plugins
        .iter()
        .enumerate()
        .map(|(i, (instance_id, name, bypassed))| {
            plugin_slot(channel_id, *instance_id, name, *bypassed, i, total)
        })
        .collect();

    let slots_column = if plugin_slots.is_empty() {
        column![text("No plugins").size(TEXT_SMALL).color(TEXT_DIM)].align_x(Alignment::Center)
    } else {
        column(plugin_slots).spacing(SPACING_XS)
    };

    // Add plugin button
    let add_button = button(
        row![
            text("+").size(TEXT_HEADING),
            Space::new().width(SPACING_XS),
            text("Add Plugin").size(TEXT_SMALL),
        ]
        .align_y(Alignment::Center),
    )
    .padding([SPACING_SM, SPACING])
    .style(|_theme: &Theme, status| {
        let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if is_hovered {
                PRIMARY
            } else {
                SURFACE_LIGHT
            })),
            text_color: if is_hovered { SOOTMIX_DARK.canvas } else { TEXT },
            border: Border::default()
                .rounded(RADIUS_SM)
                .color(PRIMARY)
                .width(1.0),
            ..button::Style::default()
        }
    })
    .on_press(Message::OpenPluginBrowser(channel_id));

    // Divider
    let divider = container(Space::new().height(1))
        .width(Length::Fill)
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(SOOTMIX_DARK.border_subtle)),
            ..container::Style::default()
        });

    let content = column![
        header,
        Space::new().height(SPACING_SM),
        divider,
        Space::new().height(SPACING_SM),
        scrollable(slots_column).height(Length::Fixed(200.0)),
        Space::new().height(SPACING_SM),
        container(add_button).center_x(Length::Fill),
    ]
    .padding(PADDING)
    .spacing(SPACING_XS);

    container(content)
        .width(Length::Fixed(300.0))
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(BACKGROUND)),
            border: Border::default()
                .rounded(RADIUS)
                .color(PRIMARY)
                .width(1.0),
            shadow: iced::Shadow {
                color: Color { a: 0.3, ..Color::BLACK },
                offset: iced::Vector::new(0.0, 4.0),
                blur_radius: 16.0,
            },
            ..container::Style::default()
        })
        .into()
}

// ============================================================================
// PLUGIN BROWSER
// ============================================================================

/// Create the plugin browser modal for selecting plugins to add.
pub fn plugin_browser(
    channel_id: Uuid,
    available_plugins: Vec<PluginMetadata>,
) -> Element<'static, Message> {
    // Header
    let header = row![
        text("Add Plugin").size(TEXT_HEADING).color(TEXT),
        Space::new().width(Length::Fill),
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
            .on_press(Message::ClosePluginBrowser),
    ]
    .align_y(Alignment::Center);

    // Plugin list
    let plugin_items: Vec<Element<Message>> = if available_plugins.is_empty() {
        vec![column![
            text("No plugins found").size(TEXT_SMALL).color(TEXT_DIM),
            Space::new().height(SPACING_SM),
            text("Place plugins in:").size(TEXT_CAPTION).color(TEXT_DIM),
            text("~/.local/share/sootmix/plugins/")
                .size(TEXT_CAPTION)
                .color(TEXT_DIM),
        ]
        .align_x(Alignment::Center)
        .into()]
    } else {
        available_plugins
            .iter()
            .map(|meta| plugin_browser_item(channel_id, meta))
            .collect()
    };

    let plugin_list = scrollable(column(plugin_items).spacing(SPACING_XS)).height(Length::Fixed(300.0));

    // Download plugins button
    let download_btn = button(
        row![
            text("\u{2B07}").size(TEXT_SMALL),
            Space::new().width(SPACING_XS),
            text("Download Plugins").size(TEXT_SMALL),
        ]
        .align_y(Alignment::Center),
    )
    .padding([SPACING_XS, SPACING_SM])
    .style(|_theme: &Theme, status| {
        let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if is_hovered {
                SOOTMIX_DARK.accent_secondary
            } else {
                SURFACE_LIGHT
            })),
            text_color: if is_hovered { SOOTMIX_DARK.canvas } else { TEXT_DIM },
            border: Border::default()
                .rounded(RADIUS_SM)
                .color(SOOTMIX_DARK.accent_secondary)
                .width(1.0),
            ..button::Style::default()
        }
    })
    .on_press(Message::OpenPluginDownloader);

    // Divider style closure
    let divider_style = |_theme: &Theme| container::Style {
        background: Some(Background::Color(SOOTMIX_DARK.border_subtle)),
        ..container::Style::default()
    };

    let content = column![
        header,
        Space::new().height(SPACING_SM),
        container(Space::new().height(1)).width(Length::Fill).style(divider_style),
        Space::new().height(SPACING_SM),
        plugin_list,
        Space::new().height(SPACING_SM),
        container(Space::new().height(1)).width(Length::Fill).style(divider_style),
        Space::new().height(SPACING_SM),
        container(download_btn).center_x(Length::Fill),
    ]
    .padding(PADDING)
    .spacing(SPACING_XS);

    container(content)
        .width(Length::Fixed(350.0))
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(BACKGROUND)),
            border: Border::default()
                .rounded(RADIUS)
                .color(ACCENT)
                .width(2.0),
            shadow: iced::Shadow {
                color: Color { a: 0.3, ..Color::BLACK },
                offset: iced::Vector::new(0.0, 4.0),
                blur_radius: 16.0,
            },
            ..container::Style::default()
        })
        .into()
}

/// Create a single plugin item in the browser.
fn plugin_browser_item(channel_id: Uuid, meta: &PluginMetadata) -> Element<'static, Message> {
    let plugin_id = meta
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let name = meta
        .info
        .as_ref()
        .map(|i| i.name.to_string())
        .unwrap_or_else(|| plugin_id.clone());

    let vendor = meta
        .info
        .as_ref()
        .map(|i| i.vendor.to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    let type_label = match meta.plugin_type {
        PluginType::Native => "Native",
        PluginType::Wasm => "WASM",
        PluginType::Builtin => "Built-in",
        #[cfg(feature = "lv2-plugins")]
        PluginType::Lv2 => "LV2",
        #[cfg(feature = "vst3-plugins")]
        PluginType::Vst3 => "VST3",
    };

    // Type badge color
    let badge_color = match meta.plugin_type {
        PluginType::Native => PRIMARY,
        PluginType::Wasm => WARNING,
        PluginType::Builtin => SUCCESS,
        #[cfg(feature = "lv2-plugins")]
        PluginType::Lv2 => ACCENT,
        #[cfg(feature = "vst3-plugins")]
        PluginType::Vst3 => ACCENT,
    };

    let plugin_id_clone = plugin_id.clone();

    button(
        row![
            column![
                text(name).size(TEXT_SMALL).color(TEXT),
                row![
                    text(vendor).size(TEXT_CAPTION).color(TEXT_DIM),
                    Space::new().width(SPACING_SM),
                    container(text(type_label).size(TEXT_CAPTION).color(TEXT))
                        .padding([1, SPACING_XS as u16])
                        .style(move |_theme: &Theme| container::Style {
                            background: Some(Background::Color(Color { a: 0.2, ..badge_color })),
                            border: Border::default().rounded(2.0),
                            ..container::Style::default()
                        }),
                ]
                .align_y(Alignment::Center),
            ]
            .spacing(SPACING_XS),
            Space::new().width(Length::Fill),
            text("+").size(TEXT_HEADING).color(PRIMARY),
        ]
        .align_y(Alignment::Center)
        .padding([SPACING_XS, SPACING_SM]),
    )
    .padding(0)
    .width(Length::Fill)
    .style(|_theme: &Theme, status| {
        let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if is_hovered {
                SURFACE_LIGHT
            } else {
                SURFACE
            })),
            text_color: TEXT,
            border: Border::default()
                .rounded(RADIUS_SM)
                .color(if is_hovered {
                    PRIMARY
                } else {
                    SOOTMIX_DARK.border_subtle
                })
                .width(1.0),
            ..button::Style::default()
        }
    })
    .on_press(Message::AddPluginToChannel(channel_id, plugin_id_clone))
    .into()
}

// ============================================================================
// PLUGIN EDITOR
// ============================================================================

/// Parameter info for the plugin editor UI.
pub struct PluginEditorParam {
    /// Parameter index.
    pub index: u32,
    /// Parameter name.
    pub name: String,
    /// Unit label.
    pub unit: String,
    /// Minimum value.
    pub min: f32,
    /// Maximum value.
    pub max: f32,
    /// Current value.
    pub value: f32,
}

/// Create the plugin editor panel showing plugin parameters.
pub fn plugin_editor(
    instance_id: Uuid,
    plugin_name: &str,
    params: Vec<PluginEditorParam>,
) -> Element<'static, Message> {
    // Header
    let header = row![
        text(format!("{} \u{2014} Parameters", plugin_name))
            .size(TEXT_HEADING)
            .color(TEXT),
        Space::new().width(Length::Fill),
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
            .on_press(Message::ClosePluginEditor),
    ]
    .align_y(Alignment::Center);

    // Parameter sliders
    let param_elements: Vec<Element<Message>> = if params.is_empty() {
        vec![text("No parameters").size(TEXT_SMALL).color(TEXT_DIM).into()]
    } else {
        params
            .into_iter()
            .map(|param| parameter_slider(instance_id, param))
            .collect()
    };

    let params_column = column(param_elements).spacing(SPACING_SM);

    // Divider
    let divider = container(Space::new().height(1))
        .width(Length::Fill)
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(SOOTMIX_DARK.border_subtle)),
            ..container::Style::default()
        });

    let content = column![
        header,
        Space::new().height(SPACING_SM),
        divider,
        Space::new().height(SPACING_SM),
        scrollable(params_column).height(Length::Fixed(300.0)),
    ]
    .padding(PADDING)
    .spacing(SPACING_XS);

    container(content)
        .width(Length::Fixed(320.0))
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(BACKGROUND)),
            border: Border::default()
                .rounded(RADIUS)
                .color(SOOTMIX_DARK.accent_warm)
                .width(2.0),
            shadow: iced::Shadow {
                color: Color { a: 0.3, ..Color::BLACK },
                offset: iced::Vector::new(0.0, 4.0),
                blur_radius: 16.0,
            },
            ..container::Style::default()
        })
        .into()
}

/// Create a parameter slider widget.
fn parameter_slider(instance_id: Uuid, param: PluginEditorParam) -> Element<'static, Message> {
    let param_index = param.index;
    let display_value = if param.unit.is_empty() {
        format!("{:.2}", param.value)
    } else {
        format!("{:.2} {}", param.value, param.unit)
    };

    let name_text = text(param.name).size(TEXT_SMALL).color(TEXT);
    let value_text = text(display_value).size(TEXT_CAPTION).color(TEXT_DIM);

    let slider_widget = slider(param.min..=param.max, param.value, move |v| {
        Message::PluginParameterChanged(instance_id, param_index, v)
    })
    .step(0.01)
    .width(Length::Fill)
    .style(|_theme: &Theme, _status| slider::Style {
        rail: slider::Rail {
            backgrounds: (
                Background::Color(SOOTMIX_DARK.accent_warm),
                Background::Color(SLIDER_TRACK),
            ),
            width: 4.0,
            border: Border::default().rounded(2.0),
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle { radius: 7.0 },
            background: Background::Color(TEXT),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    });

    column![
        row![name_text, Space::new().width(Length::Fill), value_text,].align_y(Alignment::Center),
        slider_widget,
    ]
    .spacing(SPACING_XS)
    .into()
}

// ============================================================================
// HELPERS
// ============================================================================

/// Truncate a plugin name for display.
fn truncate_name(name: &str, max_len: usize) -> String {
    if name.len() <= max_len {
        name.to_string()
    } else {
        format!("{}...", &name[..max_len.saturating_sub(3)])
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
