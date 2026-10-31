# Levra

Faire de la place sur un Mac sans rien casser, et désinstaller les apps complètement.

- **Accueil** : l'espace disque, ce qui peut partir sans risque, les apps inutilisées, la corbeille (que le Finder vide, mot de passe compris), la mémoire et ce qui tourne en arrière-plan, chacun menant à sa page. Au premier lancement, une fenêtre présente l'app et l'accès complet au disque.
- **Nettoyage** : trouve ce qui peut partir (builds de projets, caches de paquets et d'apps, Xcode, simulateurs, Android, Docker, worktrees git, installeurs, corbeille, sauvegardes iPhone) classé du plus sûr au plus délicat ; pour chaque élément, sa taille réelle et, sur demande, l'action exacte qui sera exécutée.
- **Explorateur** : mesure le dossier personnel dès l'ouverture (ou un autre dossier, ≈ 5× plus vite que `du`), puis navigation instantanée. Mesuré depuis `/`, les dossiers de macOS sont regroupés à part.
- **Gros fichiers** : les plus gros fichiers, filtrables par taille et par ancienneté, avec leur emplacement dans les mots du Finder. Les fichiers système sont signalés et ne peuvent pas être cochés.
- **Doublons** : fichiers au contenu identique dans le dossier personnel, avec l'espace *réellement* libérable. Les copies clonées par le Finder (clones APFS), qui partagent leurs données et ne libéreraient rien, sont écartées.
- **Mémoire** :
  - la mémoire vive comptée comme le Moniteur d'activité (apps, verrouillée, compressée, cache, swap, pression) ;
  - les apps et programmes qui en consomment le plus, chaque app regroupant ses processus d'aide ;
  - un bouton pour les quitter, et une alerte quand la pression devient critique.

  Il n'y a pas de bouton « libérer la RAM » : `purge` ne vide qu'un cache que macOS rend de lui-même, et seul quitter ce qui consomme libère vraiment de la mémoire.
- **Désinstallation complète** : l'app et tout ce qu'elle laisse (préférences, conteneurs, caches, agents de démarrage, services, outils privilégiés, reçus…), en tenant compte des fichiers partagés avec d'autres apps installées. `brew uninstall --zap` est proposé quand l'app vient de Homebrew. Les apps pas ouvertes depuis 6 mois sont regroupées et peuvent partir ensemble, avec une seule demande de mot de passe.
- **Allègement** : ce que les apps lancent en arrière-plan (agents et services launchd : outils de mise à jour, assistants), rattaché à l'app installée dont il dépend. Ceux qu'aucune app installée n'explique sont signalés, et ceux qui ne lancent plus rien sont proposés à la suppression. La page dit aussi ce qu'il en est des apps d'Apple : Bourse, News, Astuces et les autres apps intégrées à macOS sont sur le volume système scellé et ne peuvent pas être supprimées ; GarageBand, iMovie, Pages, Numbers et Keynote, si elles sont installées, se désinstallent comme les autres.
- **Détection** : quand une app disparaît de `/Applications` (glissée à la corbeille, supprimée depuis le Launchpad), une notification propose d'effacer ses restes.
- **Barre des menus** : espace libre, alerte sous un seuil, et nettoyage rapide (la sélection sans risque, après confirmation) sans ouvrir la fenêtre.

## Architecture

```
crates/levra-core      Bibliothèque Rust : toute la logique (scan, règles, désinstallation, suppression)
crates/levra-cli       `levra`, la même chose en ligne de commande
crates/levra-ffi       Bindings Swift générés par UniFFI
app/                   App SwiftUI (macOS 14+), qui n'appelle que levra-ffi
scripts/build-app.sh   Rust → bindings → XCFramework → app → build/Levra.app
```

Dans `app/Sources/Levra/Design` :

| Fichier | Rôle |
|---|---|
| `Theme.swift` | Couleurs (clair et sombre), styles de texte, rayons et marges : tout écran s'y réfère. |
| `Motion.swift` | Courbes et durées des animations, plus les transitions d'entrée et la cascade des listes. Les animations sont courtes et en « ease-out », jamais déclenchées par un raccourci clavier, et réduites à des fondus si « Réduire les animations » est activé. |
| `Controls.swift`, `Surfaces.swift` | Composants : boutons, cases à cocher, pastilles, sélecteurs, cartes, bannières, états vides, barre d'action. |
| `Papicons.swift` | Icônes [PapIcons](https://pub.dev/packages/papicons) converties en tracés SwiftUI par `scripts/papicons.py` (ne pas modifier à la main). |
| `LevraMark.swift` | Le logo : un carré ouvert dont le dernier trait s'échappe. Il sert dans l'app et à l'icône, que `scripts/build-app.sh` dessine à chaque build (via `scripts/make-icon.swift`, au format du modèle d'icône macOS) : pour la changer, on ne modifie que ce fichier. |

Pour ajouter une icône, ajoute son nom à `ICONS` dans `scripts/papicons.py`, télécharge le paquet PapIcons, puis lance :

```sh
scripts/papicons.py chemin/vers/papicons/lib/styles/stroke_rounded.dart
```

La licence MIT de PapIcons figure dans `THIRD-PARTY-NOTICES.md`.

Dans `levra-core` :

| Module | Rôle |
|---|---|
| `walk` | Parcours parallèle (rayon). Taille *allouée* comme `du` : fichiers creux au réel, liens physiques comptés une fois, sans changer de volume. |
| `explore` | Arbre complet d'un dossier ; les fichiers < 1 Mo sont regroupés par dossier pour borner la mémoire. |
| `catalog/*` | Un détecteur par famille. Un dossier n'est signalé que si un marqueur fiable prouve ce qu'il est (`CACHEDIR.TAG`, `pyvenv.cfg`, fichier de projet voisin…). |
| `duplicates` | Trois passes pour lire le moins possible : même taille, puis hachage BLAKE3 du début et de la fin, puis du contenu entier. Espace libérable lu avec `getattrlist(ATTR_CMNEXT_PRIVATESIZE)`. |
| `memory` | Chiffres système via `host_statistics64` et `sysctl`, empreinte physique de chaque processus via `proc_pid_rusage` (la colonne « Mémoire » du Moniteur d'activité), regroupement par identifiant de bundle, arrêt des programmes en ligne de commande. |
| `apps`, `leftovers` | Identité des apps (bundle id, équipe de signature, dernière ouverture) et recherche des restes. |
| `launchd` | Lecture des agents et services (`~/Library/LaunchAgents`, `/Library/LaunchAgents`, `/Library/LaunchDaemons`) et de leur état dans la session. |
| `debloat` | Rattache chaque agent ou service à l'app qui l'utilise (identifiant associé, programme dans l'app, sinon même éditeur), compte les apps intégrées à macOS. |
| `guard` | Dernier garde-fou avant toute suppression (voir ci-dessous). |
| `execute` | Le seul module qui supprime. |

## Sécurité

- L'analyse ne fait que lire. Rien n'est supprimé sans confirmation.
- L'interface ne transmet que des **identifiants** : les chemins et commandes restent dans Rust, l'interface ne peut pas en fabriquer.
- Chaque chemin repasse par `guard::check_removable` juste avant d'être touché : chemin absolu sans `..`, parent résolu (un lien symbolique ne peut pas rediriger ailleurs), sous une racine autorisée (`~` à 2 niveaux minimum, `/Applications`, `/Library/x/y`…), jamais un dossier conteneur (`~/Library/Caches`, `~/Library/Application Support`…).
- Trois actions possibles :
  - **corbeille** : récupérable ;
  - **suppression définitive** : réservée à ce qui se régénère seul ;
  - **commande de l'outil propriétaire** : `docker builder prune`, `xcrun simctl`, `git worktree remove`, `brew cleanup`, exécutée sans shell.
- La désinstallation met tout à la corbeille.
- Mémoire :
  - **apps** : elles sont quittées par AppKit, comme depuis le Dock, ce qui leur laisse proposer d'enregistrer.
  - **programmes en ligne de commande** : ils reçoivent `SIGTERM` (ou `SIGKILL` pour « forcer »). Juste avant le signal, le cœur revérifie que le numéro de processus désigne toujours le même exécutable, appartenant à l'utilisateur.
  - **protégés** : macOS (Finder, Dock, loginwindow…), les machines virtuelles et Levra ne sont jamais proposés.
- Allègement :
  - seuls les agents et services sans app installée peuvent être retirés : le cœur refuse ceux d'une app installée ou de Homebrew, que leur propriétaire remettrait en place ou dont il a besoin ;
  - la présélection se limite à ceux qui ne lancent plus rien (fichier vide, programme disparu) ;
  - le fichier part à la corbeille, avec l'outil privilégié d'un service système ; un agent de la session est arrêté juste avant ;
  - les apps intégrées à macOS ne sont jamais touchées : il faudrait désactiver la protection du système (SIP).
- Doublons : la suppression passe par la corbeille. Un groupe n'est touché que si l'un de ses exemplaires reste en place, intact depuis l'analyse. Une copie modifiée depuis l'analyse n'est jamais supprimée.
- Présélection :
  - **nettoyage** : seulement les éléments « sûrs », et pas les builds des projets actifs depuis moins de 30 jours ;
  - **désinstallation** : seulement les correspondances par identifiant ou par nom exacts, hors conteneurs que macOS garde fermés faute d'accès complet au disque. Une correspondance par éditeur ou par nom proche est proposée mais jamais cochée. Un fichier partagé avec une autre app installée est signalé.

## Autorisations macOS

| Autorisation | Sert à | Sans elle |
|---|---|---|
| Accès complet au disque | Voir la corbeille (et sa taille), les sauvegardes iPhone, Mail, Messages ; retirer les conteneurs des apps désinstallées | Ces dossiers sont listés comme « non analysés » ; les conteneurs sont signalés et laissés en place (macOS les ferme à toutes les autres apps, Finder compris) |
| Automatisation › Finder | Faire mettre à la corbeille par le Finder ce qui est verrouillé (fichiers de root, apps protégées) ; le Finder demande alors le mot de passe administrateur, une fois pour tout le lot | Ces éléments restent en place et sont signalés comme non traités |
| Gestion des apps | Mettre à la corbeille les apps de l'utilisateur sans passer par le Finder | Le Finder s'en charge |

## Build

Prérequis : Rust ≥ 1.88, Xcode 16+ (Swift 6), Mac Apple Silicon.

```sh
scripts/build-app.sh          # produit build/Levra.app (signée localement, runtime renforcé)
scripts/build-app.sh --open   # et la lance
```

Pour travailler sur l'interface dans Xcode : lancer le script une fois (il génère `app/Frameworks` et les bindings), puis ouvrir `app/Package.swift`.

Vérifications, les mêmes que la CI :

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

## Distribution

Levra est distribuée hors du Mac App Store : la sandbox que le Store impose l'empêcherait de lancer `docker`, `brew` ou `xcrun`, de toucher aux fichiers de root et de confier au Finder ce qui est verrouillé.

Une fois sur le Mac qui publie :

1. un certificat « Developer ID Application » dans le trousseau (Xcode › Réglages › Comptes › Gérer les certificats › +) ;
2. les identifiants de notarisation, avec un mot de passe d'app créé sur account.apple.com :

   ```sh
   xcrun notarytool store-credentials levra --apple-id <adresse> --team-id <équipe>
   ```

Puis, pour chaque version, après avoir augmenté `CFBundleShortVersionString` et `CFBundleVersion` dans `app/Info.plist` :

```sh
LEVRA_NOTARY_PROFILE=levra scripts/release.sh
```

Le script signe l'app avec le runtime renforcé (`app/Levra.entitlements` n'y autorise que les Apple Events, pour le Finder), la fait notariser et y agrafe le ticket pour qu'elle s'ouvre aussi hors ligne, puis fait de même pour `build/Levra-<version>.dmg`. Si Apple refuse, il affiche le rapport de notarisation. `LEVRA_SIGN_IDENTITY` choisit le certificat quand il y en a plusieurs.

### Avec GitHub Actions

- **À chaque push sur `main` et chaque pull request** (`.github/workflows/build.yml`) : les vérifications ci-dessus, puis le build de l'app. `Levra-<commit>.zip` se télécharge en bas de la page de l'exécution, dans l'onglet Actions, pendant 30 jours. Signée ad hoc, cette app sert à tester : au premier lancement, macOS la bloque, et il faut passer par Réglages Système › Confidentialité et sécurité › « Ouvrir quand même ».
- **À chaque tag `v<version>`** (`.github/workflows/release.yml`) : `scripts/release.sh` sur un Mac de GitHub, puis une release GitHub avec le `.dmg` notarisé, que tout le monde peut télécharger et ouvrir. Le tag doit reprendre la version de `app/Info.plist` :

  ```sh
  git tag v0.1.0
  git push origin v0.1.0
  ```

La release a besoin de ces secrets, dans Settings › Secrets and variables › Actions du dépôt :

| Secret | Contenu |
|---|---|
| `DEVELOPER_ID_P12` | Le certificat « Developer ID Application » avec sa clé privée, exporté en `.p12` depuis Trousseaux d'accès, en base64 : `base64 -i levra.p12 \| pbcopy` |
| `DEVELOPER_ID_P12_PASSWORD` | Le mot de passe choisi à l'export |
| `NOTARY_APPLE_ID` | L'adresse du compte Apple Developer |
| `NOTARY_PASSWORD` | Un mot de passe d'app créé sur account.apple.com |
| `NOTARY_TEAM_ID` | L'identifiant d'équipe, 10 caractères (developer.apple.com › Account › Membership) |

## CLI

```sh
cargo build --release -p levra-cli
target/release/levra scan                      # ce qui peut partir (* = recommandé)
target/release/levra clean --recommended        # affiche ce qui serait fait
target/release/levra clean --recommended --apply
target/release/levra explore ~/Developer --top 20
target/release/levra duplicates                # doublons, en lecture seule
target/release/levra memory                    # ce qui occupe la mémoire vive
target/release/levra apps
target/release/levra uninstall Docker           # plan de désinstallation, sans rien supprimer
target/release/levra debloat                   # agents et services, apps d'Apple (lecture seule)
```

## Limites connues

- Les clones APFS sont comptés en entier, comme avec `du`. Libérer un clone ne rend pas forcément tout l'espace affiché.
- La détection des désinstallations ne marche que si Levra tourne (barre des menus, option « Ouvrir à la connexion »).
- Les services système (`/Library/LaunchDaemons`) mis à la corbeille ne s'arrêtent qu'au redémarrage suivant.
- Les dossiers de configuration placés directement dans le dossier personnel (`~/.docker`, `~/.vscode`…) ne sont jamais proposés : le garde-fou refuse tout ce qui est à moins de deux niveaux sous `~`.
- Doublons :
  - les fichiers de moins de 1 Mo sont ignorés ;
  - les fichiers iCloud non téléchargés sont ignorés aussi, car les lire les téléchargerait ;
  - les dossiers dont le nom se termine par une extension alphabétique (`Projet.old`) ne sont pas parcourus, comme les paquets (`.app`, `.photoslibrary`).
- Mémoire :
  - les processus d'autres utilisateurs (WindowServer, services système) sont comptés mais pas mesurés, car il faudrait des droits root (`top` les mesure parce qu'il est setuid root) ;
  - une machine virtuelle (Docker Desktop, l'app Claude) apparaît à part, sans lien avec l'app qui l'a lancée.
- Allègement :
  - l'état (actif ou non) des services système ne se lit pas sans droits administrateur ; seul celui des agents est affiché ;
  - les agents déclarés à l'intérieur d'une app (`SMAppService`) ne sont pas listés : ils partent avec l'app et se désactivent dans Réglages Système › Général › Ouverture et extensions ;
  - un agent est rattaché aux apps du même éditeur quand rien de plus précis ne le relie à une app (Microsoft AutoUpdate est ainsi lié à toutes les apps Microsoft installées) ;
  - la bibliothèque de sons de GarageBand dans `/Library/Audio/Apple Loops` n'est pas reconnue comme un reste de GarageBand.
- Pas de build Intel (x86_64) pour l'instant.
