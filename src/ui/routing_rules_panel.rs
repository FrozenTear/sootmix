// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Routing rules panel UI component.

use crate::config::{MatchTarget, RoutingRulesConfig};
use crate::message::Message;
use crate::state::EditingRule;
use crate::ui::theme::*;
use iced::widget::{
    button, checkbox, column, container, pick_list, row, scrollable, text, text_input, Space,
};
use iced::{Alignment, Background, Border, Element, Fill, Length, Theme};

/// Create the routing rules panel.
pub fn routing_rules_panel<'a>(
    rules: &'a RoutingRulesConfig,
    editing: Option<&'a EditingRule>,
    channel_names: Vec<String>,
) -> Element<'a, Message> {
    let header = row![
        text("Auto-Routing Rules").size(14).color(TEXT),
        Space::new().width(Fill),
        button(text("+ New Rule").size(11))
            .padding([4, 10])
            .style(|_theme: &Theme, _status| button::Style {
                background: Some(Background::Color(PRIMARY)),
                text_color: TEXT,
                border: Border::default().rounded(BORDER_RADIUS_SMALL),
                ..button::Style::default()
            })
            .on_press(Message::StartEditingRule(None)),
        Space::new().width(SPACING_SMALL),
        button(text("Close").size(11))
            .padding([4, 10])
            .style(|_theme: &Theme, _status| button::Style {
                background: Some(Background::Color(SURFACE_LIGHT)),
                text_color: TEXT,
                border: Border::default().rounded(BORDER_RADIUS_SMALL),
                ..button::Style::default()
            })
            .on_press(Message::CloseRoutingRulesPanel),
    ]
    .align_y(Alignment::Center);

    let content: Element<Message> = if let Some(edit) = editing {
        rule_edit_form(edit, channel_names)
    } else if rules.rules.is_empty() {
        container(
            text("No routing rules defined. Click '+ New Rule' to create one.")
                .size(12)
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

        scrollable(column(rule_items).spacing(SPACING_SMALL))
            .height(Length::Fixed(200.0))
            .into()
    };

    let panel = column![
        header,
        Space::new().height(SPACING_SMALL),
        content,
    ]
    .padding(PADDING);

    container(panel)
        .width(Fill)
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(SURFACE)),
            border: Border::default()
                .rounded(BORDER_RADIUS)
                .color(SURFACE_LIGHT)
                .width(1.0),
            ..container::Style::default()
        })
        .into()
}

/// Create a single rule item row.
fn rule_item<'a>(
    rule: &'a crate::config::RoutingRule,
    index: usize,
    total: usize,
) -> Element<'a, Message> {
    let enabled_checkbox = checkbox(rule.enabled)
        .on_toggle(move |_| Message::ToggleRoutingRule(rule.id))
        .size(14);

    let name_text = text(&rule.name)
        .size(12)
        .color(if rule.enabled { TEXT } else { TEXT_DIM });

    let pattern_text = text(format!(
        "{} {} \"{}\"",
        rule.match_target.display_name(),
        rule.match_type.type_name(),
        truncate(rule.match_type.pattern(), 20)
    ))
    .size(10)
    .color(TEXT_DIM);

    let target_text = text(format!("-> {}", &rule.target_channel))
        .size(11)
        .color(SUCCESS);

    let priority_text = text(format!("P{}", rule.priority))
        .size(10)
        .color(TEXT_DIM);

    // Move up/down buttons
    let up_button: Element<Message> = if index > 0 {
        button(text("^").size(10))
            .padding([2, 6])
            .style(|_theme: &Theme, _status| button::Style {
                background: Some(Background::Color(SURFACE_LIGHT)),
                text_color: TEXT_DIM,
                border: Border::default().rounded(2.0),
                ..button::Style::default()
            })
            .on_press(Message::MoveRoutingRuleUp(rule.id))
            .into()
    } else {
        Space::new().width(22).into()
    };

    let down_button: Element<Message> = if index < total - 1 {
        button(text("v").size(10))
            .padding([2, 6])
            .style(|_theme: &Theme, _status| button::Style {
                background: Some(Background::Color(SURFACE_LIGHT)),
                text_color: TEXT_DIM,
                border: Border::default().rounded(2.0),
                ..button::Style::default()
            })
            .on_press(Message::MoveRoutingRuleDown(rule.id))
            .into()
    } else {
        Space::new().width(22).into()
    };

    let edit_button = button(text("Edit").size(10))
        .padding([2, 8])
        .style(|_theme: &Theme, _status| button::Style {
            background: Some(Background::Color(SURFACE_LIGHT)),
            text_color: TEXT,
            border: Border::default().rounded(2.0),
            ..button::Style::default()
        })
        .on_press(Message::StartEditingRule(Some(rule.id)));

    let delete_button = button(text("X").size(10))
        .padding([2, 6])
        .style(|_theme: &Theme, _status| button::Style {
            background: Some(Background::Color(MUTED_COLOR)),
            text_color: TEXT,
            border: Border::default().rounded(2.0),
            ..button::Style::default()
        })
        .on_press(Message::DeleteRoutingRule(rule.id));

    let rule_content = row![
        enabled_checkbox,
        Space::new().width(SPACING_SMALL),
        column![name_text, pattern_text].spacing(2),
        Space::new().width(SPACING),
        target_text,
        Space::new().width(Fill),
        priority_text,
        Space::new().width(SPACING_SMALL),
        up_button,
        down_button,
        Space::new().width(SPACING_SMALL),
        edit_button,
        Space::new().width(2),
        delete_button,
    ]
    .align_y(Alignment::Center)
    .padding([6, 10]);

    container(rule_content)
        .width(Fill)
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(BACKGROUND)),
            border: Border::default()
                .rounded(BORDER_RADIUS_SMALL)
                .color(SURFACE_LIGHT)
                .width(1.0),
            ..container::Style::default()
        })
        .into()
}

/// Create the rule edit form.
fn rule_edit_form<'a>(edit: &'a EditingRule, channel_names: Vec<String>) -> Element<'a, Message> {
    let is_new = edit.id.is_none();
    let title = if is_new { "New Rule" } else { "Edit Rule" };

    let name_input = text_input("Rule name...", &edit.name)
        .on_input(Message::RuleNameChanged)
        .padding(6)
        .size(12);

    let match_target_options = MatchTarget::all().to_vec();
    let match_target_picker = pick_list(
        match_target_options,
        Some(edit.match_target),
        |target| Message::RuleMatchTargetChanged(target),
    )
    .text_size(11)
    .padding(4);

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
    .text_size(11)
    .padding(4);

    let pattern_input = text_input("Pattern...", &edit.pattern)
        .on_input(Message::RulePatternChanged)
        .padding(6)
        .size(12);

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
    .text_size(11)
    .padding(4);

    let priority_input = text_input("Priority", &edit.priority.to_string())
        .on_input(Message::RulePriorityChanged)
        .padding(6)
        .size(12)
        .width(60);

    let cancel_button = button(text("Cancel").size(11))
        .padding([4, 12])
        .style(|_theme: &Theme, _status| button::Style {
            background: Some(Background::Color(SURFACE_LIGHT)),
            text_color: TEXT,
            border: Border::default().rounded(BORDER_RADIUS_SMALL),
            ..button::Style::default()
        })
        .on_press(Message::CancelEditingRule);

    let save_button = button(text("Save").size(11))
        .padding([4, 12])
        .style(|_theme: &Theme, _status| button::Style {
            background: Some(Background::Color(PRIMARY)),
            text_color: TEXT,
            border: Border::default().rounded(BORDER_RADIUS_SMALL),
            ..button::Style::default()
        })
        .on_press(Message::SaveRoutingRule);

    let form = column![
        text(title).size(13).color(TEXT),
        Space::new().height(SPACING_SMALL),
        row![
            text("Name:").size(11).color(TEXT_DIM).width(70),
            name_input,
        ]
        .align_y(Alignment::Center)
        .spacing(SPACING_SMALL),
        Space::new().height(SPACING_SMALL),
        row![
            text("Match:").size(11).color(TEXT_DIM).width(70),
            match_target_picker,
            match_type_picker,
            pattern_input,
        ]
        .align_y(Alignment::Center)
        .spacing(SPACING_SMALL),
        Space::new().height(SPACING_SMALL),
        row![
            text("Route to:").size(11).color(TEXT_DIM).width(70),
            channel_picker,
            Space::new().width(SPACING),
            text("Priority:").size(11).color(TEXT_DIM),
            priority_input,
        ]
        .align_y(Alignment::Center)
        .spacing(SPACING_SMALL),
        Space::new().height(SPACING),
        row![Space::new().width(Fill), cancel_button, Space::new().width(SPACING_SMALL), save_button,]
            .align_y(Alignment::Center),
    ]
    .padding(PADDING);

    container(form)
        .width(Fill)
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

/// Truncate a string with ellipsis.
fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        s.chars().take(max_len.saturating_sub(2)).collect::<String>() + ".."
    }
}
