//! Local Time Machine / APFS snapshot listing and cleanup (macOS).

#[cfg(target_os = "macos")]
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub enum SnapKind {
    TimeMachine,
    OsUpdate,
    Other,
}

#[derive(Debug, Clone)]
pub struct LocalSnapshot {
    pub name: String,
    /// `YYYY-MM-DD-HHMMSS` for TM `tmutil deletelocalsnapshots`, if applicable.
    pub date: Option<String>,
    pub kind: SnapKind,
    pub purgeable: bool,
}

impl LocalSnapshot {
    pub fn label(&self) -> String {
        match (&self.date, self.kind) {
            (Some(d), SnapKind::TimeMachine) => format_tm_date(d),
            _ => short_name(&self.name),
        }
    }

    pub fn kind_label(&self) -> &'static str {
        match self.kind {
            SnapKind::TimeMachine => "Time Machine",
            SnapKind::OsUpdate => "OS update",
            SnapKind::Other => "Snapshot",
        }
    }

    pub fn is_tm(&self) -> bool {
        self.kind == SnapKind::TimeMachine
    }
}

#[derive(Debug, Clone)]
pub struct CleanReport {
    pub before: u32,
    pub deleted: u32,
    pub kept: u32,
    pub errors: Vec<String>,
}

/// All local APFS snapshots (TM + OS update + other), oldest first.
pub fn list_local_snapshots() -> Vec<LocalSnapshot> {
    #[cfg(target_os = "macos")]
    {
        // tmutil is the complete list; diskutil often returns a subset (or fails
        // inside a translocated .app) and used to hide ~15 snapshots.
        let mut by_name = std::collections::BTreeMap::<String, LocalSnapshot>::new();
        for snap in list_via_tmutil() {
            by_name.insert(snap.name.clone(), snap);
        }
        for snap in list_via_diskutil() {
            by_name
                .entry(snap.name.clone())
                .and_modify(|s| s.purgeable = s.purgeable || snap.purgeable)
                .or_insert(snap);
        }
        let mut snaps: Vec<_> = by_name.into_values().collect();
        snaps.sort_by(|a, b| {
            a.date
                .as_deref()
                .unwrap_or(&a.name)
                .cmp(b.date.as_deref().unwrap_or(&b.name))
        });
        snaps
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

/// Time Machine locals only.
pub fn list_tm_locals() -> Vec<LocalSnapshot> {
    list_local_snapshots()
        .into_iter()
        .filter(|s| s.is_tm())
        .collect()
}

#[cfg(target_os = "macos")]
fn list_via_diskutil() -> Vec<LocalSnapshot> {
    let device = volume_device("/System/Volumes/Data")
        .or_else(|| volume_device("/"))
        .unwrap_or_default();
    if device.is_empty() {
        return Vec::new();
    }
    let out = Command::new("diskutil")
        .args(["apfs", "listSnapshots", &device])
        .output();
    let Ok(o) = out else {
        return Vec::new();
    };
    if !o.status.success() {
        return Vec::new();
    }
    parse_diskutil_snapshots(&String::from_utf8_lossy(&o.stdout))
}

#[cfg(target_os = "macos")]
fn parse_diskutil_snapshots(text: &str) -> Vec<LocalSnapshot> {
    let mut snaps = Vec::new();
    let mut cur_name: Option<String> = None;
    let mut cur_purgeable = false;
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Name:") {
            if let Some(name) = cur_name.take() {
                snaps.push(classify_snapshot(name, cur_purgeable));
            }
            cur_name = Some(rest.trim().to_string());
            cur_purgeable = false;
        } else if t.starts_with("Purgeable:") {
            cur_purgeable = t.to_ascii_lowercase().contains("yes");
        }
    }
    if let Some(name) = cur_name {
        snaps.push(classify_snapshot(name, cur_purgeable));
    }
    snaps
}

#[cfg(target_os = "macos")]
fn list_via_tmutil() -> Vec<LocalSnapshot> {
    let out = Command::new("tmutil")
        .args(["listlocalsnapshots", "/"])
        .output();
    let Ok(o) = out else {
        return Vec::new();
    };
    let mut snaps = Vec::new();
    for line in String::from_utf8_lossy(&o.stdout).lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("Snapshots for") {
            continue;
        }
        if line.starts_with("com.apple.") {
            snaps.push(classify_snapshot(line.to_string(), true));
        }
    }
    snaps
}

#[cfg(target_os = "macos")]
fn classify_snapshot(name: String, purgeable: bool) -> LocalSnapshot {
    let kind = if name.contains("TimeMachine") {
        SnapKind::TimeMachine
    } else if name.contains("os.update") || name.contains("OSUpdate") {
        SnapKind::OsUpdate
    } else {
        SnapKind::Other
    };
    let date = name
        .strip_prefix("com.apple.TimeMachine.")
        .map(|rest| rest.strip_suffix(".local").unwrap_or(rest).to_string())
        .filter(|d| d.len() >= 15 && d.chars().all(|c| c.is_ascii_digit() || c == '-'));
    LocalSnapshot {
        name,
        date,
        kind,
        purgeable,
    }
}

#[cfg(target_os = "macos")]
fn volume_device(mount: &str) -> Option<String> {
    let out = Command::new("diskutil")
        .args(["info", "-plist", mount])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let txt = String::from_utf8_lossy(&out.stdout);
    plist_string(&txt, "DeviceIdentifier")
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

fn format_tm_date(date: &str) -> String {
    // 2026-09-13-181501 → 2026-09-13 18:15:01
    if date.len() >= 17 {
        let (d, t) = date.split_at(10);
        let t = t.trim_start_matches('-');
        if t.len() >= 6 {
            return format!("{} {}:{}:{}", d, &t[0..2], &t[2..4], &t[4..6]);
        }
    }
    date.to_string()
}

fn short_name(name: &str) -> String {
    name.rsplit('.').next().unwrap_or(name).to_string()
}

/// Delete oldest TM local snapshots, keeping the `keep_newest` most recent.
/// `keep_newest == 0` deletes every local Time Machine snapshot.
pub fn delete_old_tm_locals(keep_newest: usize) -> CleanReport {
    let snaps = list_tm_locals();
    let before = snaps.len() as u32;
    if snaps.len() <= keep_newest {
        return CleanReport {
            before,
            deleted: 0,
            kept: before,
            errors: Vec::new(),
        };
    }
    #[cfg(target_os = "macos")]
    let cut = snaps.len() - keep_newest;
    #[cfg(target_os = "macos")]
    let mut deleted = 0u32;
    #[cfg(not(target_os = "macos"))]
    let deleted = 0u32;
    let mut errors = Vec::new();

    #[cfg(target_os = "macos")]
    {
        for snap in snaps.iter().take(cut) {
            let Some(date) = &snap.date else {
                continue;
            };
            let out = Command::new("tmutil")
                .args(["deletelocalsnapshots", date])
                .output();
            match out {
                Ok(o) if o.status.success() => deleted += 1,
                Ok(o) => {
                    let err = String::from_utf8_lossy(&o.stderr);
                    let msg = if err.trim().is_empty() {
                        String::from_utf8_lossy(&o.stdout).trim().to_string()
                    } else {
                        err.trim().to_string()
                    };
                    errors.push(format!("{date}: {msg}"));
                }
                Err(e) => errors.push(format!("{date}: {e}")),
            }
        }
        let _ = Command::new("tmutil")
            .args([
                "thinlocalsnapshots",
                "/System/Volumes/Data",
                "1000000000000",
                "4",
            ])
            .output();
    }

    #[cfg(not(target_os = "macos"))]
    {
        errors.push("Time Machine cleanup is only available on macOS".into());
    }

    let kept = before.saturating_sub(deleted);
    CleanReport {
        before,
        deleted,
        kept,
        errors,
    }
}
