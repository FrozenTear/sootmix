// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! VST3 module loading and factory management.

use crate::plugins::PluginLoadError;
use libloading::Library;
use std::path::{Path, PathBuf};
use tracing::debug;
use vst3::ComPtr;
use vst3::Steinberg::Vst::{IComponent, IComponent_iid};
use vst3::Steinberg::{
    IPluginFactory, IPluginFactory2, IPluginFactory2Trait, IPluginFactoryTrait, PClassInfo,
    PClassInfo2, TUID,
};

/// A loaded VST3 module (.vst3 bundle).
pub struct Vst3Module {
    /// Path to the .vst3 bundle.
    bundle_path: PathBuf,
    /// The loaded shared library.
    #[allow(dead_code)]
    library: Library,
    /// The plugin factory.
    factory: ComPtr<IPluginFactory>,
}

// SAFETY: The VST3 module can be sent between threads.
// The factory is reference-counted and thread-safe.
unsafe impl Send for Vst3Module {}
unsafe impl Sync for Vst3Module {}

impl Vst3Module {
    /// Load a VST3 bundle.
    pub fn load(bundle_path: &Path) -> Result<Self, PluginLoadError> {
        // Find the binary inside the bundle
        let binary_path = Self::find_binary(bundle_path)?;

        debug!("Loading VST3 binary: {:?}", binary_path);

        // Load the shared library
        let library = unsafe {
            Library::new(&binary_path)
                .map_err(|e| PluginLoadError::Vst3Error(format!("Failed to load library: {}", e)))?
        };

        // Get the module entry and initialize
        let init_dll: libloading::Symbol<unsafe extern "C" fn() -> bool> = unsafe {
            library.get(b"InitDll\0").map_err(|e| {
                PluginLoadError::Vst3Error(format!("InitDll not found: {}", e))
            })?
        };

        let init_result = unsafe { init_dll() };
        if !init_result {
            return Err(PluginLoadError::Vst3Error(
                "InitDll returned false".to_string(),
            ));
        }

        // Get the factory
        let get_factory: libloading::Symbol<
            unsafe extern "C" fn() -> *mut vst3::Steinberg::IPluginFactory,
        > = unsafe {
            library.get(b"GetPluginFactory\0").map_err(|e| {
                PluginLoadError::Vst3Error(format!("GetPluginFactory not found: {}", e))
            })?
        };

        let factory_ptr = unsafe { get_factory() };
        if factory_ptr.is_null() {
            return Err(PluginLoadError::Vst3Error(
                "GetPluginFactory returned null".to_string(),
            ));
        }

        let factory = unsafe {
            ComPtr::from_raw(factory_ptr).ok_or_else(|| {
                PluginLoadError::Vst3Error("Failed to wrap factory".to_string())
            })?
        };

        Ok(Self {
            bundle_path: bundle_path.to_path_buf(),
            library,
            factory,
        })
    }

    /// Find the binary inside a VST3 bundle.
    fn find_binary(bundle_path: &Path) -> Result<PathBuf, PluginLoadError> {
        // VST3 bundle structure on Linux:
        // MyPlugin.vst3/
        //   Contents/
        //     x86_64-linux/
        //       MyPlugin.so

        let arch = if cfg!(target_arch = "x86_64") {
            "x86_64-linux"
        } else if cfg!(target_arch = "aarch64") {
            "aarch64-linux"
        } else {
            return Err(PluginLoadError::Vst3Error(
                "Unsupported architecture".to_string(),
            ));
        };

        let contents_path = bundle_path.join("Contents").join(arch);

        if !contents_path.exists() {
            return Err(PluginLoadError::Vst3Error(format!(
                "Contents/{} directory not found in bundle",
                arch
            )));
        }

        // Find the .so file
        for entry in std::fs::read_dir(&contents_path).map_err(|e| {
            PluginLoadError::Vst3Error(format!("Failed to read directory: {}", e))
        })? {
            let entry = entry.map_err(|e| {
                PluginLoadError::Vst3Error(format!("Failed to read entry: {}", e))
            })?;

            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("so") {
                return Ok(path);
            }
        }

        Err(PluginLoadError::Vst3Error(
            "No .so file found in bundle".to_string(),
        ))
    }

    /// Get the bundle path.
    pub fn bundle_path(&self) -> &Path {
        &self.bundle_path
    }

    /// Get the plugin factory.
    pub fn factory(&self) -> &ComPtr<IPluginFactory> {
        &self.factory
    }

    /// Get the number of classes in the factory.
    pub fn class_count(&self) -> i32 {
        unsafe { self.factory.countClasses() }
    }

    /// Get class info by index.
    pub fn get_class_info(&self, index: i32) -> Option<PClassInfo> {
        let mut info: PClassInfo = unsafe { std::mem::zeroed() };
        let result = unsafe { self.factory.getClassInfo(index, &mut info) };

        if result == vst3::Steinberg::kResultOk {
            Some(info)
        } else {
            None
        }
    }

    /// Get extended class info (PClassInfo2) if available.
    pub fn get_class_info2(&self, index: i32) -> Option<PClassInfo2> {
        // Try to get IPluginFactory2
        let factory2: Option<ComPtr<IPluginFactory2>> = self.factory.cast();

        if let Some(f2) = factory2 {
            let mut info: PClassInfo2 = unsafe { std::mem::zeroed() };
            let result = unsafe { f2.getClassInfo2(index, &mut info) };

            if result == vst3::Steinberg::kResultOk {
                return Some(info);
            }
        }

        None
    }

    /// Create a component instance by class ID.
    pub fn create_component(&self, class_id: &TUID) -> Result<ComPtr<IComponent>, PluginLoadError> {
        let mut component: *mut std::ffi::c_void = std::ptr::null_mut();

        // createInstance takes FIDString (pointer to char8) for both cid and iid
        let result = unsafe {
            self.factory.createInstance(
                class_id.as_ptr() as *const i8,
                IComponent_iid.as_ptr() as *const i8,
                &mut component,
            )
        };

        if result != vst3::Steinberg::kResultOk || component.is_null() {
            return Err(PluginLoadError::Vst3Error(
                "Failed to create component instance".to_string(),
            ));
        }

        let component = unsafe {
            ComPtr::from_raw(component as *mut IComponent).ok_or_else(|| {
                PluginLoadError::Vst3Error("Failed to wrap component".to_string())
            })?
        };

        Ok(component)
    }
}

impl Drop for Vst3Module {
    fn drop(&mut self) {
        // Call ExitDll before unloading
        if let Ok(exit_dll) = unsafe {
            self.library
                .get::<unsafe extern "C" fn() -> bool>(b"ExitDll\0")
        } {
            unsafe {
                exit_dll();
            }
        }
    }
}

/// Convert a VST3 TUID to a hex string.
pub fn tuid_to_string(tuid: &TUID) -> String {
    tuid.iter()
        .map(|b| format!("{:02X}", *b as u8))
        .collect::<Vec<_>>()
        .join("")
}

/// Parse a hex string to a VST3 TUID.
pub fn string_to_tuid(s: &str) -> Option<TUID> {
    if s.len() != 32 {
        return None;
    }

    let mut tuid: TUID = [0i8; 16];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let hex_str = std::str::from_utf8(chunk).ok()?;
        tuid[i] = u8::from_str_radix(hex_str, 16).ok()? as i8;
    }

    Some(tuid)
}
