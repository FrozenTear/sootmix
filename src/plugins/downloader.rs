// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Plugin download manager for fetching and installing LV2 plugin packs.
//!
//! Downloads tarballs from GitHub releases, extracts them, and installs
//! LV2 bundles to ~/.lv2/ for automatic discovery.

#![allow(dead_code, unused_imports)]

use crate::plugins::registry::PluginPack;
use std::path::PathBuf;
use thiserror::Error;
use tracing::{debug, error, info, warn};

/// Errors that can occur during plugin download/installation.
#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("HTTP request failed: {0}")]
    Http(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Extraction failed: {0}")]
    Extraction(String),

    #[error("Invalid archive format: {0}")]
    InvalidFormat(String),

    #[error("Download cancelled")]
    Cancelled,

    #[error("Plugin pack not found: {0}")]
    NotFound(String),
}

/// Result type for download operations.
pub type DownloadResult<T> = Result<T, DownloadError>;

/// Download manager for plugin packs.
pub struct DownloadManager {
    /// HTTP client for downloads.
    client: reqwest::Client,
    /// Target directory for LV2 plugins.
    lv2_dir: PathBuf,
    /// Temporary directory for downloads.
    temp_dir: PathBuf,
}

impl Default for DownloadManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DownloadManager {
    /// Create a new download manager.
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        let lv2_dir = home.join(".lv2");
        let temp_dir = std::env::temp_dir().join("sootmix-downloads");

        Self {
            client: reqwest::Client::builder()
                .user_agent("SootMix Plugin Downloader")
                .build()
                .unwrap_or_default(),
            lv2_dir,
            temp_dir,
        }
    }

    /// Get the LV2 installation directory.
    pub fn lv2_dir(&self) -> &PathBuf {
        &self.lv2_dir
    }

    /// Download and install a plugin pack.
    ///
    /// # Arguments
    /// * `pack` - The plugin pack to download
    /// * `progress_tx` - Channel to send progress updates (0.0 to 1.0)
    pub async fn download_pack(
        &self,
        pack: &PluginPack,
        progress_tx: tokio::sync::mpsc::Sender<f32>,
    ) -> DownloadResult<()> {
        info!("Starting download of {} v{}", pack.name, pack.version);

        // Ensure directories exist
        std::fs::create_dir_all(&self.temp_dir)?;
        std::fs::create_dir_all(&self.lv2_dir)?;

        // Determine file extension from URL
        let extension = if pack.download_url.ends_with(".7z") {
            ".7z"
        } else if pack.download_url.ends_with(".tar.xz") {
            ".tar.xz"
        } else if pack.download_url.ends_with(".tar.gz") {
            ".tar.gz"
        } else if pack.download_url.ends_with(".tgz") {
            ".tgz"
        } else if pack.download_url.ends_with(".txz") {
            ".txz"
        } else {
            ".tar.gz" // Default fallback
        };

        // Download the archive
        let archive_path = self.temp_dir.join(format!("{}{}", pack.id, extension));
        self.download_file(&pack.download_url, &archive_path, pack.file_size, progress_tx.clone()).await?;

        // Extract the archive (progress 0.8 - 0.95)
        let _ = progress_tx.send(0.8).await;
        self.extract_archive(&archive_path, &pack.id).await?;

        // Cleanup temp file
        let _ = progress_tx.send(0.95).await;
        if let Err(e) = std::fs::remove_file(&archive_path) {
            warn!("Failed to cleanup temp file {:?}: {}", archive_path, e);
        }

        let _ = progress_tx.send(1.0).await;
        info!("Successfully installed {} v{}", pack.name, pack.version);

        Ok(())
    }

    /// Download a file with progress reporting.
    async fn download_file(
        &self,
        url: &str,
        dest: &PathBuf,
        expected_size: u64,
        progress_tx: tokio::sync::mpsc::Sender<f32>,
    ) -> DownloadResult<()> {
        use tokio::io::AsyncWriteExt;

        debug!("Downloading {} to {:?}", url, dest);

        let response = self.client
            .get(url)
            .send()
            .await
            .map_err(|e| DownloadError::Http(e.to_string()))?;

        if !response.status().is_success() {
            return Err(DownloadError::Http(format!(
                "HTTP {} for {}",
                response.status(),
                url
            )));
        }

        let total_size = response.content_length().unwrap_or(expected_size);
        let mut file = tokio::fs::File::create(dest).await?;
        let mut downloaded: u64 = 0;

        let mut stream = response.bytes_stream();
        use futures::StreamExt;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| DownloadError::Http(e.to_string()))?;
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;

            // Report progress (0.0 - 0.8 for download phase)
            let progress = (downloaded as f32 / total_size as f32) * 0.8;
            let _ = progress_tx.send(progress).await;
        }

        file.flush().await?;
        debug!("Download complete: {} bytes", downloaded);

        Ok(())
    }

    /// Extract an archive and install LV2 bundles.
    /// Supports .tar.gz, .tar.xz, and .7z formats.
    async fn extract_archive(&self, archive_path: &PathBuf, pack_id: &str) -> DownloadResult<()> {
        debug!("Extracting {:?}", archive_path);

        let filename = archive_path.to_string_lossy();

        // Extract to temp directory first
        let extract_dir = self.temp_dir.join(format!("{}-extract", pack_id));
        if extract_dir.exists() {
            std::fs::remove_dir_all(&extract_dir)?;
        }
        std::fs::create_dir_all(&extract_dir)?;

        // Detect format and extract
        if filename.ends_with(".7z") {
            // 7z archive
            sevenz_rust::decompress_file(archive_path, &extract_dir)
                .map_err(|e| DownloadError::Extraction(e.to_string()))?;
        } else if filename.ends_with(".tar.xz") || filename.ends_with(".txz") {
            // XZ compressed tarball
            use tar::Archive;
            use xz2::read::XzDecoder;
            let file = std::fs::File::open(archive_path)?;
            let decoder = XzDecoder::new(file);
            let mut archive = Archive::new(decoder);
            archive
                .unpack(&extract_dir)
                .map_err(|e| DownloadError::Extraction(e.to_string()))?;
        } else if filename.ends_with(".tar.gz") || filename.ends_with(".tgz") {
            // Gzip compressed tarball
            use tar::Archive;
            use flate2::read::GzDecoder;
            let file = std::fs::File::open(archive_path)?;
            let decoder = GzDecoder::new(file);
            let mut archive = Archive::new(decoder);
            archive
                .unpack(&extract_dir)
                .map_err(|e| DownloadError::Extraction(e.to_string()))?;
        } else {
            return Err(DownloadError::InvalidFormat(format!(
                "Unsupported archive format: {}",
                filename
            )));
        }

        // Find and move LV2 bundles
        self.install_lv2_bundles(&extract_dir).await?;

        // Cleanup extraction directory
        if let Err(e) = std::fs::remove_dir_all(&extract_dir) {
            warn!("Failed to cleanup extraction dir {:?}: {}", extract_dir, e);
        }

        Ok(())
    }

    /// Find LV2 bundles in the extracted directory and install them.
    async fn install_lv2_bundles(&self, extract_dir: &PathBuf) -> DownloadResult<()> {
        let mut installed = 0;

        // Walk the directory looking for .lv2 bundles
        for entry in walkdir::WalkDir::new(extract_dir)
            .min_depth(1)
            .max_depth(5)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name() {
                    if name.to_string_lossy().ends_with(".lv2") {
                        let dest = self.lv2_dir.join(name);

                        // Remove existing bundle if present
                        if dest.exists() {
                            debug!("Removing existing bundle: {:?}", dest);
                            std::fs::remove_dir_all(&dest)?;
                        }

                        // Copy the bundle
                        debug!("Installing bundle: {:?} -> {:?}", path, dest);
                        copy_dir_recursive(path, &dest)?;
                        installed += 1;
                    }
                }
            }
        }

        info!("Installed {} LV2 bundles to {:?}", installed, self.lv2_dir);

        if installed == 0 {
            warn!("No LV2 bundles found in archive");
        }

        Ok(())
    }

    /// Check if a plugin pack is already installed.
    pub fn is_pack_installed(&self, pack: &PluginPack) -> bool {
        // Check if any of the pack's plugin URIs correspond to installed bundles
        // This is a heuristic - we look for bundles with matching names

        if !self.lv2_dir.exists() {
            return false;
        }

        // Check for known bundle names based on pack ID
        let expected_bundles: Vec<&str> = match pack.id.as_str() {
            "lsp-plugins" => vec!["lsp-plugins.lv2"],
            "calf-plugins" => vec!["calf.lv2"],
            "x42-plugins" => vec!["fil4.lv2", "meters.lv2", "darc.lv2"],
            "zam-plugins" => vec!["zamcomp.lv2", "zamtube.lv2", "zamgate.lv2"],
            _ => vec![],
        };

        expected_bundles.iter().any(|bundle| {
            self.lv2_dir.join(bundle).exists()
        })
    }

    /// Get list of installed pack IDs.
    pub fn get_installed_packs(&self) -> Vec<String> {
        use crate::plugins::registry::get_available_packs;

        get_available_packs()
            .iter()
            .filter(|pack| self.is_pack_installed(pack))
            .map(|pack| pack.id.clone())
            .collect()
    }
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &std::path::Path, dst: &PathBuf) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;

    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let entry_path = entry.path();
        let dest_path = dst.join(entry.file_name());

        if entry_path.is_dir() {
            copy_dir_recursive(&entry_path, &dest_path)?;
        } else {
            std::fs::copy(&entry_path, &dest_path)?;
        }
    }

    Ok(())
}

/// Module with blocking versions of async functions for use in sync contexts.
pub mod blocking {
    use super::*;

    /// Check if a pack is installed (sync version).
    pub fn is_pack_installed(pack_id: &str) -> bool {
        use crate::plugins::registry::get_pack_by_id;

        let manager = DownloadManager::new();
        if let Some(pack) = get_pack_by_id(pack_id) {
            manager.is_pack_installed(&pack)
        } else {
            false
        }
    }

    /// Get list of installed pack IDs (sync version).
    pub fn get_installed_packs() -> Vec<String> {
        DownloadManager::new().get_installed_packs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_download_manager_creation() {
        let manager = DownloadManager::new();
        assert!(manager.lv2_dir.ends_with(".lv2"));
    }
}
