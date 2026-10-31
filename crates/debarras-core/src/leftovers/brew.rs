//! Homebrew casks. `brew uninstall --zap` also removes the files the
//! cask's maintainers listed, which a generic search can miss.

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use crate::apps::AppInfo;
use crate::tools;

#[derive(Deserialize)]
struct CaskList {
    casks: Vec<Cask>,
}

#[derive(Deserialize)]
struct Cask {
    token: String,
    #[serde(default)]
    artifacts: Vec<serde_json::Value>,
}

/// Cask that installed the app, when Homebrew is installed.
pub fn brew_cask_for(app: &AppInfo, home: &Path) -> Option<String> {
    let brew = tools::find("brew", home)?;
    let json = tools::output(
        &brew,
        &["info", "--cask", "--json=v2", "--installed"],
        home,
        Duration::from_secs(30),
    )?;
    let bundle = app.path.file_name()?.to_string_lossy().into_owned();
    cask_installing(&json, &bundle)
}

/// Token of the cask whose `app` artifact is `bundle` (`Docker.app`).
fn cask_installing(json: &str, bundle: &str) -> Option<String> {
    let list: CaskList = serde_json::from_str(json).ok()?;
    list.casks.into_iter().find_map(|cask| {
        let installs_it = cask.artifacts.iter().any(|artifact| {
            artifact
                .get("app")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|apps| apps.iter().any(|a| a.as_str() == Some(bundle)))
        });
        installs_it.then_some(cask.token)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_cask_that_installed_an_app() {
        let json = r#"{"formulae": [], "casks": [
            {"token": "android-commandlinetools", "artifacts": [{"binary": ["/x/sdkmanager"]}]},
            {"token": "docker-desktop", "artifacts": [{"app": ["Docker.app"]}, {"zap": [{"trash": ["~/.docker"]}]}]}
        ]}"#;
        assert_eq!(cask_installing(json, "Docker.app").as_deref(), Some("docker-desktop"));
        assert_eq!(cask_installing(json, "Notion.app"), None);
    }
}
