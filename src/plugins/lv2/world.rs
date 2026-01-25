// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! LV2 World singleton management.
//!
//! The Lilv World is a global object that manages LV2 plugin discovery and
//! must outlive all plugin instances. We use a global singleton pattern with
//! reference counting to ensure proper lifetime management.

use crate::plugins::PluginLoadError;
use lilv::World;
use std::sync::{Arc, OnceLock};
use tracing::{debug, info};

/// Global LV2 world instance.
static LV2_WORLD: OnceLock<Arc<Lv2World>> = OnceLock::new();

/// Wrapper around Lilv World.
pub struct Lv2World {
    /// The underlying Lilv World.
    inner: World,
}

// SAFETY: The lilv World is internally synchronized and safe to share.
unsafe impl Send for Lv2World {}
unsafe impl Sync for Lv2World {}

impl Lv2World {
    /// Get or create the global LV2 world instance.
    pub fn global() -> Result<Arc<Self>, PluginLoadError> {
        let world = LV2_WORLD.get_or_init(|| {
            match Self::create() {
                Ok(w) => Arc::new(w),
                Err(e) => {
                    panic!("Failed to initialize LV2 world: {}", e);
                }
            }
        });

        Ok(Arc::clone(world))
    }

    /// Create a new LV2 world (internal).
    fn create() -> Result<Self, PluginLoadError> {
        debug!("Initializing LV2 world");

        // Create world and load all installed LV2 plugins
        let world = World::with_load_all();

        let plugin_count = world.plugins().count();
        info!("LV2 world initialized with {} plugins", plugin_count);

        Ok(Self { inner: world })
    }

    /// Get the underlying World.
    pub fn inner(&self) -> &World {
        &self.inner
    }

    /// Get the number of available plugins.
    pub fn plugin_count(&self) -> usize {
        self.inner.plugins().count()
    }
}
