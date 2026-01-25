// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Plugin chain UI components.
//!
//! Provides UI for managing per-channel plugin chains:
//! - Plugin browser for discovering and adding plugins
//! - Plugin chain strip showing active plugins on a channel
//! - Plugin slot for individual plugin controls (bypass, remove, edit)

use crate::message::Message;
use crate::plugins::{PluginMetadata, PluginType};
use crate::ui::theme::*;
use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};
use uuid::Uuid;

/// Create an "FX" button for opening the plugin chain panel.
/// Shows the number of active plugins if any.
pub fn fx_button<'a>(channel_id: Uuid, plugin_count: usize) -> Element<'a, Message> {
    let label = if plugin_count > 0 {
        format!("FX ({})", plugin_count)
    } else {
        "FX".to_string()
    };

    let has_plugins = plugin_count > 0;

    button(text(label).size(11))
        .padding([4, 8])
        .style(move |_theme: &Theme, status| {
            let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            let bg_color = if has_plugins {
                if is_hovered { ACCENT } else { PRIMARY }
            } else {
                if is_hovered { SURFACE_LIGHT } else { SURFACE }
            };
            button::Style {
                background: Some(Background::Color(bg_color)),
                text_color: TEXT,
                border: Border::default()
                    .rounded(BORDER_RADIUS_SMALL)
                    .color(if has_plugins { PRIMARY } else { SURFACE_LIGHT })
                    .width(1.0),
                ..button::Style::default()
            }
        })
        .on_press(Message::OpenPluginBrowser(channel_id))
        .into()
}

/// Create a plugin slot widget showing a single plugin in the chain.
pub fn plugin_slot<'a>(
    channel_id: Uuid,
    instance_id: Uuid,
    plugin_name: &str,
    bypassed: bool,
    slot_index: usize,
    total_slots: usize,
) -> Element<'a, Message> {
    // Plugin name (clickable to open editor)
    let name_button = button(
        text(truncate_name(plugin_name, 12))
            .size(11)
            .color(if bypassed { TEXT_DIM } else { TEXT }),
    )
    .padding([4, 6])
    .style(move |_theme: &Theme, status| {
        let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if is_hovered {
                SURFACE_LIGHT
            } else {
                Color::TRANSPARENT
            })),
            text_color: if bypassed { TEXT_DIM } else { TEXT },
            border: Border::default().rounded(BORDER_RADIUS_SMALL),
            ..button::Style::default()
        }
    })
    .on_press(Message::OpenPluginEditor(channel_id, instance_id));

    // Bypass button
    let bypass_button = button(text(if bypassed { "B" } else { "." }).size(10))
        .padding([2, 6])
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
                border: Border::default().rounded(BORDER_RADIUS_SMALL),
                ..button::Style::default()
            }
        })
        .on_press(Message::TogglePluginBypass(channel_id, instance_id));

    // Move up button (disabled if first)
    let up_button: Element<'a, Message> = if slot_index > 0 {
        button(text("^").size(10))
            .padding([2, 4])
            .style(|_theme: &Theme, status| {
                let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
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

    // Move down button (disabled if last)
    let down_button: Element<'a, Message> = if slot_index < total_slots - 1 {
        button(text("v").size(10))
            .padding([2, 4])
            .style(|_theme: &Theme, status| {
                let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
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
    let remove_button = button(text("x").size(10))
        .padding([2, 4])
        .style(|_theme: &Theme, status| {
            let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(Background::Color(if is_hovered {
                    MUTED_COLOR
                } else {
                    Color::TRANSPARENT
                })),
                text_color: if is_hovered { TEXT } else { TEXT_DIM },
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
    .spacing(2)
    .align_y(Alignment::Center);

    container(slot_content)
        .padding([4, 6])
        .style(move |_theme: &Theme| container::Style {
            background: Some(Background::Color(if bypassed {
                SURFACE
            } else {
                SURFACE_LIGHT
            })),
            border: Border::default()
                .rounded(BORDER_RADIUS_SMALL)
                .color(if bypassed { SURFACE_LIGHT } else { PRIMARY })
                .width(1.0),
            ..container::Style::default()
        })
        .into()
}

/// Create the plugin chain panel showing all plugins for a channel.
pub fn plugin_chain_panel(
    channel_id: Uuid,
    channel_name: &str,
    plugins: Vec<(Uuid, String, bool)>, // (instance_id, name, bypassed)
) -> Element<'static, Message> {
    let header = row![
        text(format!("{} - Plugin Chain", channel_name))
            .size(14)
            .color(TEXT),
        Space::new().width(Length::Fill),
        button(text("x").size(14))
            .padding([2, 6])
            .style(|_theme: &Theme, status| {
                let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: Some(Background::Color(if is_hovered {
                        MUTED_COLOR
                    } else {
                        Color::TRANSPARENT
                    })),
                    text_color: TEXT_DIM,
                    border: Border::default(),
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
        column![text("No plugins").size(11).color(TEXT_DIM)]
            .align_x(Alignment::Center)
    } else {
        column(plugin_slots).spacing(4)
    };

    // Add plugin button
    let add_button = button(
        row![
            text("+").size(14),
            Space::new().width(4),
            text("Add Plugin").size(11),
        ]
        .align_y(Alignment::Center),
    )
    .padding([6, 12])
    .style(|_theme: &Theme, status| {
        let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if is_hovered { PRIMARY } else { SURFACE_LIGHT })),
            text_color: TEXT,
            border: Border::default()
                .rounded(BORDER_RADIUS_SMALL)
                .color(PRIMARY)
                .width(1.0),
            ..button::Style::default()
        }
    })
    .on_press(Message::OpenPluginBrowser(channel_id));

    let content = column![
        header,
        Space::new().height(SPACING),
        container(Space::new().height(1))
            .width(Length::Fill)
            .style(|_theme: &Theme| container::Style {
                background: Some(Background::Color(SURFACE_LIGHT)),
                ..container::Style::default()
            }),
        Space::new().height(SPACING),
        scrollable(slots_column).height(Length::Fixed(200.0)),
        Space::new().height(SPACING),
        container(add_button).center_x(Length::Fill),
    ]
    .padding(PADDING)
    .spacing(SPACING_SMALL);

    container(content)
        .width(Length::Fixed(280.0))
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(BACKGROUND)),
            border: Border::default()
                .rounded(BORDER_RADIUS)
                .color(PRIMARY)
                .width(1.0),
            ..container::Style::default()
        })
        .into()
}

/// Create the plugin browser modal for selecting plugins to add.
pub fn plugin_browser(
    channel_id: Uuid,
    available_plugins: Vec<PluginMetadata>,
) -> Element<'static, Message> {
    let header = row![
        text("Add Plugin").size(16).color(TEXT),
        Space::new().width(Length::Fill),
        button(text("x").size(14))
            .padding([2, 6])
            .style(|_theme: &Theme, status| {
                let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: Some(Background::Color(if is_hovered {
                        MUTED_COLOR
                    } else {
                        Color::TRANSPARENT
                    })),
                    text_color: TEXT_DIM,
                    border: Border::default(),
                    ..button::Style::default()
                }
            })
            .on_press(Message::ClosePluginBrowser),
    ]
    .align_y(Alignment::Center);

    // Plugin list
    let plugin_items: Vec<Element<Message>> = if available_plugins.is_empty() {
        vec![
            column![
                text("No plugins found").size(12).color(TEXT_DIM),
                Space::new().height(8),
                text("Place .so plugins in:").size(10).color(TEXT_DIM),
                text("~/.local/share/sootmix/plugins/native/")
                    .size(9)
                    .color(TEXT_DIM),
            ]
            .align_x(Alignment::Center)
            .into(),
        ]
    } else {
        available_plugins
            .iter()
            .map(|meta| plugin_browser_item(channel_id, meta))
            .collect()
    };

    let plugin_list = scrollable(column(plugin_items).spacing(4)).height(Length::Fixed(300.0));

    let content = column![
        header,
        Space::new().height(SPACING),
        container(Space::new().height(1))
            .width(Length::Fill)
            .style(|_theme: &Theme| container::Style {
                background: Some(Background::Color(SURFACE_LIGHT)),
                ..container::Style::default()
            }),
        Space::new().height(SPACING),
        plugin_list,
    ]
    .padding(PADDING)
    .spacing(SPACING_SMALL);

    container(content)
        .width(Length::Fixed(350.0))
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(BACKGROUND)),
            border: Border::default()
                .rounded(BORDER_RADIUS)
                .color(PRIMARY)
                .width(2.0),
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

    let plugin_id_clone = plugin_id.clone();
    let detail_text = format!("{} - {}", vendor, type_label);

    button(
        row![
            column![
                text(name).size(12).color(TEXT),
                text(detail_text).size(9).color(TEXT_DIM),
            ]
            .spacing(2),
            Space::new().width(Length::Fill),
            text("+").size(14).color(PRIMARY),
        ]
        .align_y(Alignment::Center)
        .padding([4, 8]),
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
                .rounded(BORDER_RADIUS_SMALL)
                .color(if is_hovered { PRIMARY } else { SURFACE_LIGHT })
                .width(1.0),
            ..button::Style::default()
        }
    })
    .on_press(Message::AddPluginToChannel(channel_id, plugin_id_clone))
    .into()
}

/// Truncate a plugin name for display.
fn truncate_name(name: &str, max_len: usize) -> String {
    if name.len() <= max_len {
        name.to_string()
    } else {
        format!("{}...", &name[..max_len.saturating_sub(3)])
    }
}

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
    let _instance_id_for_close = instance_id;

    let header = row![
        text(format!("{} - Parameters", plugin_name))
            .size(14)
            .color(TEXT),
        Space::new().width(Length::Fill),
        button(text("x").size(14))
            .padding([2, 6])
            .style(|_theme: &Theme, status| {
                let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: Some(Background::Color(if is_hovered {
                        MUTED_COLOR
                    } else {
                        Color::TRANSPARENT
                    })),
                    text_color: TEXT_DIM,
                    border: Border::default(),
                    ..button::Style::default()
                }
            })
            .on_press(Message::ClosePluginEditor),
    ]
    .align_y(Alignment::Center);

    // Parameter sliders
    let param_elements: Vec<Element<Message>> = if params.is_empty() {
        vec![text("No parameters").size(11).color(TEXT_DIM).into()]
    } else {
        params
            .into_iter()
            .map(|param| parameter_slider(instance_id, param))
            .collect()
    };

    let params_column = column(param_elements).spacing(8);

    let content = column![
        header,
        Space::new().height(SPACING),
        container(Space::new().height(1))
            .width(Length::Fill)
            .style(|_theme: &Theme| container::Style {
                background: Some(Background::Color(SURFACE_LIGHT)),
                ..container::Style::default()
            }),
        Space::new().height(SPACING),
        scrollable(params_column).height(Length::Fixed(300.0)),
    ]
    .padding(PADDING)
    .spacing(SPACING_SMALL);

    container(content)
        .width(Length::Fixed(300.0))
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(BACKGROUND)),
            border: Border::default()
                .rounded(BORDER_RADIUS)
                .color(ACCENT)
                .width(2.0),
            ..container::Style::default()
        })
        .into()
}

/// Create a parameter slider widget.
fn parameter_slider(instance_id: Uuid, param: PluginEditorParam) -> Element<'static, Message> {
    use iced::widget::slider;

    let param_index = param.index;
    let display_value = if param.unit.is_empty() {
        format!("{:.2}", param.value)
    } else {
        format!("{:.2} {}", param.value, param.unit)
    };

    let name_text = text(param.name).size(11).color(TEXT);
    let value_text = text(display_value).size(10).color(TEXT_DIM);

    let slider_widget = slider(param.min..=param.max, param.value, move |v| {
        Message::PluginParameterChanged(instance_id, param_index, v)
    })
    .step(0.01)
    .width(Length::Fill)
    .style(|_theme: &Theme, _status| slider::Style {
        rail: slider::Rail {
            backgrounds: (
                Background::Color(ACCENT),
                Background::Color(SLIDER_TRACK),
            ),
            width: 4.0,
            border: Border::default().rounded(2.0),
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle { radius: 6.0 },
            background: Background::Color(TEXT),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    });

    column![
        row![name_text, Space::new().width(Length::Fill), value_text,].align_y(Alignment::Center),
        slider_widget,
    ]
    .spacing(4)
    .into()
}
