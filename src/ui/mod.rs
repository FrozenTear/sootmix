// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! UI components for SootMix.

pub mod apps_panel;
pub mod channel_strip;
pub mod focus_panel;
pub mod layout_drafts;
pub mod meter;
pub mod plugin_chain;
pub mod plugin_downloader;
pub mod routing_rules_panel;
pub mod settings_panel;
pub mod theme;

pub use plugin_downloader::plugin_downloader;
pub use settings_panel::settings_panel;
