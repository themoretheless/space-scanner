//! Mounted volume listing — fast defaults first, full refresh in background.

use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;

use sysinfo::Disks;

#[derive(Debug, Clone)]
pub struct VolumeInfo {
    pub name: String,
    pub mount: PathBuf,
    pub total: u64,
    pub available: u64,
    /// APFS "Capacity Consumed" when known (often >> sum of visible files).
    pub apfs_consumed: Option<u64>,
    /// Local Time Machine / APFS snapshot count on this volume.
    pub snapshot_count: u32,
}

impl VolumeInfo {
    pub fn used(&self) -> u64 {
        self.apfs_consumed
            .unwrap_or_else(|| self.total.saturating_sub(self.available))
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
            apfs_consumed: None,
            snapshot_count: 0,
        });
    }

    if let Ok(entries) = std::fs::read_dir("/Volumes") {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "Macintosh HD" || is_same_file(&path, Path::new("/")) {
                continue;
            }
            out.push(VolumeInfo {
                name,
                mount: path,
                total: 0,
                available: 0,
                apfs_consumed: None,
                snapshot_count: 0,
            });
        }
    }

    if let Some(home) = std::env::var_os("HOME") {
        out.push(VolumeInfo {
            name: "Home".into(),
            mount: PathBuf::from(home),
            total: 0,
            available: 0,
            apfs_consumed: None,
            snapshot_count: 0,
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
                name.trim().to_string()
            };
            Some(VolumeInfo {
                name,
                mount,
                total: d.total_space(),
                available: d.available_space(),
                apfs_consumed: None,
                snapshot_count: 0,
            })
        })
        .collect();

    #[cfg(target_os = "macos")]
    {
        enrich_apfs(&mut out);
    }

    out.sort_by(|a, b| {
        mount_preference(&b.mount)
            .cmp(&mount_preference(&a.mount))
            .then_with(|| b.total.cmp(&a.total))
    });

    let mut unique: Vec<VolumeInfo> = Vec::new();
    for vol in out {
        let dup = unique.iter().any(|u| same_disk(u, &vol));
        if !dup {
            unique.push(vol);
        }
    }

    unique.sort_by_key(|volume| std::cmp::Reverse(volume.used()));
    unique
}

/// Best matching volume row for a scanned root path.
pub fn match_volume<'a>(volumes: &'a [VolumeInfo], root: &str) -> Option<&'a VolumeInfo> {
    let root = Path::new(root);
    volumes
        .iter()
        .filter(|v| root.starts_with(&v.mount) || v.mount.starts_with(root))
        .max_by_key(|v| v.mount.as_os_str().len())
}

#[cfg(target_os = "macos")]
fn enrich_apfs(vols: &mut [VolumeInfo]) {
    for vol in vols.iter_mut() {
        let mount = vol.mount.to_string_lossy();
        let probe = if mount.as_ref() == "/" {
            "/System/Volumes/Data"
        } else {
            mount.as_ref()
        };
        if let Some((consumed, snaps)) = apfs_volume_stats(probe) {
            vol.apfs_consumed = Some(consumed);
            vol.snapshot_count = snaps;
            if vol.total == 0 {
                vol.total = consumed;
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn apfs_volume_stats(mount: &str) -> Option<(u64, u32)> {
    let out = Command::new("diskutil")
        .args(["info", "-plist", mount])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let txt = String::from_utf8_lossy(&out.stdout);
    let consumed =
        plist_u64(&txt, "VolumeUsedSpace").or_else(|| plist_u64(&txt, "CapacityInUse"))?;
    let snaps = snapshot_count_for_mount(mount);
    Some((consumed, snaps))
}

#[cfg(target_os = "macos")]
fn plist_u64(plist: &str, key: &str) -> Option<u64> {
    let key_tag = format!("<key>{key}</key>");
    let idx = plist.find(&key_tag)?;
    let rest = &plist[idx + key_tag.len()..];
    let start = rest.find("<integer>")? + "<integer>".len();
    let end = rest[start..].find("</integer>")? + start;
    rest[start..end].trim().parse().ok()
}

#[cfg(target_os = "macos")]
fn snapshot_count_for_mount(mount: &str) -> u32 {
    let info = Command::new("diskutil")
        .args(["info", "-plist", mount])
        .output()
        .ok();
    let device = info
        .as_ref()
        .and_then(|o| {
            let t = String::from_utf8_lossy(&o.stdout);
            plist_string(&t, "DeviceIdentifier")
        })
        .unwrap_or_default();
    if device.is_empty() {
        return tmutil_snapshot_lines();
    }
    let out = Command::new("diskutil")
        .args(["apfs", "listSnapshots", &device])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let t = String::from_utf8_lossy(&o.stdout);
            t.lines()
                .filter(|l| l.trim_start().starts_with("Name:"))
                .count() as u32
        }
        _ => tmutil_snapshot_lines(),
    }
}

#[cfg(target_os = "macos")]
fn plist_string(plist: &str, key: &str) -> Option<String> {
    let key_tag = format!("<key>{key}</key>");
    let idx = plist.find(&key_tag)?;
    let rest = &plist[idx + key_tag.len()..];
    let start = rest.find("<string>")? + "<string>".len();
    let end = rest[start..].find("</string>")? + start;
    Some(rest[start..end].trim().to_string())
}

#[cfg(target_os = "macos")]
fn tmutil_snapshot_lines() -> u32 {
    Command::new("tmutil")
        .args(["listlocalsnapshots", "/"])
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| l.contains("com.apple."))
                .count() as u32
        })
        .unwrap_or(0)
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
        || s.starts_with("/System/Volumes/CloudStorage")
}

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
    if a.total > 0
        && a.total == b.total
        && a.available == b.available
        && a.name.eq_ignore_ascii_case(&b.name)
    {
        return true;
    }
    let a_s = a.mount.to_string_lossy();
    let b_s = b.mount.to_string_lossy();
    let root_pair = (a_s.as_ref() == "/" && b_s.as_ref() == "/System/Volumes/Data")
        || (b_s.as_ref() == "/" && a_s.as_ref() == "/System/Volumes/Data");
    root_pair && a.total > 0 && a.total == b.total
}

fn is_same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vol(name: &str, mount: &str, total: u64, available: u64) -> VolumeInfo {
        VolumeInfo {
            name: name.into(),
            mount: PathBuf::from(mount),
            total,
            available,
            apfs_consumed: None,
            snapshot_count: 0,
        }
    }

    #[test]
    fn skips_system_auxiliary_mounts() {
        for path in [
            "/System/Volumes/Preboot",
            "/System/Volumes/Update",
            "/System/Volumes/xarts",
            "/System/Volumes/iSCPreboot",
            "/System/Volumes/Hardware",
            "/System/Volumes/VM",
            "/private/var/vm",
            "/dev",
            "/dev/fd",
            "/System/Volumes/CloudStorage",
        ] {
            assert!(should_skip_mount(Path::new(path)), "expected skip: {path}");
        }
        assert!(!should_skip_mount(Path::new("/")));
        assert!(!should_skip_mount(Path::new("/System/Volumes/Data")));
        assert!(!should_skip_mount(Path::new("/Volumes/External")));
    }

    #[test]
    fn prefers_data_volume_and_dedupes_apfs_pair() {
        assert!(
            mount_preference(Path::new("/System/Volumes/Data")) > mount_preference(Path::new("/"))
        );
        assert!(mount_preference(Path::new("/")) > mount_preference(Path::new("/Volumes/Ext")));
        let root = vol("Macintosh HD", "/", 1000, 400);
        let data = vol("Data", "/System/Volumes/Data", 1000, 400);
        let external = vol("External", "/Volumes/External", 2000, 1000);
        assert!(same_disk(&root, &data));
        assert!(!same_disk(&root, &external));
    }

    #[test]
    fn match_volume_picks_deepest_mount_and_used_prefers_apfs_consumed() {
        let mut root = vol("Macintosh HD", "/", 1000, 400);
        root.apfs_consumed = Some(720);
        let ext = vol("SSD", "/Volumes/SSD", 2000, 500);
        let vols = [root.clone(), ext];

        assert_eq!(root.used(), 720);
        assert_eq!(vols[1].used(), 1500);
        assert_eq!(
            match_volume(&vols, "/Volumes/SSD/Projects").map(|v| v.name.as_str()),
            Some("SSD")
        );
        assert_eq!(
            match_volume(&vols, "/Users/me").map(|v| v.name.as_str()),
            Some("Macintosh HD")
        );
    }
}
