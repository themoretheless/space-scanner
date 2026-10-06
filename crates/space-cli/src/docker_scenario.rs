//! Docker-owned data is inventoried through Docker, never filesystem cleanup.
use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Deserialize, Serialize)]
struct Volume {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Size")]
    size: String,
    #[serde(rename = "Links")]
    links: String,
    #[serde(rename = "Labels")]
    labels: String,
    #[serde(rename = "Driver")]
    driver: String,
}

impl Volume {
    fn unreferenced(&self) -> bool {
        self.links.parse::<u64>() == Ok(0)
    }

    fn anonymous(&self) -> bool {
        self.labels
            .split(',')
            .any(|label| label.split('=').next() == Some("com.docker.volume.anonymous"))
    }
}

#[derive(Deserialize)]
struct Inventory {
    #[serde(rename = "Volumes")]
    volumes: Vec<Volume>,
}

pub fn run(json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("docker")
        .args(["system", "df", "-v", "--format", "{{json .}}"])
        .output()?;
    if !output.status.success() {
        return Err(format!("docker: {}", String::from_utf8_lossy(&output.stderr).trim()).into());
    }
    let inventory: Inventory = serde_json::from_slice(&output.stdout)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&inventory.volumes)?);
        return Ok(());
    }
    let unused = inventory
        .volumes
        .iter()
        .filter(|v| v.unreferenced())
        .count();
    let anonymous = inventory
        .volumes
        .iter()
        .filter(|v| v.unreferenced() && v.anonymous())
        .count();
    println!("Docker volumes: {} total; {unused} without container references; {anonymous} of those explicitly anonymous.", inventory.volumes.len());
    println!("References include stopped containers. Unknown reference counts are preserved.");
    println!("\nReview inventory (Docker-reported sizes; includes referenced volumes):");
    for volume in &inventory.volumes {
        let state = if volume.unreferenced() {
            "review"
        } else {
            "preserve"
        };
        let kind = if volume.anonymous() {
            "anonymous"
        } else {
            "named/unknown"
        };
        println!(
            "[{state}] {:>10} links={} {kind} driver={} {}",
            volume.size, volume.links, volume.driver, volume.name
        );
        if !volume.labels.is_empty() {
            println!("  labels: {}", volume.labels);
        }
    }
    println!("\nBefore deleting any selected volume:");
    println!("1. Inspect its labels and creation date with docker volume inspect VOLUME.");
    println!("2. Verify project ownership and inspect or back up its contents through a read-only mount.");
    println!("3. Recheck all container references immediately before any explicit removal.");
    println!(
        "No references does not mean disposable: volumes may contain databases or unique files."
    );
    println!("This scenario never removes volumes or Docker.raw. Docker reclaimable space is not a guarantee of immediately recovered Mac disk space.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_counts_and_labels_do_not_infer_disposability_from_names() {
        let inventory: Inventory = serde_json::from_str(r#"{"Volumes":[
            {"Name":"database","Size":"43GB","Links":"0","Labels":"com.docker.compose.project=app","Driver":"local"},
            {"Name":"hash","Size":"210MB","Links":"1","Labels":"com.docker.volume.anonymous=","Driver":"local"},
            {"Name":"unknown","Size":"N/A","Links":"N/A","Labels":"","Driver":"remote"},
            {"Name":"unused","Size":"0B","Links":"0","Labels":"com.docker.volume.anonymous=","Driver":"local"}
        ]}"#).unwrap();
        assert!(inventory.volumes[0].unreferenced());
        assert!(!inventory.volumes[0].anonymous());
        assert!(!inventory.volumes[1].unreferenced());
        assert!(inventory.volumes[1].anonymous());
        assert!(!inventory.volumes[2].unreferenced());
        assert!(inventory.volumes[3].unreferenced());
        assert!(inventory.volumes[3].anonymous());
    }
}
