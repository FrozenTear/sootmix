// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Diagnostic utilities: support-report bundling and update checks.

use std::path::PathBuf;
use std::time::Duration;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const RELEASES_URL: &str =
    "https://api.github.com/repos/FrozenTear/sootmix/releases/latest";

#[derive(Debug, Clone)]
pub struct UpdateCheck {
    pub current: String,
    pub latest: String,
    pub newer_available: bool,
}

/// Query the GitHub releases API and compare against the compiled-in version.
pub async fn check_for_updates() -> Result<UpdateCheck, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent(format!("sootmix/{}", VERSION))
        .build()
        .map_err(|e| format!("HTTP client init failed: {}", e))?;

    let body = client
        .get(RELEASES_URL)
        .send()
        .await
        .map_err(|e| format!("request failed: {}", e))?
        .error_for_status()
        .map_err(|e| format!("GitHub returned error: {}", e))?
        .text()
        .await
        .map_err(|e| format!("response read failed: {}", e))?;

    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("response parse failed: {}", e))?;

    let latest = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "tag_name missing from GitHub response".to_string())?
        .to_string();

    let current = format!("v{}", VERSION);
    let newer_available = version_tuple(&latest) > version_tuple(&current);

    Ok(UpdateCheck {
        current,
        latest,
        newer_available,
    })
}

/// Parse "vMAJOR.MINOR.PATCH-N" into comparable tuple. Missing components -> 0.
fn version_tuple(v: &str) -> (u32, u32, u32, u32) {
    let stripped = v.trim_start_matches('v');
    let (base, suffix) = match stripped.split_once('-') {
        Some((b, s)) => (b, Some(s)),
        None => (stripped, None),
    };
    let mut parts = base.split('.');
    let major = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let suffix_n = suffix.and_then(|s| s.parse().ok()).unwrap_or(0);
    (major, minor, patch, suffix_n)
}

/// Collect logs, graph state, and config into a tarball in $HOME.
/// Returns the absolute path of the generated archive on success.
pub async fn generate_report() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "$HOME not set".to_string())?;
    let timestamp = chrono_like_now();
    let dir_name = format!("sootmix-report-{}", timestamp);
    let archive_name = format!("{}.tar.gz", dir_name);
    let archive_path = PathBuf::from(&home).join(&archive_name);

    let journal = run_command("journalctl", &[
        "--user",
        "-u",
        "sootmix-daemon.service",
        "-n",
        "2000",
        "--no-pager",
    ]).await;

    let pw_dump = run_command("pw-dump", &[]).await;
    let pw_link = run_command("pw-link", &["-l"]).await;
    let uname = run_command("uname", &["-a"]).await;
    let pw_version = run_command("pipewire", &["--version"]).await;
    let wp_version = run_command("wireplumber", &["--version"]).await;

    let version_txt = format!(
        "sootmix GUI version: {}\nreport generated: {}\n",
        VERSION, timestamp
    );

    let config_dir = PathBuf::from(&home).join(".config/sootmix");
    let mixer_toml = read_opt(&config_dir.join("mixer.toml")).await;
    let routing_rules_toml = read_opt(&config_dir.join("routing_rules.toml")).await;

    // Build tarball. Use blocking tar crate on a thread since we already
    // have async IO done — this section is CPU/IO but tiny.
    let path_for_task = archive_path.clone();
    let dir_name_task = dir_name.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::fs::File;
        use std::io::Write;

        let file = File::create(&path_for_task)
            .map_err(|e| format!("create archive: {}", e))?;
        let encoder = GzEncoder::new(file, Compression::default());
        let mut tar = tar::Builder::new(encoder);

        let add = |tar: &mut tar::Builder<GzEncoder<File>>,
                   name: &str,
                   data: &[u8]|
         -> Result<(), String> {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_mtime(0);
            header.set_cksum();
            let path = format!("{}/{}", dir_name_task, name);
            tar.append_data(&mut header, path, data)
                .map_err(|e| format!("tar append {}: {}", name, e))
        };

        add(&mut tar, "version.txt", version_txt.as_bytes())?;
        add(&mut tar, "uname.txt", uname.as_bytes())?;
        add(&mut tar, "pipewire-version.txt", pw_version.as_bytes())?;
        add(&mut tar, "wireplumber-version.txt", wp_version.as_bytes())?;
        add(&mut tar, "daemon.log", journal.as_bytes())?;
        add(&mut tar, "pw-dump.json", pw_dump.as_bytes())?;
        add(&mut tar, "pw-link.txt", pw_link.as_bytes())?;
        if let Some(contents) = mixer_toml {
            add(&mut tar, "config/mixer.toml", contents.as_bytes())?;
        }
        if let Some(contents) = routing_rules_toml {
            add(&mut tar, "config/routing_rules.toml", contents.as_bytes())?;
        }

        tar.finish().map_err(|e| format!("tar finalize: {}", e))?;
        let mut encoder = tar.into_inner().map_err(|e| format!("tar unwrap: {}", e))?;
        encoder.flush().map_err(|e| format!("gzip flush: {}", e))?;
        encoder.finish().map_err(|e| format!("gzip finish: {}", e))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("report task panicked: {}", e))??;

    Ok(archive_path)
}

/// Run a command and return stdout (or an error message captured as text,
/// so the report always carries *something* — absence of a binary is itself
/// diagnostic information).
async fn run_command(program: &str, args: &[&str]) -> String {
    match tokio::process::Command::new(program)
        .args(args)
        .output()
        .await
    {
        Ok(out) => {
            let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
            if !out.stderr.is_empty() {
                s.push_str("\n--- stderr ---\n");
                s.push_str(&String::from_utf8_lossy(&out.stderr));
            }
            if !out.status.success() {
                s.push_str(&format!("\n--- exit: {} ---\n", out.status));
            }
            s
        }
        Err(e) => format!("<command `{} {}` failed: {}>\n", program, args.join(" "), e),
    }
}

async fn read_opt(path: &std::path::Path) -> Option<String> {
    tokio::fs::read_to_string(path).await.ok()
}

/// UTC timestamp for tarball filenames: YYYYMMDD-HHMMSS. UTC is fine here —
/// filenames only need to disambiguate reports from the same user, not match
/// their wall clock.
fn chrono_like_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, mo, d, h, mi, s) = ymd_hms(secs);
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", y, mo, d, h, mi, s)
}

/// Convert a Unix-epoch second (UTC) into Y-M-D H:M:S using Howard Hinnant's
/// civil-from-days algorithm.
fn ymd_hms(mut t: i64) -> (i32, u32, u32, u32, u32, u32) {
    let s = t.rem_euclid(60) as u32;
    t = t.div_euclid(60);
    let mi = t.rem_euclid(60) as u32;
    t = t.div_euclid(60);
    let h = t.rem_euclid(24) as u32;
    let mut days = t.div_euclid(24);

    days += 719468;
    let era = if days >= 0 { days / 146097 } else { (days - 146096) / 146097 };
    let doe = (days - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + if mo <= 2 { 1 } else { 0 };
    (y, mo, d, h, mi, s)
}
