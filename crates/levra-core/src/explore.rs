//! Full size tree of a folder, built once and then browsed instantly.
//!
//! Every directory becomes a node. Files at least `min_file_bytes` large
//! get their own node; smaller ones are summed into a single
//! [`NodeKind::SmallFiles`] node per directory, which keeps memory bounded
//! on a whole home folder (millions of files).

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};

use rayon::prelude::*;

use crate::walk::{Bounds, HardLinks, ScanState, allocated};

pub type NodeId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NodeKind {
    Directory,
    File,
    /// Files below the size threshold of one directory, summed together.
    SmallFiles,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub kind: NodeKind,
    pub bytes: u64,
    /// Number of files and folders inside (1 for a file).
    pub items: u64,
    pub modified: Option<SystemTime>,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
}

#[derive(Debug, Clone)]
pub struct Tree {
    root_path: PathBuf,
    nodes: Vec<Node>,
    min_file_bytes: u64,
}

impl Tree {
    pub const ROOT: NodeId = 0;

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Children sorted from largest to smallest.
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.nodes.get(id).map_or(&[], |n| n.children.as_slice())
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes.get(id)?.parent
    }

    /// Location on disk. A small-files node resolves to its directory.
    pub fn path(&self, id: NodeId) -> Option<PathBuf> {
        let mut names = Vec::new();
        let mut current = id;
        while current != Self::ROOT {
            let node = self.nodes.get(current)?;
            if node.kind != NodeKind::SmallFiles {
                names.push(node.name.as_str());
            }
            current = node.parent?;
        }
        let mut path = self.root_path.clone();
        path.extend(names.iter().rev());
        Some(path)
    }

    /// Largest individual files, optionally only those untouched since `before`.
    pub fn largest_files(&self, limit: usize, min_bytes: u64, before: Option<SystemTime>) -> Vec<NodeId> {
        let mut files: Vec<NodeId> = (0..self.nodes.len())
            .filter(|&id| {
                let node = &self.nodes[id];
                node.kind == NodeKind::File
                    && node.bytes >= min_bytes
                    && before.is_none_or(|limit| node.modified.is_some_and(|m| m < limit))
            })
            .collect();
        let largest_first = |&a: &NodeId, &b: &NodeId| self.nodes[b].bytes.cmp(&self.nodes[a].bytes);
        // on isole les `limit` plus gros avant de trier, au lieu de trier des centaines de milliers de fichiers
        if limit < files.len() {
            files.select_nth_unstable_by(limit, largest_first);
            files.truncate(limit);
        }
        files.sort_unstable_by(largest_first);
        files
    }

    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    /// The directory node at `path`, if the tree holds it.
    pub fn find(&self, path: &Path) -> Option<NodeId> {
        let relative = path.strip_prefix(&self.root_path).ok()?;
        let mut current = Self::ROOT;
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return None;
            };
            let name = name.to_str()?;
            current = *self.nodes[current]
                .children
                .iter()
                .find(|&&child| self.nodes[child].kind == NodeKind::Directory && self.nodes[child].name == name)?;
        }
        Some(current)
    }

    fn depth(&self, mut id: NodeId) -> usize {
        let mut depth = 0;
        while let Some(parent) = self.nodes[id].parent {
            depth += 1;
            id = parent;
        }
        depth
    }

    /// Brings the tree up to date after changes inside these folders (as
    /// reported by FSEvents), re-reading only their own level: their
    /// subfolders that did not change keep their measurement.
    pub fn refresh(&mut self, state: &ScanState, changed: &[PathBuf]) -> io::Result<()> {
        let root_meta = fs::symlink_metadata(&self.root_path)?;
        let links = HardLinks::default();
        let ctx = Context {
            bounds: Bounds::new(&self.root_path, &root_meta),
            links: &links,
            min_file_bytes: self.min_file_bytes,
        };

        // un dossier créé ou supprimé n'est pas ds l'arbre : c son parent le plus proche qu'on relit
        let mut dirty: HashSet<NodeId> = HashSet::new();
        for path in changed {
            let mut path = path.as_path();
            loop {
                if let Some(id) = self.find(path) {
                    dirty.insert(id);
                    break;
                }
                match path.parent() {
                    Some(parent) if parent.starts_with(&self.root_path) => path = parent,
                    _ => break,
                }
            }
        }

        // du plus profond au moins profond pour que chq parent recalcule avec des enfants déjà à jour
        let mut order: Vec<(usize, NodeId)> = dirty.iter().map(|&id| (self.depth(id), id)).collect();
        order.sort_unstable_by(|a, b| b.cmp(a));
        let mut ancestors: HashSet<NodeId> = HashSet::new();
        for (_, id) in order {
            if state.cancel.is_cancelled() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "annulé"));
            }
            let Some(path) = self.path(id) else { continue };
            self.reread_level(state, &ctx, id, &path);
            let mut current = self.nodes[id].parent;
            while let Some(parent) = current {
                ancestors.insert(parent);
                current = self.nodes[parent].parent;
            }
        }

        let mut order: Vec<(usize, NodeId)> = ancestors.into_iter().map(|id| (self.depth(id), id)).collect();
        order.sort_unstable_by(|a, b| b.cmp(a));
        for (_, id) in order {
            self.recompute(id);
        }
        Ok(())
    }

    /// Lists `id` again: unchanged subfolders are kept, new ones measured,
    /// files read afresh.
    fn reread_level(&mut self, state: &ScanState, ctx: &Context, id: NodeId, path: &Path) {
        let Some(level) = state.read_level(path, &ctx.bounds, ctx.links) else {
            return;
        };
        let old: Vec<NodeId> = std::mem::take(&mut self.nodes[id].children);
        let mut children = Vec::with_capacity(level.subdirs.len() + level.files.len() + 1);

        let built: Vec<(Option<NodeId>, Option<BuiltDir>)> = level
            .subdirs
            .into_par_iter()
            .map(|sub| {
                let name = display_name(sub.name);
                let existing = old
                    .iter()
                    .copied()
                    .find(|&child| self.nodes[child].kind == NodeKind::Directory && self.nodes[child].name == name);
                match existing {
                    Some(child) => (Some(child), None),
                    None => (
                        None,
                        Some(build_dir(state, ctx, &sub.path, name, sub.own_bytes, sub.modified)),
                    ),
                }
            })
            .collect();
        for (existing, fresh) in built {
            match (existing, fresh) {
                (Some(child), _) => children.push(child),
                (None, Some(dir)) => children.push(self.insert(dir, Some(id))),
                (None, None) => {}
            }
        }

        let (mut small_bytes, mut small_count) = (0, 0);
        for file in level.files {
            if file.bytes >= ctx.min_file_bytes {
                children.push(self.leaf(
                    display_name(file.name),
                    NodeKind::File,
                    file.bytes,
                    1,
                    file.modified,
                    id,
                ));
            } else {
                small_bytes += file.bytes;
                small_count += 1;
            }
        }
        if small_count > 0 {
            let name = format!("{small_count} petits fichiers");
            children.push(self.leaf(name, NodeKind::SmallFiles, small_bytes, small_count, None, id));
        }
        self.nodes[id].children = children;
        self.recompute(id);
    }

    /// Size and item count of a directory from its children.
    fn recompute(&mut self, id: NodeId) {
        let mut children = std::mem::take(&mut self.nodes[id].children);
        children.sort_unstable_by(|&a, &b| self.nodes[b].bytes.cmp(&self.nodes[a].bytes));
        let bytes = children.iter().map(|&c| self.nodes[c].bytes).sum();
        let items = children
            .iter()
            .map(|&c| {
                let child = &self.nodes[c];
                if child.kind == NodeKind::Directory {
                    child.items + 1
                } else {
                    child.items
                }
            })
            .sum();
        let node = &mut self.nodes[id];
        node.children = children;
        node.bytes = bytes;
        node.items = items;
    }

    /// Writes the tree to `path`, with `stamp` (the FSEvents event id it is
    /// up to date with). Nodes left behind by refreshes are not written.
    pub fn save(&self, path: &Path, stamp: u64) -> io::Result<()> {
        let temporary = path.with_extension("tmp");
        let mut out = BufWriter::new(File::create(&temporary)?);
        out.write_all(CACHE_MAGIC)?;
        write_bytes(&mut out, self.root_path.as_os_str().as_encoded_bytes())?;
        out.write_all(&stamp.to_le_bytes())?;
        out.write_all(&self.min_file_bytes.to_le_bytes())?;
        // préfixe en profondeur : chq nœud arrive après son parent, donc on peut tt reconstruire
        let mut stack = vec![(Self::ROOT, u32::MAX)];
        let mut written: u32 = 0;
        let mut body = Vec::new();
        while let Some((id, parent)) = stack.pop() {
            let node = &self.nodes[id];
            let index = written;
            written += 1;
            write_bytes(&mut body, node.name.as_bytes())?;
            body.push(node.kind as u8);
            body.extend_from_slice(&node.bytes.to_le_bytes());
            body.extend_from_slice(&node.items.to_le_bytes());
            let modified = node.modified.map_or(u64::MAX, crate::time::unix_seconds);
            body.extend_from_slice(&modified.to_le_bytes());
            body.extend_from_slice(&parent.to_le_bytes());
            for &child in node.children.iter().rev() {
                stack.push((child, index));
            }
        }
        out.write_all(&written.to_le_bytes())?;
        out.write_all(&body)?;
        out.into_inner().map_err(io::IntoInnerError::into_error)?.sync_all()?;
        fs::rename(temporary, path)
    }

    /// Reads a tree written by [`Tree::save`], with its stamp.
    pub fn load(path: &Path) -> io::Result<(Tree, u64)> {
        let mut input = BufReader::new(File::open(path)?);
        let mut magic = [0u8; 8];
        input.read_exact(&mut magic)?;
        if &magic != CACHE_MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "cache d'une autre version"));
        }
        let root_path = PathBuf::from(String::from_utf8(read_bytes(&mut input)?).map_err(invalid)?);
        let stamp = read_u64(&mut input)?;
        let min_file_bytes = read_u64(&mut input)?;
        let count = read_u32(&mut input)? as usize;
        let mut nodes: Vec<Node> = Vec::with_capacity(count);
        for _ in 0..count {
            let name = String::from_utf8(read_bytes(&mut input)?).map_err(invalid)?;
            let mut kind = [0u8];
            input.read_exact(&mut kind)?;
            let kind = match kind[0] {
                0 => NodeKind::Directory,
                1 => NodeKind::File,
                2 => NodeKind::SmallFiles,
                _ => return Err(invalid("type de nœud inconnu")),
            };
            let bytes = read_u64(&mut input)?;
            let items = read_u64(&mut input)?;
            let modified = match read_u64(&mut input)? {
                u64::MAX => None,
                secs => Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs)),
            };
            let parent = match read_u32(&mut input)? {
                u32::MAX => None,
                index => Some(index as usize),
            };
            let id = nodes.len();
            if let Some(parent) = parent {
                nodes
                    .get_mut(parent)
                    .ok_or_else(|| invalid("parent manquant"))?
                    .children
                    .push(id);
            }
            nodes.push(Node {
                name,
                kind,
                bytes,
                items,
                modified,
                parent,
                children: Vec::new(),
            });
        }
        if nodes.is_empty() {
            return Err(invalid("cache vide"));
        }
        Ok((
            Tree {
                root_path,
                nodes,
                min_file_bytes,
            },
            stamp,
        ))
    }

    fn insert(&mut self, dir: BuiltDir, parent: Option<NodeId>) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node {
            name: dir.name,
            kind: NodeKind::Directory,
            bytes: dir.bytes,
            items: dir.items,
            modified: dir.modified,
            parent,
            children: Vec::new(),
        });

        let mut children = Vec::with_capacity(dir.subdirs.len() + dir.files.len() + 1);
        for sub in dir.subdirs {
            children.push(self.insert(sub, Some(id)));
        }
        for file in dir.files {
            children.push(self.leaf(file.name, NodeKind::File, file.bytes, 1, file.modified, id));
        }
        if dir.small_count > 0 {
            let name = format!("{} petits fichiers", dir.small_count);
            children.push(self.leaf(name, NodeKind::SmallFiles, dir.small_bytes, dir.small_count, None, id));
        }
        children.sort_unstable_by(|&a, &b| self.nodes[b].bytes.cmp(&self.nodes[a].bytes));
        self.nodes[id].children = children;
        id
    }

    fn leaf(
        &mut self,
        name: String,
        kind: NodeKind,
        bytes: u64,
        items: u64,
        modified: Option<SystemTime>,
        parent: NodeId,
    ) -> NodeId {
        self.nodes.push(Node {
            name,
            kind,
            bytes,
            items,
            modified,
            parent: Some(parent),
            children: Vec::new(),
        });
        self.nodes.len() - 1
    }
}

const CACHE_MAGIC: &[u8; 8] = b"LVRTREE1";

fn invalid(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

fn write_bytes(out: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    out.write_all(&(bytes.len() as u32).to_le_bytes())?;
    out.write_all(bytes)
}

fn read_bytes(input: &mut impl Read) -> io::Result<Vec<u8>> {
    let len = read_u32(input)? as usize;
    // un nom de fichier fait jamais 1 Mo, si c le cas le fichier est pourri
    if len > 1 << 20 {
        return Err(invalid("nom trop long"));
    }
    let mut bytes = vec![0u8; len];
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn read_u32(input: &mut impl Read) -> io::Result<u32> {
    let mut bytes = [0u8; 4];
    input.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(input: &mut impl Read) -> io::Result<u64> {
    let mut bytes = [0u8; 8];
    input.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

/// Default threshold under which files are grouped per directory.
pub const DEFAULT_MIN_FILE_BYTES: u64 = 1 << 20;

pub fn explore(state: &ScanState, root: &Path, min_file_bytes: u64) -> io::Result<Tree> {
    let meta = fs::symlink_metadata(root)?;
    if !meta.is_dir() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a directory"));
    }
    let links = HardLinks::default();
    let built = build_dir(
        state,
        &Context {
            bounds: Bounds::new(root, &meta),
            links: &links,
            min_file_bytes,
        },
        root,
        display_name(root.as_os_str().to_os_string()),
        allocated(&meta),
        meta.modified().ok(),
    );
    let mut tree = Tree {
        root_path: root.to_path_buf(),
        nodes: Vec::new(),
        min_file_bytes,
    };
    tree.insert(built, None);
    Ok(tree)
}

struct Context<'a> {
    bounds: Bounds,
    links: &'a HardLinks,
    min_file_bytes: u64,
}

struct BuiltDir {
    name: String,
    bytes: u64,
    items: u64,
    modified: Option<SystemTime>,
    subdirs: Vec<BuiltDir>,
    files: Vec<BuiltFile>,
    small_bytes: u64,
    small_count: u64,
}

struct BuiltFile {
    name: String,
    bytes: u64,
    modified: Option<SystemTime>,
}

fn build_dir(
    state: &ScanState,
    ctx: &Context,
    path: &Path,
    name: String,
    own_bytes: u64,
    modified: Option<SystemTime>,
) -> BuiltDir {
    let level = state.read_level(path, &ctx.bounds, ctx.links).unwrap_or_default();

    let subdirs: Vec<BuiltDir> = level
        .subdirs
        .into_par_iter()
        .map(|sub| {
            build_dir(
                state,
                ctx,
                &sub.path,
                display_name(sub.name),
                sub.own_bytes,
                sub.modified,
            )
        })
        .collect();

    let mut files = Vec::new();
    let (mut small_bytes, mut small_count) = (0, 0);
    for file in level.files {
        if file.bytes >= ctx.min_file_bytes {
            files.push(BuiltFile {
                name: display_name(file.name),
                bytes: file.bytes,
                modified: file.modified,
            });
        } else {
            small_bytes += file.bytes;
            small_count += 1;
        }
    }

    let bytes = own_bytes
        + small_bytes
        + files.iter().map(|f| f.bytes).sum::<u64>()
        + subdirs.iter().map(|d| d.bytes).sum::<u64>();
    let items = small_count + files.len() as u64 + subdirs.iter().map(|d| d.items + 1).sum::<u64>();

    BuiltDir {
        name,
        bytes,
        items,
        modified,
        subdirs,
        files,
        small_bytes,
        small_count,
    }
}

/// APFS only stores valid UTF-8 names, so the lossy branch is for other
/// file systems (USB keys, network shares).
fn display_name(name: OsString) -> String {
    name.into_string()
        .unwrap_or_else(|raw| raw.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::time::Duration;

    fn write_file(path: &Path, bytes: usize) {
        File::create(path).unwrap().write_all(&vec![1u8; bytes]).unwrap();
    }

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("big/deeper")).unwrap();
        fs::create_dir(dir.path().join("small")).unwrap();
        write_file(&dir.path().join("big/deeper/video.mov"), 3_000_000);
        write_file(&dir.path().join("big/notes.txt"), 2_000);
        write_file(&dir.path().join("small/a.txt"), 1_000);
        write_file(&dir.path().join("small/b.txt"), 1_000);
        dir
    }

    fn child_named<'a>(tree: &'a Tree, parent: NodeId, name: &str) -> (NodeId, &'a Node) {
        tree.children(parent)
            .iter()
            .map(|&id| (id, tree.node(id).unwrap()))
            .find(|(_, n)| n.name == name)
            .unwrap_or_else(|| panic!("no child named {name}"))
    }

    #[test]
    fn directory_sizes_include_their_contents() {
        let dir = fixture();
        let tree = explore(&ScanState::default(), dir.path(), DEFAULT_MIN_FILE_BYTES).unwrap();

        let root = tree.node(Tree::ROOT).unwrap();
        let (big_id, big) = child_named(&tree, Tree::ROOT, "big");
        let (_, deeper) = child_named(&tree, big_id, "deeper");
        assert!(deeper.bytes >= 3_000_000);
        assert!(big.bytes > deeper.bytes);
        assert!(root.bytes >= big.bytes);
        assert_eq!(root.items, 7, "3 folders + 4 files");
    }

    #[test]
    fn children_are_sorted_largest_first() {
        let dir = fixture();
        let tree = explore(&ScanState::default(), dir.path(), DEFAULT_MIN_FILE_BYTES).unwrap();

        let sizes: Vec<u64> = tree
            .children(Tree::ROOT)
            .iter()
            .map(|&id| tree.node(id).unwrap().bytes)
            .collect();
        assert!(sizes.windows(2).all(|w| w[0] >= w[1]), "{sizes:?}");
        assert_eq!(tree.node(tree.children(Tree::ROOT)[0]).unwrap().name, "big");
    }

    #[test]
    fn small_files_are_grouped_per_directory() {
        let dir = fixture();
        let tree = explore(&ScanState::default(), dir.path(), DEFAULT_MIN_FILE_BYTES).unwrap();

        let (small_id, _) = child_named(&tree, Tree::ROOT, "small");
        let grouped = tree.children(small_id);
        assert_eq!(grouped.len(), 1);
        let node = tree.node(grouped[0]).unwrap();
        assert_eq!(node.kind, NodeKind::SmallFiles);
        assert_eq!(node.items, 2);
        assert_eq!(tree.path(grouped[0]).unwrap(), dir.path().join("small"));
    }

    #[test]
    fn paths_are_rebuilt_from_the_root() {
        let dir = fixture();
        let tree = explore(&ScanState::default(), dir.path(), DEFAULT_MIN_FILE_BYTES).unwrap();

        let video = tree.largest_files(1, 0, None)[0];
        assert_eq!(tree.path(video).unwrap(), dir.path().join("big/deeper/video.mov"));
        assert_eq!(tree.path(Tree::ROOT).unwrap(), dir.path());
    }

    #[test]
    fn largest_files_can_require_old_files() {
        let dir = fixture();
        let tree = explore(&ScanState::default(), dir.path(), DEFAULT_MIN_FILE_BYTES).unwrap();

        let a_day_ago = SystemTime::now() - Duration::from_secs(86_400);
        assert!(tree.largest_files(10, 0, Some(a_day_ago)).is_empty());
        assert_eq!(tree.largest_files(10, 1_000_000, None).len(), 1);
    }

    #[test]
    fn a_saved_tree_loads_back_identical() {
        let dir = fixture();
        let tree = explore(&ScanState::default(), dir.path(), DEFAULT_MIN_FILE_BYTES).unwrap();
        let cache = dir.path().join("cache.bin");
        tree.save(&cache, 42).unwrap();

        let (loaded, stamp) = Tree::load(&cache).unwrap();
        assert_eq!(stamp, 42);
        assert_eq!(loaded.nodes.len(), tree.nodes.len());
        assert_eq!(
            loaded.node(Tree::ROOT).unwrap().bytes,
            tree.node(Tree::ROOT).unwrap().bytes
        );
        let video = loaded.largest_files(1, 0, None)[0];
        assert_eq!(loaded.path(video).unwrap(), dir.path().join("big/deeper/video.mov"));
    }

    #[test]
    fn a_refresh_rereads_only_what_changed() {
        let dir = fixture();
        let mut tree = explore(&ScanState::default(), dir.path(), DEFAULT_MIN_FILE_BYTES).unwrap();
        let before = tree.node(Tree::ROOT).unwrap().bytes;
        let deeper = tree.find(&dir.path().join("big/deeper")).unwrap();

        // un gros fichier en plus ds small, un nouveau dossier, et big/notes.txt supprimé
        write_file(&dir.path().join("small/movie.mp4"), 2_000_000);
        fs::create_dir(dir.path().join("new")).unwrap();
        write_file(&dir.path().join("new/data.bin"), 1_500_000);
        fs::remove_file(dir.path().join("big/notes.txt")).unwrap();
        tree.refresh(
            &ScanState::default(),
            &[dir.path().join("small"), dir.path().join("new"), dir.path().join("big")],
        )
        .unwrap();

        let root = tree.node(Tree::ROOT).unwrap();
        assert!(root.bytes >= before + 3_400_000, "{} vs {before}", root.bytes);
        assert_eq!(root.items, 9, "4 dossiers + 5 fichiers");
        assert_eq!(
            tree.find(&dir.path().join("big/deeper")),
            Some(deeper),
            "sous-dossier inchangé gardé"
        );
        assert!(tree.find(&dir.path().join("new")).is_some());
        let sizes: Vec<u64> = tree
            .children(Tree::ROOT)
            .iter()
            .map(|&id| tree.node(id).unwrap().bytes)
            .collect();
        assert!(sizes.windows(2).all(|w| w[0] >= w[1]), "{sizes:?}");
        let (_, big) = child_named(&tree, Tree::ROOT, "big");
        assert_eq!(big.items, 2, "deeper + video");
    }

    #[test]
    #[ignore = "lit le vrai dossier perso"]
    fn caching_the_home_folder_is_fast() {
        let home = PathBuf::from(std::env::var("HOME").unwrap());
        let started = std::time::Instant::now();
        let tree = explore(&ScanState::default(), &home, DEFAULT_MIN_FILE_BYTES).unwrap();
        let scanned = started.elapsed();
        let cache = std::env::temp_dir().join("levra-home-cache.bin");
        let started = std::time::Instant::now();
        tree.save(&cache, 1).unwrap();
        let saved = started.elapsed();
        let started = std::time::Instant::now();
        let (loaded, _) = Tree::load(&cache).unwrap();
        let loaded_in = started.elapsed();
        let size = fs::metadata(&cache).unwrap().len();
        let mut refreshed = loaded.clone();
        let started = std::time::Instant::now();
        refreshed
            .refresh(
                &ScanState::default(),
                &[home.join("Downloads"), home.join("Library/Caches")],
            )
            .unwrap();
        let refresh = started.elapsed();
        println!(
            "{} nœuds : mesure {scanned:?}, sauvegarde {saved:?}, chargement {loaded_in:?} ({size} octets), rafraîchissement {refresh:?}",
            tree.nodes.len()
        );
        assert_eq!(
            loaded.node(Tree::ROOT).unwrap().bytes,
            tree.node(Tree::ROOT).unwrap().bytes
        );
        let _ = fs::remove_file(cache);
    }

    #[test]
    fn exploring_a_file_is_an_error() {
        let dir = fixture();
        let err = explore(&ScanState::default(), &dir.path().join("big/notes.txt"), 0).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
