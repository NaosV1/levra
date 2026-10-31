//! French labels and size formatting for terminal output.

use debarras_core::debloat::{Owner, Schedule};
use debarras_core::launchd::Scope;
use debarras_core::memory::Pressure;
use debarras_core::{Category, LeftoverKind, Safety};

/// Decimal units, as the Finder shows them.
pub fn bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["o", "Ko", "Mo", "Go", "To"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} o")
    } else {
        format!("{value:.1} {}", UNITS[unit]).replace('.', ",")
    }
}

pub fn pressure(pressure: Pressure) -> &'static str {
    match pressure {
        Pressure::Normal => "normale",
        Pressure::Warning => "élevée",
        Pressure::Critical => "critique",
    }
}

pub fn safety(safety: Safety) -> &'static str {
    match safety {
        Safety::Safe => "sûr",
        Safety::Review => "à vérifier",
        Safety::Careful => "prudence",
    }
}

pub fn category(category: Category) -> &'static str {
    match category {
        Category::ProjectBuilds => "Builds de projets",
        Category::GitWorktrees => "Worktrees git",
        Category::PackageCaches => "Caches de paquets",
        Category::AppCaches => "Caches d'applications",
        Category::Xcode => "Xcode",
        Category::Android => "Android",
        Category::Docker => "Docker",
        Category::PersonalFiles => "Fichiers personnels",
    }
}

pub fn leftover_kind(kind: LeftoverKind) -> &'static str {
    match kind {
        LeftoverKind::Application => "Application",
        LeftoverKind::Support => "Données (Application Support)",
        LeftoverKind::Cache => "Caches",
        LeftoverKind::Preferences => "Préférences",
        LeftoverKind::Container => "Conteneurs",
        LeftoverKind::GroupContainer => "Conteneurs partagés",
        LeftoverKind::SavedState => "État de fenêtres",
        LeftoverKind::WebData => "Données web",
        LeftoverKind::Logs => "Journaux",
        LeftoverKind::Scripts => "Scripts",
        LeftoverKind::LaunchAgent => "Agents de démarrage",
        LeftoverKind::LaunchDaemon => "Services système",
        LeftoverKind::HelperTool => "Outils privilégiés",
        LeftoverKind::Temporary => "Fichiers temporaires",
        LeftoverKind::Receipt => "Reçus d'installation",
    }
}

pub fn scope(scope: Scope) -> &'static str {
    match scope {
        Scope::UserAgent => "Agent",
        Scope::Agent => "Agent (tous)",
        Scope::Daemon => "Service système",
    }
}

pub fn schedule(schedule: Schedule) -> String {
    match schedule {
        Schedule::AtLoad => "à l'ouverture".to_owned(),
        Schedule::KeepAlive => "en permanence".to_owned(),
        Schedule::Interval(3600) => "toutes les heures".to_owned(),
        Schedule::Interval(seconds) if seconds >= 3600 && seconds % 3600 == 0 => {
            format!("toutes les {} h", seconds / 3600)
        }
        Schedule::Interval(seconds) if seconds >= 60 => format!("toutes les {} min", seconds / 60),
        Schedule::Interval(seconds) => format!("toutes les {seconds} s"),
        Schedule::Calendar => "à heures fixes".to_owned(),
        Schedule::OnDemand => "à la demande".to_owned(),
        Schedule::Unknown => "illisible".to_owned(),
    }
}

pub fn owner(owner: &Owner) -> String {
    match owner {
        Owner::Apps(apps) => apps.iter().map(|app| app.name.as_str()).collect::<Vec<_>>().join(", "),
        Owner::Homebrew => "Homebrew (brew services)".to_owned(),
        Owner::NoApp => "aucune app installée de cet éditeur".to_owned(),
        Owner::Broken => "ne lance plus rien (fichier vide ou programme disparu)".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_the_finder() {
        assert_eq!(bytes(999), "999 o");
        assert_eq!(bytes(43_700_000_000), "43,7 Go");
        assert_eq!(bytes(1_500_000), "1,5 Mo");
    }

    #[test]
    fn says_how_often_a_job_runs() {
        assert_eq!(schedule(Schedule::Interval(3600)), "toutes les heures");
        assert_eq!(schedule(Schedule::Interval(7200)), "toutes les 2 h");
        assert_eq!(schedule(Schedule::Interval(900)), "toutes les 15 min");
        assert_eq!(schedule(Schedule::Interval(30)), "toutes les 30 s");
    }
}
