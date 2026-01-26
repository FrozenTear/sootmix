// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Auto-routing rules for automatic app-to-channel assignment.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// How to match the app identifier.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "pattern")]
pub enum MatchType {
    /// Pattern must be contained in the target.
    Contains(String),
    /// Pattern must exactly match the target.
    Exact(String),
    /// Pattern is a regular expression.
    Regex(String),
    /// Pattern is a glob pattern (*, ?, [abc]).
    Glob(String),
}

impl MatchType {
    /// Get the pattern string.
    pub fn pattern(&self) -> &str {
        match self {
            MatchType::Contains(p) => p,
            MatchType::Exact(p) => p,
            MatchType::Regex(p) => p,
            MatchType::Glob(p) => p,
        }
    }

    /// Get the match type name.
    pub fn type_name(&self) -> &'static str {
        match self {
            MatchType::Contains(_) => "contains",
            MatchType::Exact(_) => "exact",
            MatchType::Regex(_) => "regex",
            MatchType::Glob(_) => "glob",
        }
    }

    /// Check if a value matches this pattern.
    pub fn matches(&self, value: &str) -> bool {
        match self {
            MatchType::Contains(pattern) => {
                value.to_lowercase().contains(&pattern.to_lowercase())
            }
            MatchType::Exact(pattern) => {
                value.eq_ignore_ascii_case(pattern)
            }
            MatchType::Regex(pattern) => {
                match Regex::new(pattern) {
                    Ok(re) => re.is_match(value),
                    Err(_) => false,
                }
            }
            MatchType::Glob(pattern) => {
                glob_match(pattern, value)
            }
        }
    }
}

/// What field to match against.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub enum MatchTarget {
    /// Match against the app name.
    #[default]
    AppName,
    /// Match against the binary name.
    Binary,
    /// Match against either app name or binary.
    Either,
}

impl MatchTarget {
    /// Get all variants for UI selection.
    pub fn all() -> &'static [MatchTarget] {
        &[MatchTarget::AppName, MatchTarget::Binary, MatchTarget::Either]
    }

    /// Get display name.
    pub fn display_name(&self) -> &'static str {
        match self {
            MatchTarget::AppName => "App Name",
            MatchTarget::Binary => "Binary",
            MatchTarget::Either => "Either",
        }
    }
}

impl fmt::Display for MatchTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// How to handle multiple audio streams from the same application.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub enum AppGrouping {
    /// Route all nodes with the same app identifier together (default).
    /// When an app matches a rule, all its audio streams go to the same channel.
    #[default]
    GroupByApp,
    /// Route each node individually.
    /// Each audio stream can potentially be routed to different channels.
    Individual,
}

impl AppGrouping {
    /// Get all variants for UI selection.
    pub fn all() -> &'static [AppGrouping] {
        &[AppGrouping::GroupByApp, AppGrouping::Individual]
    }

    /// Get display name.
    pub fn display_name(&self) -> &'static str {
        match self {
            AppGrouping::GroupByApp => "Group by App",
            AppGrouping::Individual => "Individual Streams",
        }
    }

    /// Get description for UI tooltip.
    pub fn description(&self) -> &'static str {
        match self {
            AppGrouping::GroupByApp => "Route all audio streams from the same app together",
            AppGrouping::Individual => "Route each audio stream independently",
        }
    }
}

impl fmt::Display for AppGrouping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// A routing rule that automatically routes matching apps to a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRule {
    /// Unique identifier.
    pub id: Uuid,
    /// Human-readable name for the rule.
    pub name: String,
    /// What to match against.
    pub match_target: MatchTarget,
    /// How to match.
    pub match_type: MatchType,
    /// Target channel name to route to.
    pub target_channel: String,
    /// Whether this rule is active.
    pub enabled: bool,
    /// Priority (lower = higher priority, evaluated first).
    pub priority: u32,
}

impl RoutingRule {
    /// Create a new routing rule.
    pub fn new(name: impl Into<String>, pattern: impl Into<String>, target_channel: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            match_target: MatchTarget::Either,
            match_type: MatchType::Contains(pattern.into()),
            target_channel: target_channel.into(),
            enabled: true,
            priority: 100,
        }
    }

    /// Check if an app matches this rule.
    pub fn matches(&self, app_name: &str, binary: Option<&str>) -> bool {
        if !self.enabled {
            return false;
        }

        match self.match_target {
            MatchTarget::AppName => self.match_type.matches(app_name),
            MatchTarget::Binary => {
                binary.map(|b| self.match_type.matches(b)).unwrap_or(false)
            }
            MatchTarget::Either => {
                self.match_type.matches(app_name)
                    || binary.map(|b| self.match_type.matches(b)).unwrap_or(false)
            }
        }
    }
}

/// Collection of routing rules with persistence support.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoutingRulesConfig {
    /// List of routing rules, ordered by priority.
    pub rules: Vec<RoutingRule>,
    /// How to handle multiple audio streams from the same app.
    #[serde(default)]
    pub app_grouping: AppGrouping,
}

impl RoutingRulesConfig {
    /// Create an empty config.
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            app_grouping: AppGrouping::default(),
        }
    }

    /// Parse from TOML string.
    pub fn from_toml(content: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(content)
    }

    /// Serialize to TOML string.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    /// Sort rules by priority.
    pub fn sort_by_priority(&mut self) {
        self.rules.sort_by_key(|r| r.priority);
    }

    /// Find the first matching rule for an app.
    pub fn find_match(&self, app_name: &str, binary: Option<&str>) -> Option<&RoutingRule> {
        // Rules are expected to be sorted by priority
        self.rules.iter().find(|r| r.matches(app_name, binary))
    }

    /// Add a new rule.
    pub fn add_rule(&mut self, rule: RoutingRule) {
        self.rules.push(rule);
        self.sort_by_priority();
    }

    /// Remove a rule by ID.
    pub fn remove_rule(&mut self, id: Uuid) -> Option<RoutingRule> {
        if let Some(pos) = self.rules.iter().position(|r| r.id == id) {
            Some(self.rules.remove(pos))
        } else {
            None
        }
    }

    /// Get a rule by ID.
    pub fn get_rule(&self, id: Uuid) -> Option<&RoutingRule> {
        self.rules.iter().find(|r| r.id == id)
    }

    /// Get a mutable rule by ID.
    pub fn get_rule_mut(&mut self, id: Uuid) -> Option<&mut RoutingRule> {
        self.rules.iter_mut().find(|r| r.id == id)
    }

    /// Toggle a rule's enabled state.
    pub fn toggle_rule(&mut self, id: Uuid) -> bool {
        if let Some(rule) = self.get_rule_mut(id) {
            rule.enabled = !rule.enabled;
            rule.enabled
        } else {
            false
        }
    }

    /// Move a rule up in priority (decrease priority value).
    pub fn move_up(&mut self, id: Uuid) {
        if let Some(pos) = self.rules.iter().position(|r| r.id == id) {
            if pos > 0 {
                // Swap priorities with previous rule
                let prev_priority = self.rules[pos - 1].priority;
                let curr_priority = self.rules[pos].priority;
                self.rules[pos - 1].priority = curr_priority;
                self.rules[pos].priority = prev_priority;
                self.sort_by_priority();
            }
        }
    }

    /// Move a rule down in priority (increase priority value).
    pub fn move_down(&mut self, id: Uuid) {
        if let Some(pos) = self.rules.iter().position(|r| r.id == id) {
            if pos < self.rules.len() - 1 {
                // Swap priorities with next rule
                let next_priority = self.rules[pos + 1].priority;
                let curr_priority = self.rules[pos].priority;
                self.rules[pos + 1].priority = curr_priority;
                self.rules[pos].priority = next_priority;
                self.sort_by_priority();
            }
        }
    }
}

/// Simple glob pattern matching.
/// Supports: * (any chars), ? (single char), [abc] (char class)
fn glob_match(pattern: &str, value: &str) -> bool {
    let pattern = pattern.to_lowercase();
    let value = value.to_lowercase();
    glob_match_impl(&pattern, &value)
}

fn glob_match_impl(pattern: &str, value: &str) -> bool {
    let mut pattern_chars = pattern.chars().peekable();
    let mut value_chars = value.chars().peekable();

    while let Some(pc) = pattern_chars.next() {
        match pc {
            '*' => {
                // Consume consecutive stars
                while pattern_chars.peek() == Some(&'*') {
                    pattern_chars.next();
                }
                // Collect remaining pattern
                let remaining_pattern: String = pattern_chars.collect();
                if remaining_pattern.is_empty() {
                    return true; // * at end matches everything
                }
                // Try matching remaining pattern at each position
                let remaining_value: String = value_chars.collect();
                for i in 0..=remaining_value.len() {
                    if glob_match_impl(&remaining_pattern, &remaining_value[i..]) {
                        return true;
                    }
                }
                return false;
            }
            '?' => {
                if value_chars.next().is_none() {
                    return false; // No char to match
                }
            }
            '[' => {
                // Character class
                let mut class_chars = Vec::new();
                let mut negated = false;

                if pattern_chars.peek() == Some(&'!') || pattern_chars.peek() == Some(&'^') {
                    negated = true;
                    pattern_chars.next();
                }

                while let Some(c) = pattern_chars.next() {
                    if c == ']' {
                        break;
                    }
                    class_chars.push(c);
                }

                let vc = match value_chars.next() {
                    Some(c) => c,
                    None => return false,
                };

                let in_class = class_chars.contains(&vc);
                if (in_class && negated) || (!in_class && !negated) {
                    return false;
                }
            }
            c => {
                // Literal character match
                match value_chars.next() {
                    Some(vc) if vc == c => continue,
                    _ => return false,
                }
            }
        }
    }

    // Pattern consumed, check if value is also consumed
    value_chars.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contains_match() {
        let mt = MatchType::Contains("fire".to_string());
        assert!(mt.matches("Firefox"));
        assert!(mt.matches("firefox"));
        assert!(mt.matches("FIREFOX Audio"));
        assert!(!mt.matches("Chrome"));
    }

    #[test]
    fn test_exact_match() {
        let mt = MatchType::Exact("firefox".to_string());
        assert!(mt.matches("Firefox"));
        assert!(mt.matches("firefox"));
        assert!(!mt.matches("Firefox Audio"));
    }

    #[test]
    fn test_regex_match() {
        let mt = MatchType::Regex(r"^(firefox|chrome)$".to_string());
        assert!(mt.matches("firefox"));
        assert!(mt.matches("chrome"));
        assert!(!mt.matches("Firefox")); // Regex is case-sensitive
        assert!(!mt.matches("firefox-esr"));
    }

    #[test]
    fn test_regex_case_insensitive() {
        let mt = MatchType::Regex(r"(?i)^firefox$".to_string());
        assert!(mt.matches("firefox"));
        assert!(mt.matches("Firefox"));
        assert!(mt.matches("FIREFOX"));
    }

    #[test]
    fn test_glob_match() {
        assert!(glob_match("fire*", "firefox"));
        assert!(glob_match("fire*", "Firefox"));
        assert!(glob_match("*fox", "firefox"));
        assert!(glob_match("*fox*", "firefox audio"));
        assert!(glob_match("fire???", "firefox"));
        assert!(!glob_match("fire???", "firefoxes"));
        assert!(glob_match("[fc]hrome", "chrome"));
        assert!(!glob_match("[fc]hrome", "bhrome"));
    }

    #[test]
    fn test_routing_rule_match() {
        let rule = RoutingRule::new("Discord", "discord", "Communication");

        assert!(rule.matches("Discord", Some("discord")));
        assert!(rule.matches("Discord Voice", Some("discord")));
        assert!(rule.matches("Something", Some("discord")));
        assert!(!rule.matches("Spotify", Some("spotify")));
    }

    #[test]
    fn test_routing_rule_binary_only() {
        let mut rule = RoutingRule::new("Discord Binary", "discord", "Communication");
        rule.match_target = MatchTarget::Binary;

        assert!(!rule.matches("Discord", None));
        assert!(rule.matches("Something", Some("discord")));
        assert!(!rule.matches("Discord", Some("other-binary")));
    }
}
