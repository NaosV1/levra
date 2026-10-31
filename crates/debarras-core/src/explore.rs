//! Full size tree of a folder, built once and then browsed instantly.
//!
//! Every directory becomes a node. Files at least `min_file_bytes` large
//! get their own node; smaller ones are summed into a single
//! [`NodeKind::SmallFiles`] node per directory, which keeps memory bounded
//! on a whole home folder (millions of files).

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rayon::prelude::*;

use crate::walk::{HardLinks, ScanState, allocated};

pub type NodeId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug)]
pub struct Tree {
    root_path: PathBuf,
    nodes: Vec<Node>,
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
        files.sort_unstable_by(|&a, &b| self.nodes[b].bytes.cmp(&self.nodes[a].bytes));
        files.truncate(limit);
        files
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
            dev: meta.dev(),
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
    };
    tree.insert(built, None);
    Ok(tree)
}

struct Context<'a> {
    dev: u64,
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
    let level = state.read_level(path, ctx.dev, ctx.links).unwrap_or_default();

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
    fn exploring_a_file_is_an_error() {
        let dir = fixture();
        let err = explore(&ScanState::default(), &dir.path().join("big/notes.txt"), 0).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
