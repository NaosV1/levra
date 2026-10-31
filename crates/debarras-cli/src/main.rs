//! `debarras`: the Débarras core from a terminal. Every command that
//! removes something only prints what it would do unless `--apply` is set.

mod labels;

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use debarras_core::{
    AppInfo, DuplicateOptions, ExecutionReport, Roots, ScanOptions, ScanState, Tree, apps, catalog, debloat,
    duplicates, explore, leftovers, memory,
};

const USAGE: &str = "\
Usage :
  debarras scan [--json]                    Ce qui peut être récupéré
  debarras clean [ID…] [--recommended] [--apply]
                                            Nettoie les éléments choisis (ou les recommandés)
  debarras explore [DOSSIER] [--top N]      Ce qui occupe un dossier (par défaut : ~)
  debarras duplicates [DOSSIER] [--top N] [--json]
                                            Fichiers en double (lecture seule ; supprimer depuis l'app)
  debarras memory [--top N] [--json]        Ce qui occupe la mémoire vive
  debarras apps [--json]                    Applications installées
  debarras uninstall APP [--brew] [--apply] Désinstalle complètement une app (nom, chemin ou bundle id)
  debarras debloat [--json]                 Agents et services installés par les apps, apps d'Apple
                                            (lecture seule ; supprimer depuis l'app)

Sans --apply, rien n'est supprimé : la commande affiche ce qu'elle ferait.";

struct Args {
    positional: Vec<String>,
    flags: Vec<String>,
    top: usize,
}

impl Args {
    fn parse(raw: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut args = Self {
            positional: Vec::new(),
            flags: Vec::new(),
            top: 15,
        };
        let mut raw = raw.peekable();
        while let Some(arg) = raw.next() {
            match arg.as_str() {
                "--top" => {
                    let value = raw.next().ok_or("--top attend un nombre")?;
                    args.top = value
                        .parse()
                        .map_err(|_| format!("--top : « {value} » n'est pas un nombre"))?;
                }
                "--json" | "--apply" | "--recommended" | "--brew" => args.flags.push(arg),
                flag if flag.starts_with("--") => return Err(format!("option inconnue : {flag}")),
                _ => args.positional.push(arg),
            }
        }
        Ok(args)
    }

    fn has(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f == flag)
    }
}

fn main() -> ExitCode {
    let mut raw = env::args().skip(1);
    let command = raw.next();
    let args = match Args::parse(raw) {
        Ok(args) => args,
        Err(err) => {
            eprintln!("{err}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let Some(home) = env::var_os("HOME").map(PathBuf::from) else {
        eprintln!("HOME n'est pas défini");
        return ExitCode::FAILURE;
    };

    let result = match command.as_deref() {
        Some("scan") => scan(&home, &args),
        Some("clean") => clean(&home, &args),
        Some("explore") => explore_folder(&home, &args),
        Some("duplicates") => find_duplicates(&home, &args),
        Some("memory") => show_memory(&args),
        Some("apps") => list_apps(&home, &args),
        Some("uninstall") => uninstall(&home, &args),
        Some("debloat") => debloat_report(&home, &args),
        _ => {
            println!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(code) => code,
        Err(err) => {
            eprintln!("Erreur : {err}");
            ExitCode::FAILURE
        }
    }
}

fn scan(home: &Path, args: &Args) -> Result<ExitCode, String> {
    let started = Instant::now();
    let report = catalog::scan(&ScanOptions::for_home(home), &ScanState::default());
    if args.has("--json") {
        println!("{}", serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?);
        return Ok(ExitCode::SUCCESS);
    }

    let now = debarras_core::time::unix_seconds(std::time::SystemTime::now());
    let mut recommended = 0;
    for finding in &report.findings {
        let mark = if finding.recommended(now) {
            recommended += finding.bytes;
            "*"
        } else {
            " "
        };
        println!(
            "{mark}{:>9}  {:<10}  {:<22}  {}",
            labels::bytes(finding.bytes),
            labels::safety(finding.safety),
            labels::category(finding.category),
            finding.title
        );
        println!(" {:>9}  {}", "", finding.id);
    }
    println!(
        "\n{} éléments ; recommandés (*) : {}. Analyse en {:.1} s.",
        report.findings.len(),
        labels::bytes(recommended),
        started.elapsed().as_secs_f64()
    );
    print_denied(&report.denied);
    Ok(ExitCode::SUCCESS)
}

fn print_denied(denied: &[PathBuf]) {
    if denied.is_empty() {
        return;
    }
    println!(
        "\nmacOS a refusé l'accès à {} dossiers (Accès complet au disque requis), par exemple :",
        denied.len()
    );
    for path in denied.iter().take(5) {
        println!("  {}", path.display());
    }
}

fn clean(home: &Path, args: &Args) -> Result<ExitCode, String> {
    if args.positional.is_empty() && !args.has("--recommended") {
        return Err("indique des identifiants (voir debarras scan) ou --recommended".into());
    }
    let report = catalog::scan(&ScanOptions::for_home(home), &ScanState::default());
    let now = debarras_core::time::unix_seconds(std::time::SystemTime::now());
    let ids: Vec<String> = report
        .findings
        .iter()
        .filter(|f| args.positional.contains(&f.id) || args.has("--recommended") && f.recommended(now))
        .map(|f| f.id.clone())
        .collect();
    for id in &args.positional {
        if !ids.contains(id) {
            eprintln!("Introuvable (déjà nettoyé ?) : {id}");
        }
    }

    for finding in report.findings.iter().filter(|f| ids.contains(&f.id)) {
        let how = finding.action.command_line().unwrap_or_else(|| {
            format!(
                "{} de {} élément(s)",
                action_verb(&finding.action),
                finding.action.paths().len()
            )
        });
        println!("{:>9}  {}  →  {how}", labels::bytes(finding.bytes), finding.title);
    }
    if !args.has("--apply") {
        println!("\nAucune suppression : relance avec --apply pour nettoyer.");
        return Ok(ExitCode::SUCCESS);
    }
    Ok(print_execution(&report.execute(&ids, home)))
}

fn action_verb(action: &debarras_core::Action) -> &'static str {
    match action {
        debarras_core::Action::Trash { .. } => "Corbeille",
        debarras_core::Action::Delete { .. } => "Suppression",
        debarras_core::Action::Command { .. } => "Commande",
        debarras_core::Action::Manual => "Manuel",
    }
}

fn print_execution(report: &ExecutionReport) -> ExitCode {
    println!(
        "\n{} élément(s) traités, ~{} libérés.",
        report.completed.len(),
        labels::bytes(report.freed_bytes)
    );
    for pending in &report.needs_authorization {
        println!("  Autorisation macOS requise : {}", pending.path.display());
    }
    for failure in &report.failures {
        println!("  Échec : {} — {}", failure.target, failure.reason);
    }
    if report.failures.is_empty() && report.needs_authorization.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn explore_folder(home: &Path, args: &Args) -> Result<ExitCode, String> {
    let root = args
        .positional
        .first()
        .map_or_else(|| home.to_path_buf(), PathBuf::from);
    let started = Instant::now();
    let state = ScanState::default();
    let tree = explore::explore(&state, &root, explore::DEFAULT_MIN_FILE_BYTES).map_err(|e| e.to_string())?;
    let root_node = tree.node(Tree::ROOT).ok_or("arbre vide")?;
    println!(
        "{} : {} dans {} éléments ({:.1} s)\n",
        root.display(),
        labels::bytes(root_node.bytes),
        root_node.items,
        started.elapsed().as_secs_f64()
    );
    for &id in tree.children(Tree::ROOT).iter().take(args.top) {
        let node = tree.node(id).ok_or("nœud manquant")?;
        println!("{:>9}  {}", labels::bytes(node.bytes), node.name);
    }
    println!("\nPlus gros fichiers :");
    for id in tree.largest_files(args.top, 0, None) {
        let (Some(node), Some(path)) = (tree.node(id), tree.path(id)) else {
            continue;
        };
        println!("{:>9}  {}", labels::bytes(node.bytes), path.display());
    }
    print_denied(&state.denied());
    Ok(ExitCode::SUCCESS)
}

fn find_duplicates(home: &Path, args: &Args) -> Result<ExitCode, String> {
    let mut options = DuplicateOptions::for_home(home);
    if let Some(folder) = args.positional.first() {
        options.roots = vec![PathBuf::from(folder)];
    }
    let started = Instant::now();
    let report = duplicates::find(&options, &ScanState::default());
    if args.has("--json") {
        println!("{}", serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?);
        return Ok(ExitCode::SUCCESS);
    }

    for group in report.groups.iter().take(args.top) {
        println!(
            "{:>9} récupérables — {} copies de {}",
            labels::bytes(group.reclaimable()),
            group.files.len(),
            labels::bytes(group.size)
        );
        for file in &group.files {
            let role = if file.keep { "garder " } else { "doublon" };
            println!("    {role}  {}", file.path.display());
        }
    }
    let total: u64 = report.groups.iter().map(|g| g.reclaimable()).sum();
    println!(
        "\n{} groupes de doublons, {} récupérables. {} groupes de clones APFS ignorés (ils ne libéreraient rien). Analyse en {:.1} s.",
        report.groups.len(),
        labels::bytes(total),
        report.clone_groups,
        started.elapsed().as_secs_f64()
    );
    print_denied(&report.denied);
    Ok(ExitCode::SUCCESS)
}

fn show_memory(args: &Args) -> Result<ExitCode, String> {
    let snapshot = memory::snapshot().map_err(|e| e.to_string())?;
    if args.has("--json") {
        println!(
            "{}",
            serde_json::to_string_pretty(&snapshot).map_err(|e| e.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }
    let stats = &snapshot.stats;
    println!(
        "Mémoire utilisée : {} sur {} (apps {}, verrouillée {}, compressée {}) · pression {}",
        labels::bytes(stats.used),
        labels::bytes(stats.total),
        labels::bytes(stats.app),
        labels::bytes(stats.wired),
        labels::bytes(stats.compressed),
        labels::pressure(stats.pressure)
    );
    println!(
        "Fichiers en cache : {} (rendus à la demande) · swap : {} sur {}\n",
        labels::bytes(stats.cached_files),
        labels::bytes(stats.swap_used),
        labels::bytes(stats.swap_total)
    );
    for group in snapshot.groups.iter().take(args.top) {
        let count = match group.processes.len() {
            1 => String::new(),
            n => format!(" ({n} processus)"),
        };
        let lock = if group.quittable { "" } else { "  [protégé]" };
        println!("{:>9}  {}{count}{lock}", labels::bytes(group.footprint), group.name);
    }
    println!(
        "\n{} processus d'autres utilisateurs (WindowServer, services système) ne sont pas mesurables sans droits administrateur ; ils sont inclus dans la mémoire utilisée.",
        snapshot.unmeasured
    );
    Ok(ExitCode::SUCCESS)
}

fn list_apps(home: &Path, args: &Args) -> Result<ExitCode, String> {
    let apps = apps::list_apps(&ScanState::default(), home);
    if args.has("--json") {
        println!("{}", serde_json::to_string_pretty(&apps).map_err(|e| e.to_string())?);
        return Ok(ExitCode::SUCCESS);
    }
    for app in &apps {
        println!("{:>9}  {:<32}  {}", labels::bytes(app.bytes), app.name, app.bundle_id);
    }
    Ok(ExitCode::SUCCESS)
}

fn uninstall(home: &Path, args: &Args) -> Result<ExitCode, String> {
    let query = args.positional.first().ok_or("indique l'app à désinstaller")?;
    let state = ScanState::default();
    let installed = apps::list_apps(&state, home);
    let app = find_app(query, &installed, &state, home)?;

    let brew_cask = leftovers::brew_cask_for(&app, home);
    let plan = leftovers::plan(&state, &Roots::system(home), &app, &installed, brew_cask);
    println!("{} ({}) — {}\n", app.name, app.bundle_id, app.path.display());
    for (kind, items) in leftovers::by_kind(&plan.items) {
        println!("{}", labels::leftover_kind(kind));
        for item in items {
            let mark = if item.selected { "[x]" } else { "[ ]" };
            let locked = if item.needs_authorization {
                "  (autorisation requise)"
            } else {
                ""
            };
            let note = item.note.as_deref().map(|n| format!("  — {n}")).unwrap_or_default();
            println!(
                "  {mark} {:>9}  {}{locked}{note}",
                labels::bytes(item.bytes),
                item.path.display()
            );
        }
    }
    if let Some(cask) = &plan.brew_cask {
        println!("\nInstallée par Homebrew : --brew lance « brew uninstall --cask --zap {cask} » en premier.");
    }
    println!("\nSélection par défaut : {}", labels::bytes(plan.selected_bytes()));

    if !args.has("--apply") {
        println!("Aucune suppression : relance avec --apply pour tout mettre à la corbeille.");
        return Ok(ExitCode::SUCCESS);
    }
    let ids: Vec<String> = plan.items.iter().filter(|i| i.selected).map(|i| i.id.clone()).collect();
    Ok(print_execution(&plan.execute(&ids, args.has("--brew"), home)))
}

fn debloat_report(home: &Path, args: &Args) -> Result<ExitCode, String> {
    let state = ScanState::default();
    let installed = apps::list_apps(&state, home);
    let report = debloat::scan(&state, home, Path::new("/"), &installed);
    if args.has("--json") {
        println!("{}", serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?);
        return Ok(ExitCode::SUCCESS);
    }

    println!("En arrière-plan (* = ne lance plus rien, proposé à la suppression) :");
    for item in &report.background {
        let mark = if item.selected { "*" } else { " " };
        let loaded = match item.loaded {
            Some(true) => "actif",
            Some(false) => "inactif",
            None => "",
        };
        println!(
            "{mark} {:<16}  {:<7}  {:<24}  {:<44}  {}",
            labels::scope(item.scope),
            loaded,
            labels::schedule(item.schedule),
            item.label,
            labels::owner(&item.owner)
        );
    }
    println!(
        "\n{} apps intégrées à macOS ({}) : elles sont sur le volume système scellé et ne peuvent pas être supprimées.",
        report.system_apps,
        labels::bytes(report.system_apps_bytes)
    );
    for app in &report.removable_apple_apps {
        println!(
            "{:>9}  {} : app d'Apple supprimable (debarras uninstall \"{}\")",
            labels::bytes(app.bytes),
            app.name,
            app.name
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Accepts a bundle path, a bundle id or an app name (case-insensitive).
fn find_app(query: &str, installed: &[AppInfo], state: &ScanState, home: &Path) -> Result<AppInfo, String> {
    if query.ends_with(".app") && Path::new(query).exists() {
        let path = Path::new(query).canonicalize().map_err(|e| e.to_string())?;
        return apps::inspect_app(state, &path, home).map_err(|e| e.to_string());
    }
    installed
        .iter()
        .find(|app| app.bundle_id.eq_ignore_ascii_case(query) || app.name.eq_ignore_ascii_case(query))
        .cloned()
        .ok_or_else(|| format!("aucune app installée ne correspond à « {query} »"))
}
