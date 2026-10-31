//! Personal data worth reviewing: forgotten installers, the Trash, iPhone
//! backups and message attachments. Several of these folders are protected
//! by macOS until the app has Full Disk Access; denied reads are reported
//! through [`ScanState::denied`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::{Category, Finding, Safety, ScanOptions, deletable, file_name, subdirectories};
use crate::execute::Action;
use crate::walk::ScanState;

const INSTALLER_EXTENSIONS: &[&str] = &["dmg", "pkg", "mpkg", "xip", "iso"];
const INSTALLER_MIN_AGE: Duration = Duration::from_secs(7 * 86_400);

pub(super) fn detect(options: &ScanOptions, state: &ScanState) -> Vec<Finding> {
    let home = &options.home;
    let mut findings = Vec::new();
    findings.extend(installers(state, &home.join("Downloads"), SystemTime::now()));
    findings.extend(trash(state, &home.join(".Trash")));
    findings.extend(ios_backups(
        state,
        &home.join("Library/Application Support/MobileSync/Backup"),
    ));

    let mail_downloads = home.join("Library/Containers/com.apple.mail/Data/Library/Mail Downloads");
    if mail_downloads.is_dir() {
        findings.push(deletable(
            state,
            Category::PersonalFiles,
            mail_downloads,
            "Pièces jointes ouvertes depuis Mail".to_owned(),
            "Copies temporaires : les originaux restent dans les e-mails.".to_owned(),
            Safety::Safe,
        ));
    }

    let attachments = home.join("Library/Messages/Attachments");
    let bytes = state.allocated_size(&attachments);
    if bytes > 0 {
        findings.push(Finding {
            id: "messages:attachments".to_owned(),
            category: Category::PersonalFiles,
            title: "Pièces jointes de Messages".to_owned(),
            detail: "À trier dans Réglages Système › Général › Stockage › Messages : supprimer les fichiers ici casserait les conversations."
                .to_owned(),
            bytes,
            safety: Safety::Careful,
            action: Action::Manual,
            last_used: None,
        });
    }
    findings
}

/// Installers downloaded more than a week ago, gathered in one finding.
fn installers(state: &ScanState, downloads: &Path, now: SystemTime) -> Option<Finding> {
    let entries = fs::read_dir(downloads).ok()?;
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| {
            let path = entry.path();
            let is_installer = path
                .extension()
                .is_some_and(|ext| INSTALLER_EXTENSIONS.iter().any(|known| ext.eq_ignore_ascii_case(known)));
            let old_enough = entry
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|modified| now.duration_since(modified).unwrap_or_default() >= INSTALLER_MIN_AGE);
            is_installer && old_enough
        })
        .map(|entry| entry.path())
        .collect();
    if paths.is_empty() {
        return None;
    }
    paths.sort();
    Some(Finding {
        id: format!("installers:{}", downloads.display()),
        category: Category::PersonalFiles,
        title: format!("Installeurs dans Téléchargements ({})", paths.len()),
        detail: "Fichiers .dmg, .pkg, .xip ou .iso de plus d'une semaine, généralement déjà installés. Ils vont à la corbeille."
            .to_owned(),
        bytes: paths.iter().map(|p| state.allocated_size(p)).sum(),
        safety: Safety::Review,
        action: Action::Trash { paths },
        last_used: None,
    })
}

fn trash(state: &ScanState, trash: &Path) -> Option<Finding> {
    let entries = match fs::read_dir(trash) {
        Ok(entries) => entries,
        Err(err) => {
            if err.kind() == io::ErrorKind::PermissionDenied {
                state.record_denied(trash);
            }
            return None;
        }
    };
    let paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    if paths.is_empty() {
        return None;
    }
    let count = match paths.len() {
        1 => "1 élément".to_owned(),
        n => format!("{n} éléments"),
    };
    Some(Finding {
        id: "trash".to_owned(),
        category: Category::PersonalFiles,
        title: format!("Corbeille ({count})"),
        detail: "Vider la corbeille supprime ces éléments définitivement.".to_owned(),
        bytes: state.allocated_size(trash),
        safety: Safety::Careful,
        action: Action::Delete { paths },
        last_used: None,
    })
}

fn ios_backups(state: &ScanState, backups: &Path) -> Vec<Finding> {
    if let Err(err) = fs::read_dir(backups)
        && err.kind() == io::ErrorKind::PermissionDenied
    {
        state.record_denied(backups);
        return Vec::new();
    }
    subdirectories(backups)
        .into_iter()
        .map(|backup| {
            let info = plist::Value::from_file(backup.join("Info.plist")).ok();
            let dictionary = info.as_ref().and_then(plist::Value::as_dictionary);
            let device = dictionary
                .and_then(|d| d.get("Device Name"))
                .and_then(plist::Value::as_string)
                .map_or_else(|| file_name(&backup), str::to_owned);
            let last_backup = dictionary
                .and_then(|d| d.get("Last Backup Date"))
                .and_then(plist::Value::as_date)
                .map(|date| crate::time::unix_seconds(date.into()));
            Finding {
                id: format!("path:{}", backup.display()),
                category: Category::PersonalFiles,
                title: format!("Sauvegarde de {device}"),
                detail: "Sauvegarde locale de l'appareil. Inutile si l'appareil est sauvegardé sur iCloud, irremplaçable sinon."
                    .to_owned(),
                bytes: state.allocated_size(&backup),
                safety: Safety::Careful,
                action: Action::Delete { paths: vec![backup] },
                last_used: last_backup,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_week_old_installers_are_proposed() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["App.dmg", "Tool.PKG", "photo.jpg", "fresh.dmg"] {
            fs::write(dir.path().join(name), b"x").unwrap();
        }
        // Seen from ten days ahead every file is old enough; seen from now, none is.
        let in_ten_days = SystemTime::now() + Duration::from_secs(10 * 86_400);
        let finding = installers(&ScanState::default(), dir.path(), in_ten_days).unwrap();
        let names: Vec<String> = finding.action.paths().iter().map(|p| file_name(p)).collect();
        assert_eq!(names, vec!["App.dmg", "Tool.PKG", "fresh.dmg"]);

        assert!(installers(&ScanState::default(), dir.path(), SystemTime::now()).is_none());
    }

    #[test]
    fn emptying_the_trash_is_careful_and_permanent() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("old.txt"), b"x").unwrap();
        let finding = trash(&ScanState::default(), dir.path()).unwrap();
        assert_eq!(finding.safety, Safety::Careful);
        assert!(matches!(finding.action, Action::Delete { .. }));
        assert_eq!(finding.title, "Corbeille (1 élément)");
    }

    #[test]
    fn backups_are_named_after_their_device() {
        let dir = tempfile::tempdir().unwrap();
        let backup = dir.path().join("00008120-001A");
        fs::create_dir(&backup).unwrap();
        let mut info = plist::Dictionary::new();
        info.insert("Device Name".into(), plist::Value::String("iPhone de Naos".into()));
        plist::Value::Dictionary(info)
            .to_file_xml(backup.join("Info.plist"))
            .unwrap();

        let findings = ios_backups(&ScanState::default(), dir.path());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].title, "Sauvegarde de iPhone de Naos");
    }
}
