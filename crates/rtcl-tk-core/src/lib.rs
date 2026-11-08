//! Backend-independent widget model for rtcl's native Tk-compatible
//! toolkit.
//!
//! This crate deliberately knows nothing about nsh, winit, UIKit or GTK:
//! it owns the observable Tk state a script manipulates — widget paths,
//! hierarchy, options, pack/grid placement — and the rules that reject
//! invalid trees.  The Tcl command layer mutates this model; a renderer
//! backend subscribes to the resulting change stream.
//!
//! Scope: Tk 8.6-compatible *widget model* for the native track described
//! in `docs/UI-NATIVE-TK.md`.  The web/AI `ui::*` DSL is separate.

use std::collections::{BTreeMap, HashMap};

/// Widget kinds in the v1 native Tk surface.  `Toplevel` is also the root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetKind {
    Toplevel,
    Frame,
    Label,
    Button,
    Entry,
    Checkbutton,
    Radiobutton,
    Scale,
    Text,
    Listbox,
}

impl WidgetKind {
    pub fn from_tk_name(name: &str) -> Option<Self> {
        Some(match name {
            "toplevel" => Self::Toplevel,
            "frame" => Self::Frame,
            "label" => Self::Label,
            "button" => Self::Button,
            "entry" => Self::Entry,
            "checkbutton" => Self::Checkbutton,
            "radiobutton" => Self::Radiobutton,
            "scale" => Self::Scale,
            "text" => Self::Text,
            "listbox" => Self::Listbox,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PackSide {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fill {
    #[default]
    None,
    X,
    Y,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PackInfo {
    pub side: PackSide,
    pub fill: Fill,
    pub expand: bool,
    pub padx: f64,
    pub pady: f64,
}

impl Default for PackInfo {
    fn default() -> Self {
        Self {
            side: PackSide::Top,
            fill: Fill::None,
            expand: false,
            padx: 0.0,
            pady: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sticky {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridInfo {
    pub row: i64,
    pub column: i64,
    pub rowspan: i64,
    pub columnspan: i64,
    pub sticky: Sticky,
    pub padx: f64,
    pub pady: f64,
}

impl Default for GridInfo {
    fn default() -> Self {
        Self {
            row: 0,
            column: 0,
            rowspan: 1,
            columnspan: 1,
            sticky: Sticky::default(),
            padx: 0.0,
            pady: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WidgetNode {
    pub path: String,
    pub kind: WidgetKind,
    pub parent: Option<String>,
    pub children: Vec<String>,
    /// Option name including the leading `-` (`-text`), as Tk spells it.
    pub options: BTreeMap<String, String>,
    pub pack: Option<PackInfo>,
    pub grid: Option<GridInfo>,
}

impl WidgetNode {
    pub fn child(&self, path: &str) -> Option<&str> {
        self.children
            .iter()
            .find(|c| c.as_str() == path)
            .map(|s| s.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TkError {
    BadPath(String),
    NotFound(String),
    AlreadyExists(String),
    CannotDestroyRoot,
    DuplicateChild(String),
}

impl std::fmt::Display for TkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadPath(p) => write!(f, "bad window path name \"{p}\""),
            Self::NotFound(p) => write!(f, "bad window path name \"{p}\""),
            Self::AlreadyExists(p) => write!(f, "window name \"{p}\" already exists"),
            Self::CannotDestroyRoot => write!(f, "cannot destroy \".\""),
            Self::DuplicateChild(p) => write!(f, "window \"{p}\" already exists"),
        }
    }
}

impl std::error::Error for TkError {}

/// The shadow widget tree a script manipulates.
#[derive(Debug, Clone)]
pub struct TkTree {
    nodes: HashMap<String, WidgetNode>,
    root: String,
}

impl Default for TkTree {
    fn default() -> Self {
        Self::new()
    }
}

impl TkTree {
    pub fn new() -> Self {
        let root = ".".to_string();
        let mut nodes = HashMap::new();
        nodes.insert(
            root.clone(),
            WidgetNode {
                path: root.clone(),
                kind: WidgetKind::Toplevel,
                parent: None,
                children: Vec::new(),
                options: BTreeMap::new(),
                pack: None,
                grid: None,
            },
        );
        Self { nodes, root }
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    pub fn get(&self, path: &str) -> Option<&WidgetNode> {
        self.nodes.get(path)
    }

    pub fn get_mut(&mut self, path: &str) -> Option<&mut WidgetNode> {
        self.nodes.get_mut(path)
    }

    pub fn contains(&self, path: &str) -> bool {
        self.nodes.contains_key(path)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// Create a child of `parent`.  `name` is a Tk relative child name
    /// (no dot); `parent` must already exist.  Returns the full path.
    pub fn create(
        &mut self,
        parent: &str,
        name: &str,
        kind: WidgetKind,
    ) -> Result<String, TkError> {
        if !valid_path(parent) {
            return Err(TkError::BadPath(parent.to_string()));
        }
        if parent != self.root && !self.nodes.contains_key(parent) {
            return Err(TkError::NotFound(parent.to_string()));
        }
        if name.is_empty() || name.contains('.') {
            return Err(TkError::BadPath(name.to_string()));
        }
        let path = if parent == "." {
            format!(".{name}")
        } else {
            format!("{parent}.{name}")
        };
        self.create_at(&path, kind)
    }

    /// Create a widget at an absolute Tk path (parent path is derived).
    pub fn create_at(&mut self, path: &str, kind: WidgetKind) -> Result<String, TkError> {
        if !valid_path(path) || path == "." {
            return Err(TkError::BadPath(path.to_string()));
        }
        if self.nodes.contains_key(path) {
            return Err(TkError::AlreadyExists(path.to_string()));
        }
        let parent = parent_path(path).ok_or_else(|| TkError::BadPath(path.to_string()))?;
        let parent_node = self
            .nodes
            .get_mut(&parent)
            .ok_or_else(|| TkError::NotFound(parent.clone()))?;
        if parent_node.children.iter().any(|c| c == path) {
            return Err(TkError::DuplicateChild(path.to_string()));
        }
        parent_node.children.push(path.to_string());
        self.nodes.insert(
            path.to_string(),
            WidgetNode {
                path: path.to_string(),
                kind,
                parent: Some(parent),
                children: Vec::new(),
                options: BTreeMap::new(),
                pack: None,
                grid: None,
            },
        );
        Ok(path.to_string())
    }

    /// Destroy a widget and its whole subtree.  The root cannot be
    /// destroyed.
    pub fn destroy(&mut self, path: &str) -> Result<(), TkError> {
        if path == self.root {
            return Err(TkError::CannotDestroyRoot);
        }
        let node = self
            .nodes
            .get(path)
            .ok_or_else(|| TkError::NotFound(path.to_string()))?
            .clone();
        if let Some(parent) = &node.parent {
            if let Some(p) = self.nodes.get_mut(parent) {
                p.children.retain(|c| c != path);
            }
        }
        self.remove_subtree(path);
        Ok(())
    }

    fn remove_subtree(&mut self, path: &str) {
        if let Some(node) = self.nodes.remove(path) {
            for child in node.children {
                self.remove_subtree(&child);
            }
        }
    }

    pub fn configure(
        &mut self,
        path: &str,
        options: impl IntoIterator<Item = (String, String)>,
    ) -> Result<(), TkError> {
        let node = self
            .nodes
            .get_mut(path)
            .ok_or_else(|| TkError::NotFound(path.to_string()))?;
        for (name, value) in options {
            if !valid_option(&name) {
                return Err(TkError::BadPath(name));
            }
            node.options.insert(name, value);
        }
        Ok(())
    }

    pub fn cget(&self, path: &str, option: &str) -> Result<Option<&str>, TkError> {
        let node = self
            .nodes
            .get(path)
            .ok_or_else(|| TkError::NotFound(path.to_string()))?;
        if !valid_option(option) {
            return Err(TkError::BadPath(option.to_string()));
        }
        Ok(node.options.get(option).map(String::as_str))
    }

    pub fn pack(&mut self, path: &str, info: PackInfo) -> Result<(), TkError> {
        let node = self
            .nodes
            .get_mut(path)
            .ok_or_else(|| TkError::NotFound(path.to_string()))?;
        node.pack = Some(info);
        node.grid = None;
        Ok(())
    }

    pub fn grid(&mut self, path: &str, info: GridInfo) -> Result<(), TkError> {
        let node = self
            .nodes
            .get_mut(path)
            .ok_or_else(|| TkError::NotFound(path.to_string()))?;
        node.grid = Some(info);
        node.pack = None;
        Ok(())
    }

    pub fn children(&self, path: &str) -> Result<&[String], TkError> {
        self.nodes
            .get(path)
            .map(|n| n.children.as_slice())
            .ok_or_else(|| TkError::NotFound(path.to_string()))
    }
}

/// Tk window path: `.` or `.seg(.seg)*`, segments `[A-Za-z0-9_-]+`.
pub fn valid_path(path: &str) -> bool {
    if path == "." {
        return true;
    }
    let Some(rest) = path.strip_prefix('.') else {
        return false;
    };
    !rest.is_empty()
        && rest.split('.').all(|seg| {
            !seg.is_empty()
                && seg
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
}

pub fn parent_path(path: &str) -> Option<String> {
    if path == "." {
        return None;
    }
    let i = path.rfind('.')?;
    Some(if i == 0 {
        ".".to_string()
    } else {
        path[..i].to_string()
    })
}

pub fn valid_option(name: &str) -> bool {
    name.len() > 1
        && name.starts_with('-')
        && name[1..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_hierarchy_from_root() {
        let mut t = TkTree::new();
        assert_eq!(t.create(".", "f", WidgetKind::Frame).unwrap(), ".f");
        assert_eq!(t.create(".f", "b", WidgetKind::Button).unwrap(), ".f.b");
        assert_eq!(t.children(".").unwrap(), [".f"]);
        assert_eq!(t.children(".f").unwrap(), [".f.b"]);
        assert!(matches!(
            t.create(".missing", "x", WidgetKind::Label),
            Err(TkError::NotFound(_))
        ));
    }

    #[test]
    fn rejects_bad_paths_and_duplicates() {
        let mut t = TkTree::new();
        assert!(matches!(
            t.create(".", "", WidgetKind::Frame),
            Err(TkError::BadPath(_))
        ));
        assert!(matches!(
            t.create(".", "a.b", WidgetKind::Frame),
            Err(TkError::BadPath(_))
        ));
        t.create(".", "f", WidgetKind::Frame).unwrap();
        assert!(matches!(
            t.create(".", "f", WidgetKind::Frame),
            Err(TkError::AlreadyExists(_))
        ));
    }

    #[test]
    fn configure_and_cget_keep_tk_option_spelling() {
        let mut t = TkTree::new();
        t.create(".", "b", WidgetKind::Button).unwrap();
        t.configure(
            ".b",
            [
                ("-text".to_string(), "Hi".to_string()),
                ("-state".to_string(), "normal".to_string()),
            ],
        )
        .unwrap();
        assert_eq!(t.cget(".b", "-text").unwrap(), Some("Hi"));
        assert_eq!(t.cget(".b", "-text").unwrap(), Some("Hi"));
        assert_eq!(t.cget(".b", "-missing").unwrap(), None);
        assert!(matches!(t.cget(".b", "text"), Err(TkError::BadPath(_))));
    }

    #[test]
    fn pack_and_grid_are_mutually_exclusive() {
        let mut t = TkTree::new();
        t.create(".", "f", WidgetKind::Frame).unwrap();
        t.pack(
            ".f",
            PackInfo {
                expand: true,
                ..PackInfo::default()
            },
        )
        .unwrap();
        assert!(t.get(".f").unwrap().grid.is_none());
        t.grid(
            ".f",
            GridInfo {
                row: 2,
                column: 3,
                ..GridInfo::default()
            },
        )
        .unwrap();
        let n = t.get(".f").unwrap();
        assert!(n.pack.is_none());
        assert_eq!(n.grid.unwrap().row, 2);
    }

    #[test]
    fn destroy_removes_subtree_and_parent_link() {
        let mut t = TkTree::new();
        t.create(".", "f", WidgetKind::Frame).unwrap();
        t.create(".f", "b", WidgetKind::Button).unwrap();
        t.create(".f", "l", WidgetKind::Label).unwrap();
        t.destroy(".f").unwrap();
        assert!(!t.contains(".f"));
        assert!(!t.contains(".f.b"));
        assert!(t.children(".").unwrap().is_empty());
        assert!(matches!(t.destroy("."), Err(TkError::CannotDestroyRoot)));
    }

    #[test]
    fn layout_paths() {
        assert!(valid_path("."));
        assert!(valid_path(".a"));
        assert!(valid_path(".a_b-c.d1"));
        assert!(!valid_path(""));
        assert!(!valid_path("a"));
        assert!(!valid_path(".a."));
        assert!(!valid_path(".a..b"));
        assert_eq!(parent_path(".a.b").as_deref(), Some(".a"));
        assert_eq!(parent_path(".a").as_deref(), Some("."));
        assert_eq!(parent_path("."), None);
    }
}
