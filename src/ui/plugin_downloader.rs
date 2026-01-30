// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Plugin downloader UI components.
//!
//! Provides a modal UI for browsing and downloading LV2 plugin packs
//! from the registry.

use crate::message::Message;
use crate::plugins::registry::{format_file_size, get_available_packs, PluginPack};
use crate::ui::theme::*;
use iced::widget::{button, column, container, progress_bar, row, scrollable, text, text_input, Space};
use iced::{Alignment, Background, Border, Color, Element, Fill, Length, Theme};
use sootmix_plugin_api::PluginCategory;
use std::collections::{HashMap, HashSet};

/// Create the plugin downloader modal panel.
pub fn plugin_downloader<'a>(
    search: &str,
    downloading: &HashMap<String, f32>,
    installed_packs: &HashSet<String>,
) -> Element<'a, Message> {
    // Header with title and close button
    let header = row![
        text("Download Plugins").size(TEXT_HEADING).color(TEXT),
        Space::new().width(Fill),
        button(text("\u{00D7}").size(TEXT_HEADING))
            .padding([SPACING_XS, SPACING_SM])
            .style(|_theme: &Theme, status| {
                let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
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
            .on_press(Message::ClosePluginDownloader),
    ]
    .align_y(Alignment::Center);

    // Search input
    let search_input = text_input("Search plugins...", search)
        .on_input(Message::DownloaderSearchChanged)
        .padding(SPACING_SM)
        .width(Length::Fill)
        .style(|_theme: &Theme, _status| text_input::Style {
            background: Background::Color(SURFACE),
            border: Border::default()
                .rounded(RADIUS_SM)
                .color(SOOTMIX_DARK.border_default)
                .width(1.0),
            icon: TEXT_DIM,
            placeholder: TEXT_DIM,
            value: TEXT,
            selection: PRIMARY,
        });

    // Divider
    let divider = container(Space::new().height(1))
        .width(Length::Fill)
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(SOOTMIX_DARK.border_subtle)),
            ..container::Style::default()
        });

    // Get available packs and filter by search
    let packs = get_available_packs();
    let search_lower = search.to_lowercase();
    let filtered_packs: Vec<&PluginPack> = if search.is_empty() {
        packs.iter().collect()
    } else {
        packs
            .iter()
            .filter(|p| {
                p.name.to_lowercase().contains(&search_lower)
                    || p.vendor.to_lowercase().contains(&search_lower)
                    || p.description.to_lowercase().contains(&search_lower)
                    || p.plugins.iter().any(|plugin| {
                        plugin.name.to_lowercase().contains(&search_lower)
                    })
            })
            .collect()
    };

    // Pack list
    let pack_items: Vec<Element<Message>> = filtered_packs
        .iter()
        .map(|pack| {
            pack_card(
                pack,
                downloading.get(&pack.id).copied(),
                installed_packs.contains(&pack.id),
            )
        })
        .collect();

    let pack_list = if pack_items.is_empty() {
        column![
            Space::new().height(SPACING_LG),
            text("No matching plugins found")
                .size(TEXT_SMALL)
                .color(TEXT_DIM),
        ]
        .align_x(Alignment::Center)
    } else {
        column(pack_items).spacing(SPACING_SM)
    };

    let scrollable_packs = scrollable(pack_list)
        .height(Length::Fixed(400.0))
        .width(Length::Fill);

    // Footer with stats
    let total_plugins: usize = packs.iter().map(|p| p.plugins.len()).sum();
    let total_size: u64 = packs.iter().map(|p| p.file_size).sum();
    let footer = row![
        text(format!(
            "{} packs \u{2022} {} plugins \u{2022} {} total",
            packs.len(),
            total_plugins,
            format_file_size(total_size)
        ))
        .size(TEXT_CAPTION)
        .color(TEXT_DIM),
    ];

    // Main content
    let content = column![
        header,
        Space::new().height(SPACING_SM),
        search_input,
        Space::new().height(SPACING_SM),
        divider,
        Space::new().height(SPACING_SM),
        scrollable_packs,
        Space::new().height(SPACING_SM),
        footer,
    ]
    .padding(PADDING)
    .spacing(SPACING_XS);

    container(content)
        .width(Length::Fixed(480.0))
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

/// Create a card for a single plugin pack.
fn pack_card<'a>(
    pack: &PluginPack,
    download_progress: Option<f32>,
    is_installed: bool,
) -> Element<'a, Message> {
    let pack_id = pack.id.clone();
    let pack_name = pack.name.clone();
    let pack_version = pack.version.clone();
    let pack_vendor = pack.vendor.clone();
    let pack_description = pack.description.clone();
    let plugin_count = pack.plugins.len();
    let is_downloadable = pack.downloadable;

    // Header row with name and version
    let name_row = row![
        text(pack_name).size(TEXT_BODY).color(TEXT),
        Space::new().width(Fill),
        text(format!("v{}", pack_version))
            .size(TEXT_CAPTION)
            .color(TEXT_DIM),
    ]
    .align_y(Alignment::Center);

    // Vendor
    let vendor = text(pack_vendor).size(TEXT_SMALL).color(TEXT_DIM);

    // Description
    let description = text(pack_description)
        .size(TEXT_CAPTION)
        .color(TEXT_DIM);

    // Stats row
    let stats = row![
        text(format!("{} plugins", plugin_count))
            .size(TEXT_CAPTION)
            .color(TEXT_DIM),
        Space::new().width(SPACING_SM),
        text("\u{2022}").size(TEXT_CAPTION).color(TEXT_DIM),
        Space::new().width(SPACING_SM),
        text(format_file_size(pack.file_size))
            .size(TEXT_CAPTION)
            .color(TEXT_DIM),
    ]
    .align_y(Alignment::Center);

    // Category badges
    let categories = get_pack_categories(pack);
    let category_badges: Vec<Element<Message>> = categories
        .into_iter()
        .take(4)
        .map(|cat| category_badge(cat))
        .collect();

    let badges_row = row(category_badges).spacing(SPACING_XS);

    // Action button or progress bar
    let action: Element<Message> = if let Some(progress) = download_progress {
        // Show progress bar (even at 0% to indicate download started)
        let percent = (progress * 100.0) as u32;
        let status_text = if percent == 0 {
            "Starting...".to_string()
        } else if percent >= 100 {
            "Installing...".to_string()
        } else {
            format!("{}%", percent)
        };
        column![
            container(
                progress_bar(0.0..=1.0, progress)
                    .style(|_theme: &Theme| progress_bar::Style {
                        background: Background::Color(SURFACE_LIGHT),
                        bar: Background::Color(PRIMARY),
                        border: Border::default().rounded(3.0),
                    })
            )
            .width(Length::Fixed(100.0)),
            text(status_text)
                .size(TEXT_CAPTION)
                .color(PRIMARY),
        ]
        .spacing(SPACING_XS)
        .into()
    } else if is_installed {
        // Show installed badge
        container(
            text("Installed \u{2713}")
                .size(TEXT_SMALL)
                .color(SUCCESS),
        )
        .padding([SPACING_XS, SPACING_SM])
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(Color { a: 0.15, ..SUCCESS })),
            border: Border::default().rounded(RADIUS_SM),
            ..container::Style::default()
        })
        .into()
    } else if !is_downloadable {
        // Show unavailable badge for unsupported archive formats
        container(
            text("Use package manager")
                .size(TEXT_SMALL)
                .color(WARNING),
        )
        .padding([SPACING_XS, SPACING_SM])
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(Color { a: 0.15, ..WARNING })),
            border: Border::default().rounded(RADIUS_SM),
            ..container::Style::default()
        })
        .into()
    } else {
        // Show download button
        button(
            text("Download")
                .size(TEXT_SMALL)
                .color(SOOTMIX_DARK.canvas),
        )
        .padding([SPACING_XS, SPACING_SM])
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
        .on_press(Message::DownloadPack(pack_id))
        .into()
    };

    // Main content layout
    let content = column![
        name_row,
        vendor,
        Space::new().height(SPACING_XS),
        description,
        Space::new().height(SPACING_SM),
        badges_row,
        Space::new().height(SPACING_SM),
        row![
            stats,
            Space::new().width(Fill),
            action,
        ]
        .align_y(Alignment::Center),
    ]
    .spacing(SPACING_XS);

    container(content)
        .padding(SPACING_SM)
        .width(Length::Fill)
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

/// Create a category badge.
fn category_badge<'a>(category: PluginCategory) -> Element<'a, Message> {
    let (label, color) = match category {
        PluginCategory::Eq => ("EQ", PRIMARY),
        PluginCategory::Dynamics => ("Dynamics", ACCENT),
        PluginCategory::Reverb => ("Reverb", SOOTMIX_DARK.accent_warm),
        PluginCategory::Delay => ("Delay", SOOTMIX_DARK.accent_warm),
        PluginCategory::Modulation => ("Modulation", WARNING),
        PluginCategory::Distortion => ("Distortion", MUTED_COLOR),
        PluginCategory::Utility => ("Utility", TEXT_DIM),
        PluginCategory::Analyzer => ("Analyzer", SUCCESS),
        PluginCategory::Filter => ("Filter", PRIMARY),
        PluginCategory::Generator => ("Generator", ACCENT),
        PluginCategory::Synth => ("Synth", SOOTMIX_DARK.accent_warm),
        PluginCategory::Other => ("Other", TEXT_DIM),
    };

    container(
        text(label)
            .size(TEXT_CAPTION)
            .color(TEXT),
    )
    .padding([2, SPACING_XS as u16])
    .style(move |_theme: &Theme| container::Style {
        background: Some(Background::Color(Color { a: 0.2, ..color })),
        border: Border::default().rounded(3.0),
        ..container::Style::default()
    })
    .into()
}

/// Get unique categories from a pack's plugins, sorted by count (most common first).
fn get_pack_categories(pack: &PluginPack) -> Vec<PluginCategory> {
    // Count occurrences of each category
    let mut category_counts: HashMap<PluginCategory, usize> = HashMap::new();
    for plugin in &pack.plugins {
        *category_counts.entry(plugin.category).or_insert(0) += 1;
    }

    // Convert to vec and sort by count (descending), then by discriminant for stability
    let mut categories: Vec<(PluginCategory, usize)> = category_counts.into_iter().collect();
    categories.sort_by(|a, b| {
        b.1.cmp(&a.1).then_with(|| {
            // Secondary sort by category name for stability
            format!("{:?}", a.0).cmp(&format!("{:?}", b.0))
        })
    });

    categories.into_iter().map(|(cat, _)| cat).collect()
}

/// Lighten a color by a factor (0.0-1.0).
fn lighten(color: Color, factor: f32) -> Color {
    Color::from_rgb(
        (color.r + (1.0 - color.r) * factor).min(1.0),
        (color.g + (1.0 - color.g) * factor).min(1.0),
        (color.b + (1.0 - color.b) * factor).min(1.0),
    )
}
