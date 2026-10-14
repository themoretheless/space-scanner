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
    let output = match Command::new("docker")
        .args(["system", "df", "-v", "--format", "{{json .}}"])
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("Docker CLI not found in PATH. Install Docker Desktop / docker-cli, \
                 or run this scenario on the machine where Docker is installed."
                .into())
        }
        Err(error) => return Err(format!("cannot run docker: {error}").into()),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if unsupported_version(stderr) {
            return Err(format!(
                "this Docker version does not support `docker system df -v --format`; \
                 upgrade Docker (CLI 23+ is required). Docker said: {stderr}"
            )
            .into());
        }
        if stderr.contains("Cannot connect") || stderr.contains("daemon") {
            return Err(format!("Docker daemon is not running or not reachable: {stderr}").into());
        }
        return Err(format!("docker: {stderr}").into());
    }
    let inventory: Inventory = serde_json::from_slice(&output.stdout).map_err(|error| {
        format!("unexpected `docker system df` output (unsupported Docker version?): {error}")
    })?;
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

/// `--format` on `system df -v` arrived with Docker CLI 23; older CLIs reject
/// the flag or the verbose/format combination.
fn unsupported_version(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("unknown flag")
        || lower.contains("unknown shorthand flag")
        || lower.contains("conflicting options")
        || (lower.contains("format") && lower.contains("not supported"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_docker_errors_are_recognised() {
        assert!(unsupported_version("unknown flag: --format"));
        assert!(unsupported_version("Error: conflicting options: --verbose and --format"));
        assert!(!unsupported_version("Cannot connect to the Docker daemon"));
    }

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
