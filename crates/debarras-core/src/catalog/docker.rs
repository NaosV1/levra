//! Docker images, build cache and orphaned volumes.

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use super::{Category, Finding, Safety, ScanOptions};
use crate::execute::Action;
use crate::tools;
use crate::walk::ScanState;

const DISK_IMAGE: &str = "Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw";

pub(super) fn detect(options: &ScanOptions, state: &ScanState) -> Vec<Finding> {
    let Some(docker) = tools::find("docker", &options.home) else {
        return Vec::new();
    };
    let usage = tools::output(
        &docker,
        &["system", "df", "--format", "{{json .}}"],
        &options.home,
        Duration::from_secs(15),
    );
    let Some(lines) = usage else {
        return stopped_finding(options, state).into_iter().collect();
    };
    let mut findings = usage_findings(&docker, &lines);
    let volumes = tools::output(
        &docker,
        &["system", "df", "-v", "--format", "{{json .}}"],
        &options.home,
        Duration::from_secs(30),
    );
    findings.extend(volumes.and_then(|json| volume_finding(&docker, &json)));
    findings
}

/// Without the daemon nothing can be measured inside the VM, but its disk
/// image shows how much is at stake.
fn stopped_finding(options: &ScanOptions, state: &ScanState) -> Option<Finding> {
    let image = options.home.join(DISK_IMAGE);
    let bytes = state.allocated_size(&image);
    (bytes > 0).then(|| Finding {
        id: "docker:stopped".to_owned(),
        category: Category::Docker,
        title: "Disque virtuel Docker".to_owned(),
        detail: "Docker est arrêté : démarre Docker Desktop puis relance l'analyse pour voir images, cache et volumes récupérables."
            .to_owned(),
        bytes,
        safety: Safety::Review,
        action: Action::Manual,
        last_used: None,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UsageLine {
    #[serde(rename = "Type")]
    kind: String,
    reclaimable: String,
}

fn usage_findings(docker: &Path, lines: &str) -> Vec<Finding> {
    lines
        .lines()
        .filter_map(|line| serde_json::from_str::<UsageLine>(line).ok())
        .filter_map(|usage| {
            let bytes = parse_size(&usage.reclaimable)?;
            let (key, title, detail, safety, args): (_, _, _, _, &[&str]) = match usage.kind.as_str() {
                "Images" => (
                    "images",
                    "Images Docker inutilisées",
                    "Images qu'aucun conteneur n'utilise : retéléchargées ou reconstruites au besoin.",
                    Safety::Review,
                    &["image", "prune", "-a", "-f"],
                ),
                "Build Cache" => (
                    "build-cache",
                    "Cache de build Docker",
                    "Couches intermédiaires des docker build : le prochain build sera plus long.",
                    Safety::Safe,
                    &["builder", "prune", "-a", "-f"],
                ),
                _ => return None,
            };
            Some(prune_finding(docker, key, title, detail, bytes, safety, args))
        })
        .collect()
}

fn prune_finding(
    docker: &Path,
    key: &str,
    title: &str,
    detail: &str,
    bytes: u64,
    safety: Safety,
    args: &[&str],
) -> Finding {
    Finding {
        id: format!("docker:{key}"),
        category: Category::Docker,
        title: title.to_owned(),
        detail: format!("{detail} Docker Desktop rend ensuite l'espace au Mac en quelques secondes."),
        bytes,
        safety,
        action: Action::Command {
            program: docker.to_path_buf(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
        },
        last_used: None,
    }
}

#[derive(Deserialize)]
struct VerboseUsage {
    #[serde(rename = "Volumes", default)]
    volumes: Vec<Volume>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Volume {
    name: String,
    links: String,
    size: String,
}

/// Docker's own "reclaimable" figure for volumes includes named volumes
/// that `docker volume prune` keeps; only anonymous orphans are counted.
fn volume_finding(docker: &Path, json: &str) -> Option<Finding> {
    let usage: VerboseUsage = serde_json::from_str(json).ok()?;
    let bytes: u64 = usage
        .volumes
        .iter()
        .filter(|v| v.links == "0" && v.name.len() == 64 && v.name.chars().all(|c| c.is_ascii_hexdigit()))
        .filter_map(|v| parse_size(&v.size))
        .sum();
    (bytes > 0).then(|| {
        prune_finding(
            docker,
            "volumes",
            "Volumes Docker anonymes orphelins",
            "Volumes anonymes rattachés à aucun conteneur, souvent des restes de tests. Ils peuvent contenir des données ; les volumes nommés ne sont pas touchés.",
            bytes,
            Safety::Careful,
            &["volume", "prune", "-f"],
        )
    })
}

/// Docker prints decimal units: `13.76GB (68%)`, `155.6kB`, `0B`.
fn parse_size(text: &str) -> Option<u64> {
    let amount = text.split_whitespace().next()?;
    let split = amount.find(|c: char| c.is_ascii_alphabetic())?;
    let (number, unit) = amount.split_at(split);
    let multiplier = match unit {
        "B" => 1.0,
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        _ => return None,
    };
    let value: f64 = number.parse().ok()?;
    Some((value * multiplier).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DF: &str = r#"{"Active":"13","Reclaimable":"13.76GB (68%)","Size":"19.95GB","TotalCount":"47","Type":"Images"}
{"Active":"6","Reclaimable":"155.6kB (71%)","Size":"217.1kB","TotalCount":"14","Type":"Containers"}
{"Active":"7","Reclaimable":"13.64GB (97%)","Size":"14GB","TotalCount":"266","Type":"Local Volumes"}
{"Active":"0","Reclaimable":"4.963GB","Size":"15.94GB","TotalCount":"816","Type":"Build Cache"}"#;

    #[test]
    fn parses_docker_sizes() {
        assert_eq!(parse_size("13.76GB (68%)"), Some(13_760_000_000));
        assert_eq!(parse_size("155.6kB"), Some(155_600));
        assert_eq!(parse_size("0B"), Some(0));
        assert_eq!(parse_size("n/a"), None);
    }

    #[test]
    fn maps_images_and_build_cache_to_their_prune_command() {
        let findings = usage_findings(Path::new("/usr/local/bin/docker"), DF);
        let commands: Vec<(String, String)> = findings
            .iter()
            .map(|f| (f.id.clone(), f.action.command_line().unwrap()))
            .collect();
        assert_eq!(
            commands,
            vec![
                ("docker:images".to_owned(), "docker image prune -a -f".to_owned()),
                ("docker:build-cache".to_owned(), "docker builder prune -a -f".to_owned()),
            ]
        );
    }

    #[test]
    fn only_anonymous_orphan_volumes_count() {
        let anonymous = "ba258e9b31977486383fceeebff8ceb3d5ba49149f335fe0fa417669aa4864b7";
        let json = format!(
            r#"{{"Images": [], "Volumes": [
                {{"Name": "{anonymous}", "Links": "0", "Size": "50.47MB"}},
                {{"Name": "rg-cargo-registry", "Links": "0", "Size": "135.4MB"}},
                {{"Name": "clipcraft_pgdata", "Links": "1", "Size": "66.36MB"}}
            ]}}"#
        );
        let finding = volume_finding(Path::new("docker"), &json).unwrap();
        assert_eq!(finding.bytes, 50_470_000);
        assert_eq!(finding.safety, Safety::Careful);
        assert_eq!(finding.action.command_line().unwrap(), "docker volume prune -f");
    }

    #[test]
    fn no_volume_finding_without_anonymous_orphans() {
        let json = r#"{"Volumes": [{"Name": "rg-cargo-registry", "Links": "0", "Size": "135.4MB"}]}"#;
        assert!(volume_finding(Path::new("docker"), json).is_none());
    }
}
