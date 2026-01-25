// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Routing rules panel UI component.
//!
//! Auto-routing rules allow automatic assignment of audio applications
//! to mixer channels based on pattern matching.

use crate::config::{MatchTarget, RoutingRulesConfig};
use crate::message::Message;
use crate::state::EditingRule;
use crate::ui::theme::*;
use iced::widget::{
    button, checkbox, column, container, pick_list, row, scrollable, text, text_input, Space,
};
use iced::{Alignment, Background, Border, Color, Element, Fill, Length, Theme};

// ============================================================================
// ROUTING RULES PANEL
// ============================================================================

/// Create the routing rules panel.
pub fn routing_rules_panel<'a>(
    rules: &'a RoutingRulesConfig,
    editing: Option<&'a EditingRule>,
    channel_names: Vec<String>,
) -> Element<'a, Message> {
    // === HEADER ===
    let header = row![
        text("Auto-Routing Rules").size(TEXT_HEADING).color(TEXT),
        Space::new().width(Fill),
        button(text("+ New Rule").size(TEXT_SMALL))
            .padding([SPACING_XS, SPACING_SM])
            .style(|_theme: &Theme, status| {
                let is_hovered =
                    matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: Some(Background::Color(if is_hovered {
                        lighten(PRIMARY, 0.1)
                    } else {
                        PRIMARY
                    })),
                    text_color: SOOTMIX_DARK.canvas,
                    border: Border::default().rounded(RADIUS_SM),
                    ..button::Style::default()
                }
            })
            .on_press(Message::StartEditingRule(None)),
        Space::new().width(SPACING_SM),
        button(text("Close").size(TEXT_SMALL))
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
                    text_color: TEXT,
                    border: Border::default().rounded(RADIUS_SM),
                    ..button::Style::default()
                }
            })
            .on_press(Message::CloseRoutingRulesPanel),
    ]
    .align_y(Alignment::Center);

    // === CONTENT ===
    let content: Element<Message> = if let Some(edit) = editing {
        rule_edit_form(edit, channel_names)
    } else if rules.rules.is_empty() {
        container(
            text("No routing rules defined. Click '+ New Rule' to create one.")
                .size(TEXT_SMALL)
                .color(TEXT_DIM),
        )
        .padding(PADDING)
        .center_x(Fill)
        .into()
    } else {
        let rule_items: Vec<Element<Message>> = rules
            .rules
            .iter()
            .enumerate()
            .map(|(idx, rule)| rule_item(rule, idx, rules.rules.len()))
            .collect();

        scrollable(column(rule_items).spacing(SPACING_XS))
            .height(Length::Fixed(200.0))
            .into()
    };

    // === DIVIDER ===
    let divider = container(Space::new().height(1))
        .width(Length::Fill)
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(SOOTMIX_DARK.border_subtle)),
            ..container::Style::default()
        });

    let panel = column![
        header,
        Space::new().height(SPACING_SM),
        divider,
        Space::new().height(SPACING_SM),
        content,
    ]
    .padding(PADDING);

    container(panel)
        .width(Fill)
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(SURFACE)),
            border: Border::default()
                .rounded(RADIUS)
                .color(SOOTMIX_DARK.border_subtle)
                .width(1.0),
            ..container::Style::default()
        })
        .into()
}

// ============================================================================
// RULE ITEM
// ============================================================================

/// Create a single rule item row.
fn rule_item<'a>(
    rule: &'a crate::config::RoutingRule,
    index: usize,
    total: usize,
) -> Element<'a, Message> {
    let rule_enabled = rule.enabled;

    let enabled_checkbox = checkbox(rule.enabled)
        .on_toggle(move |_| Message::ToggleRoutingRule(rule.id))
        .size(14);

    let name_text = text(&rule.name)
        .size(TEXT_SMALL)
        .color(if rule.enabled { TEXT } else { TEXT_DIM });

    let pattern_text = text(format!(
        "{} {} \"{}\"",
        rule.match_target.display_name(),
        rule.match_type.type_name(),
        truncate(rule.match_type.pattern(), 20)
    ))
    .size(TEXT_CAPTION)
    .color(TEXT_DIM);

    let target_text = text(format!("\u{2192} {}", &rule.target_channel))
        .size(TEXT_SMALL)
        .color(SUCCESS);

    let priority_text = container(text(format!("P{}", rule.priority)).size(TEXT_CAPTION).color(TEXT))
        .padding([1, SPACING_XS as u16])
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(Color {
                a: 0.15,
                ..PRIMARY
            })),
            border: Border::default().rounded(2.0),
            ..container::Style::default()
        });

    // Move up/down buttons
    let up_button: Element<Message> = if index > 0 {
        button(text("\u{2191}").size(TEXT_CAPTION))
            .padding([SPACING_XS, SPACING_SM])
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
                    border: Border::default().rounded(RADIUS_SM),
                    ..button::Style::default()
                }
            })
            .on_press(Message::MoveRoutingRuleUp(rule.id))
            .into()
    } else {
        Space::new().width(28).into()
    };

    let down_button: Element<Message> = if index < total - 1 {
        button(text("\u{2193}").size(TEXT_CAPTION))
            .padding([SPACING_XS, SPACING_SM])
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
                    border: Border::default().rounded(RADIUS_SM),
                    ..button::Style::default()
                }
            })
            .on_press(Message::MoveRoutingRuleDown(rule.id))
            .into()
    } else {
        Space::new().width(28).into()
    };

    let edit_button = button(text("Edit").size(TEXT_CAPTION))
        .padding([SPACING_XS, SPACING_SM])
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
                    .color(SOOTMIX_DARK.border_default)
                    .width(1.0),
                ..button::Style::default()
            }
        })
        .on_press(Message::StartEditingRule(Some(rule.id)));

    let delete_button = button(text("\u{00D7}").size(TEXT_CAPTION))
        .padding([SPACING_XS, SPACING_SM])
        .style(|_theme: &Theme, status| {
            let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(Background::Color(if is_hovered {
                    MUTED_COLOR
                } else {
                    Color { a: 0.15, ..MUTED_COLOR }
                })),
                text_color: if is_hovered { TEXT } else { MUTED_COLOR },
                border: Border::default().rounded(RADIUS_SM),
                ..button::Style::default()
            }
        })
        .on_press(Message::DeleteRoutingRule(rule.id));

    let rule_content = row![
        enabled_checkbox,
        Space::new().width(SPACING_SM),
        column![name_text, pattern_text].spacing(2),
        Space::new().width(SPACING),
        target_text,
        Space::new().width(Fill),
        priority_text,
        Space::new().width(SPACING_SM),
        up_button,
        down_button,
        Space::new().width(SPACING_SM),
        edit_button,
        Space::new().width(SPACING_XS),
        delete_button,
    ]
    .align_y(Alignment::Center)
    .padding([SPACING_SM, SPACING]);

    container(rule_content)
        .width(Fill)
        .style(move |_theme: &Theme| container::Style {
            background: Some(Background::Color(if rule_enabled {
                SURFACE_LIGHT
            } else {
                SURFACE
            })),
            border: Border::default()
                .rounded(RADIUS_SM)
                .color(if rule_enabled {
                    SOOTMIX_DARK.border_default
                } else {
                    SOOTMIX_DARK.border_subtle
                })
                .width(1.0),
            ..container::Style::default()
        })
        .into()
}

// ============================================================================
// RULE EDIT FORM
// ============================================================================

/// Create the rule edit form.
fn rule_edit_form<'a>(edit: &'a EditingRule, channel_names: Vec<String>) -> Element<'a, Message> {
    let is_new = edit.id.is_none();
    let title = if is_new { "New Rule" } else { "Edit Rule" };

    let name_input = text_input("Rule name...", &edit.name)
        .on_input(Message::RuleNameChanged)
        .padding([SPACING_XS, SPACING_SM])
        .size(TEXT_SMALL)
        .style(|_theme: &Theme, _status| text_input::Style {
            background: Background::Color(SURFACE_LIGHT),
            border: Border::default()
                .rounded(RADIUS_SM)
                .color(SOOTMIX_DARK.border_default)
                .width(1.0),
            icon: TEXT,
            placeholder: TEXT_DIM,
            value: TEXT,
            selection: PRIMARY,
        });

    let match_target_options = MatchTarget::all().to_vec();
    let match_target_picker = pick_list(
        match_target_options,
        Some(edit.match_target),
        |target| Message::RuleMatchTargetChanged(target),
    )
    .text_size(TEXT_SMALL)
    .padding([SPACING_XS, SPACING_SM])
    .style(|_theme: &Theme, _status| pick_list::Style {
        text_color: TEXT,
        placeholder_color: TEXT_DIM,
        handle_color: TEXT_DIM,
        background: Background::Color(SURFACE_LIGHT),
        border: Border::default()
            .rounded(RADIUS_SM)
            .color(SOOTMIX_DARK.border_default)
            .width(1.0),
    });

    let match_type_options = vec![
        "contains".to_string(),
        "exact".to_string(),
        "regex".to_string(),
        "glob".to_string(),
    ];
    let match_type_picker = pick_list(
        match_type_options,
        Some(edit.match_type_name.clone()),
        Message::RuleMatchTypeChanged,
    )
    .text_size(TEXT_SMALL)
    .padding([SPACING_XS, SPACING_SM])
    .style(|_theme: &Theme, _status| pick_list::Style {
        text_color: TEXT,
        placeholder_color: TEXT_DIM,
        handle_color: TEXT_DIM,
        background: Background::Color(SURFACE_LIGHT),
        border: Border::default()
            .rounded(RADIUS_SM)
            .color(SOOTMIX_DARK.border_default)
            .width(1.0),
    });

    let pattern_input = text_input("Pattern...", &edit.pattern)
        .on_input(Message::RulePatternChanged)
        .padding([SPACING_XS, SPACING_SM])
        .size(TEXT_SMALL)
        .style(|_theme: &Theme, _status| text_input::Style {
            background: Background::Color(SURFACE_LIGHT),
            border: Border::default()
                .rounded(RADIUS_SM)
                .color(SOOTMIX_DARK.border_default)
                .width(1.0),
            icon: TEXT,
            placeholder: TEXT_DIM,
            value: TEXT,
            selection: PRIMARY,
        });

    let channel_picker = pick_list(
        channel_names,
        if edit.target_channel.is_empty() {
            None
        } else {
            Some(edit.target_channel.clone())
        },
        Message::RuleTargetChannelChanged,
    )
    .placeholder("Select channel...")
    .text_size(TEXT_SMALL)
    .padding([SPACING_XS, SPACING_SM])
    .style(|_theme: &Theme, _status| pick_list::Style {
        text_color: TEXT,
        placeholder_color: TEXT_DIM,
        handle_color: TEXT_DIM,
        background: Background::Color(SURFACE_LIGHT),
        border: Border::default()
            .rounded(RADIUS_SM)
            .color(SOOTMIX_DARK.border_default)
            .width(1.0),
    });

    let priority_input = text_input("Priority", &edit.priority.to_string())
        .on_input(Message::RulePriorityChanged)
        .padding([SPACING_XS, SPACING_SM])
        .size(TEXT_SMALL)
        .width(60)
        .style(|_theme: &Theme, _status| text_input::Style {
            background: Background::Color(SURFACE_LIGHT),
            border: Border::default()
                .rounded(RADIUS_SM)
                .color(SOOTMIX_DARK.border_default)
                .width(1.0),
            icon: TEXT,
            placeholder: TEXT_DIM,
            value: TEXT,
            selection: PRIMARY,
        });

    let cancel_button = button(text("Cancel").size(TEXT_SMALL))
        .padding([SPACING_SM, SPACING])
        .style(|_theme: &Theme, status| {
            let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(Background::Color(if is_hovered {
                    SURFACE_LIGHT
                } else {
                    SURFACE
                })),
                text_color: TEXT,
                border: Border::default().rounded(RADIUS_SM),
                ..button::Style::default()
            }
        })
        .on_press(Message::CancelEditingRule);

    let save_button = button(text("Save").size(TEXT_SMALL))
        .padding([SPACING_SM, SPACING])
        .style(|_theme: &Theme, status| {
            let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(Background::Color(if is_hovered {
                    lighten(PRIMARY, 0.1)
                } else {
                    PRIMARY
                })),
                text_color: SOOTMIX_DARK.canvas,
                border: Border::default().rounded(RADIUS_SM),
                ..button::Style::default()
            }
        })
        .on_press(Message::SaveRoutingRule);

    // === FORM LAYOUT ===
    let form = column![
        text(title).size(TEXT_HEADING).color(TEXT),
        Space::new().height(SPACING_SM),
        row![
            text("Name:").size(TEXT_SMALL).color(TEXT_DIM).width(70),
            name_input,
        ]
        .align_y(Alignment::Center)
        .spacing(SPACING_SM),
        Space::new().height(SPACING_SM),
        row![
            text("Match:").size(TEXT_SMALL).color(TEXT_DIM).width(70),
            match_target_picker,
            match_type_picker,
            pattern_input,
        ]
        .align_y(Alignment::Center)
        .spacing(SPACING_SM),
        Space::new().height(SPACING_SM),
        row![
            text("Route to:").size(TEXT_SMALL).color(TEXT_DIM).width(70),
            channel_picker,
            Space::new().width(SPACING),
            text("Priority:").size(TEXT_SMALL).color(TEXT_DIM),
            priority_input,
        ]
        .align_y(Alignment::Center)
        .spacing(SPACING_SM),
        Space::new().height(SPACING),
        row![
            Space::new().width(Fill),
            cancel_button,
            Space::new().width(SPACING_SM),
            save_button,
        ]
        .align_y(Alignment::Center),
    ]
    .padding(PADDING);

    container(form)
        .width(Fill)
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(BACKGROUND)),
            border: Border::default()
                .rounded(RADIUS)
                .color(PRIMARY)
                .width(2.0),
            ..container::Style::default()
        })
        .into()
}

// ============================================================================
// HELPERS
// ============================================================================

/// Truncate a string with ellipsis.
fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        s.chars()
            .take(max_len.saturating_sub(2))
            .collect::<String>()
            + ".."
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
