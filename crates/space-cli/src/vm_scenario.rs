//! Guided read-only inventory for VM, container, and simulator disk stores.

use crate::scenario_common::{measure_path, user_home};
use serde::Serialize;
use space_core::format_bytes;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct VmEntry {
    pub kind: &'static str,
    pub path: PathBuf,
    pub disk_bytes: u64,
    pub apparent_bytes: u64,
    pub sparse_savings_bytes: u64,
    pub files: u64,
    pub inspect_command: &'static str,
    pub reclaim_hint: &'static str,
}

#[derive(Debug, Serialize)]
pub struct VmDisksReport {
    pub total_disk_bytes: u64,
    pub total_apparent_bytes: u64,
    pub entries: Vec<VmEntry>,
}

struct VmSpec {
    kind: &'static str,
    rel: &'static str,
    inspect_command: &'static str,
    reclaim_hint: &'static str,
}

const VM_SPECS: &[VmSpec] = &[
    VmSpec {
        kind: "Colima VMs",
        rel: ".colima/_lima",
        inspect_command: "colima list",
        reclaim_hint: "colima delete <profile> or colima prune",
    },
    VmSpec {
        kind: "Lima VMs",
        rel: ".lima",
        inspect_command: "limactl list",
        reclaim_hint: "limactl delete <instance>",
    },
    VmSpec {
        kind: "Docker Desktop VMs",
        rel: "Library/Containers/com.docker.docker/Data/vms",
        inspect_command: "docker system df",
        reclaim_hint: "space scenario docker-volumes / Docker Desktop settings",
    },
    VmSpec {
        kind: "OrbStack VM data",
        rel: "Library/Group Containers/HUAQ24HBR6.dev.orbstack/data",
        inspect_command: "orbctl list",
        reclaim_hint: "orbctl delete <vm> or docker system prune",
    },
    VmSpec {
        kind: "Podman machine",
        rel: ".local/share/containers/podman/machine",
        inspect_command: "podman machine list",
        reclaim_hint: "podman machine rm <name>",
    },
    VmSpec {
        kind: "CoreSimulator Devices",
        rel: "Library/Developer/CoreSimulator/Devices",
        inspect_command: "xcrun simctl list devices",
        reclaim_hint: "xcrun simctl delete unavailable",
    },
    VmSpec {
        kind: "Android AVDs",
        rel: ".android/avd",
        inspect_command: "avdmanager list avd",
        reclaim_hint: "Delete unused AVDs via Android Studio or avdmanager",
    },
    VmSpec {
        kind: "Claude Desktop VMs",
        rel: "Library/Application Support/Claude/vm_bundles",
        inspect_command: "ls -lh ~/Library/Application\\ Support/Claude/vm_bundles",
        reclaim_hint: "Re-downloaded automatically by Claude Desktop if trashed",
    },
    VmSpec {
        kind: "Claude Code VM",
        rel: "Library/Application Support/Claude/claude-code-vm",
        inspect_command: "ls -lh ~/Library/Application\\ Support/Claude/claude-code-vm",
        reclaim_hint: "Re-downloaded automatically if trashed",
    },
    VmSpec {
        kind: "UTM VMs",
        rel: "Library/Containers/com.utmapp.UTM/Data/Documents",
        inspect_command: "ls -lh ~/Library/Containers/com.utmapp.UTM/Data/Documents",
        reclaim_hint: "Remove unused .utm bundles inside UTM",
    },
    VmSpec {
        kind: "Parallels VMs",
        rel: "Parallels",
        inspect_command: "prlctl list -a",
        reclaim_hint: "Remove unused .pvm bundles in Parallels Control Center",
    },
    VmSpec {
        kind: "Parallels VMs (localized)",
        rel: "Virtual Machines.localized",
        inspect_command: "ls -lh ~/Virtual\\ Machines.localized",
        reclaim_hint: "Remove unused VM bundles in hypervisor UI",
    },
    VmSpec {
        kind: "Tart VMs",
        rel: ".tart/vms",
        inspect_command: "tart list",
        reclaim_hint: "tart delete <name>",
    },
    VmSpec {
        kind: "Vagrant boxes",
        rel: ".vagrant.d/boxes",
        inspect_command: "vagrant box list",
        reclaim_hint: "vagrant box remove <name>",
    },
];

pub fn run(json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let home = user_home()?;
    let report = collect_report(&home);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!(
        "VM, container & simulator disks: {} on-disk ({} apparent)",
        format_bytes(report.total_disk_bytes),
        format_bytes(report.total_apparent_bytes),
    );
    if report.entries.is_empty() {
        println!("No VM or simulator disk stores found.");
        return Ok(());
    }
    println!("\nInventory (sorted by on-disk allocated bytes):");
    for entry in &report.entries {
        let sparse_note = if entry.sparse_savings_bytes > 1024 * 1024 {
            format!(
                " (apparent {}, sparse saves {})",
                format_bytes(entry.apparent_bytes),
                format_bytes(entry.sparse_savings_bytes)
            )
        } else {
            String::new()
        };
        println!(
            "  {:>10}{}  [{}]  {}",
            format_bytes(entry.disk_bytes),
            sparse_note,
            entry.kind,
            entry.path.display(),
        );
        println!(
            "     inspect: `{}`  |  reclaim: {}",
            entry.inspect_command, entry.reclaim_hint
        );
    }
    println!("\nRead-only diagnostic scenario: nothing was modified or deleted.");
    Ok(())
}

pub fn collect_report(home: &Path) -> VmDisksReport {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();

    for spec in VM_SPECS {
        let path = home.join(spec.rel);
        let canon = path.canonicalize().unwrap_or_else(|_| path.clone());
        if !seen.insert(canon) {
            continue;
        }
        let Some(stats) = measure_path(&path) else {
            continue;
        };
        if stats.disk_bytes == 0 && stats.apparent_bytes == 0 {
            continue;
        }
        entries.push(VmEntry {
            kind: spec.kind,
            path,
            disk_bytes: stats.disk_bytes,
            apparent_bytes: stats.apparent_bytes,
            sparse_savings_bytes: stats.apparent_bytes.saturating_sub(stats.disk_bytes),
            files: stats.files,
            inspect_command: spec.inspect_command,
            reclaim_hint: spec.reclaim_hint,
        });
    }

    entries.sort_by(|a, b| {
        b.disk_bytes
            .cmp(&a.disk_bytes)
            .then_with(|| b.apparent_bytes.cmp(&a.apparent_bytes))
            .then_with(|| a.path.cmp(&b.path))
    });

    let total_disk_bytes = entries
        .iter()
        .fold(0u64, |sum, e| sum.saturating_add(e.disk_bytes));
    let total_apparent_bytes = entries
        .iter()
        .fold(0u64, |sum, e| sum.saturating_add(e.apparent_bytes));

    VmDisksReport {
        total_disk_bytes,
        total_apparent_bytes,
        entries,
    }
}
