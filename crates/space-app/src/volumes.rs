//! Mounted volume listing — fast defaults first, full refresh in background.

use std::path::{Path, PathBuf};

use space_core::format_bytes;
use sysinfo::Disks;

#[derive(Debug, Clone)]
pub struct VolumeInfo {
    pub name: String,
    pub mount: PathBuf,
    pub total: u64,
    pub available: u64,
}

impl VolumeInfo {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    pub fn label(&self) -> String {
        format!(
            "{}  {} / {}",
            self.name,
            format_bytes(self.used()),
            format_bytes(self.total)
        )
    }
}

/// Instant list: `/` + `/Volumes/*` without probing every disk driver.
pub fn list_volumes_fast() -> Vec<VolumeInfo> {
    let mut out = Vec::new();

    if std::fs::metadata("/").is_ok() {
        out.push(VolumeInfo {
            name: "Macintosh HD".into(),
            mount: PathBuf::from("/"),
            total: 0,
            available: 0,
        });
    }

    if let Ok(entries) = std::fs::read_dir("/Volumes") {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            // Root volume also appears here as a symlink/folder — skip it.
            if name == "Macintosh HD" || is_same_file(&path, Path::new("/")) {
                continue;
            }
            out.push(VolumeInfo {
                name,
                mount: path,
                total: 0,
                available: 0,
            });
        }
    }

    if let Some(home) = std::env::var_os("HOME") {
        out.push(VolumeInfo {
            name: "Home".into(),
            mount: PathBuf::from(home),
            total: 0,
            available: 0,
        });
    }

    out
}

/// Full disk stats — can take seconds on macOS; call off the UI thread.
pub fn list_volumes() -> Vec<VolumeInfo> {
    let disks = Disks::new_with_refreshed_list();
    let mut out: Vec<VolumeInfo> = disks
        .list()
        .iter()
        .filter_map(|d| {
            let mount = d.mount_point().to_path_buf();
            if should_skip_mount(&mount) {
                return None;
            }
            let mount_s = mount.to_string_lossy();
            let name = d.name().to_string_lossy().to_string();
            let name = if name.is_empty() {
                mount
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| mount_s.into_owned())
            } else {
                // Strip trailing spaces / APFS role noise.
                name.trim().to_string()
            };
            Some(VolumeInfo {
                name,
                mount,
                total: d.total_space(),
                available: d.available_space(),
            })
        })
        .collect();

    // Prefer the mount users should scan (Data > root > /Volumes/…).
    out.sort_by(|a, b| {
        mount_preference(&b.mount)
            .cmp(&mount_preference(&a.mount))
            .then_with(|| b.total.cmp(&a.total))
    });

    // One entry per physical/logical disk: same capacity + same display name
    // (macOS exposes `/` and `/System/Volumes/Data` as "Macintosh HD").
    let mut unique: Vec<VolumeInfo> = Vec::new();
    for vol in out {
        let dup = unique.iter().any(|u| same_disk(u, &vol));
        if !dup {
            unique.push(vol);
        }
    }

    // Present largest first for the sidebar.
    unique.sort_by(|a, b| b.total.cmp(&a.total));
    unique
}

fn should_skip_mount(mount: &Path) -> bool {
    let s = mount.to_string_lossy();
    s.starts_with("/System/Volumes/Preboot")
        || s.starts_with("/System/Volumes/Update")
        || s.starts_with("/System/Volumes/xarts")
        || s.starts_with("/System/Volumes/iSCPreboot")
        || s.starts_with("/System/Volumes/Hardware")
        || s.starts_with("/System/Volumes/VM")
        || s.starts_with("/private/var/vm")
        || s == "/dev"
        || s.starts_with("/dev/")
        // Time Machine localsnapshots / firmlink scaffolding
        || s.starts_with("/System/Volumes/CloudStorage")
}

/// Higher = preferred when collapsing APFS siblings.
fn mount_preference(mount: &Path) -> u8 {
    let s = mount.to_string_lossy();
    if s == "/System/Volumes/Data" {
        100
    } else if s == "/" {
        90
    } else if s.starts_with("/Volumes/") {
        70
    } else if s.starts_with("/System/Volumes/") {
        20
    } else {
        50
    }
}

fn same_disk(a: &VolumeInfo, b: &VolumeInfo) -> bool {
    if a.mount == b.mount {
        return true;
    }
    // Identical capacity + free + name → same APFS container reported twice.
    if a.total > 0
        && a.total == b.total
        && a.available == b.available
        && a.name.eq_ignore_ascii_case(&b.name)
    {
        return true;
    }
    // `/` vs `/System/Volumes/Data` even if names differ slightly.
    let a_s = a.mount.to_string_lossy();
    let b_s = b.mount.to_string_lossy();
    let root_pair = (a_s.as_ref() == "/" && b_s.as_ref() == "/System/Volumes/Data")
        || (b_s.as_ref() == "/" && a_s.as_ref() == "/System/Volumes/Data");
    if root_pair && a.total > 0 && a.total == b.total {
        return true;
    }
    false
}

fn is_same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}
