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

use std::collections::{BTreeMap, BTreeSet, HashMap};

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
    /// `message`: a label that wraps by default.
    Message,
    /// `scrollbar`: a trough and thumb (the renderer shows a static bar).
    Scrollbar,
    /// `ttk::separator`: a one-pixel rule.
    Separator,
    /// `ttk::progressbar`: a determinate progress fill.
    Progressbar,
    /// `ttk::notebook`: tabbed pages (children are the pages).
    Notebook,
    /// `ttk::combobox`: an entry with a value list.
    Combobox,
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
            "message" => Self::Message,
            "scrollbar" => Self::Scrollbar,
            "separator" => Self::Separator,
            "progressbar" => Self::Progressbar,
            "notebook" => Self::Notebook,
            "combobox" => Self::Combobox,
            _ => return None,
        })
    }

    /// The Tk class name (`winfo class`).
    pub fn class_name(self) -> &'static str {
        match self {
            Self::Toplevel => "Toplevel",
            Self::Frame => "Frame",
            Self::Label => "Label",
            Self::Button => "Button",
            Self::Entry => "Entry",
            Self::Checkbutton => "Checkbutton",
            Self::Radiobutton => "Radiobutton",
            Self::Scale => "Scale",
            Self::Text => "Text",
            Self::Listbox => "Listbox",
            Self::Message => "Message",
            Self::Scrollbar => "Scrollbar",
            Self::Separator => "TSeparator",
            Self::Progressbar => "TProgressbar",
            Self::Notebook => "TNotebook",
            Self::Combobox => "TCombobox",
        }
    }

    /// Whether the widget displays `-text` / `-textvariable`.
    pub fn has_text(self) -> bool {
        matches!(
            self,
            Self::Label | Self::Button | Self::Checkbutton | Self::Radiobutton | Self::Message
        )
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

#[derive(Debug, Clone, PartialEq)]
pub struct PackInfo {
    pub side: PackSide,
    pub fill: Fill,
    pub expand: bool,
    pub padx: f64,
    pub pady: f64,
    /// `-anchor`: where the parcel sits in its cavity (`center`, `w`, ...).
    pub anchor: String,
}

impl Default for PackInfo {
    fn default() -> Self {
        Self {
            side: PackSide::Top,
            fill: Fill::None,
            expand: false,
            padx: 0.0,
            pady: 0.0,
            anchor: "center".to_string(),
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

/// `place` geometry: absolute and relative offsets and sizes.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceInfo {
    pub x: f64,
    pub y: f64,
    pub relx: f64,
    pub rely: f64,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub relwidth: Option<f64>,
    pub relheight: Option<f64>,
    pub anchor: String,
}

impl Default for PlaceInfo {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            relx: 0.0,
            rely: 0.0,
            width: None,
            height: None,
            relwidth: None,
            relheight: None,
            anchor: "nw".to_string(),
        }
    }
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
    pub place: Option<PlaceInfo>,
    /// Event bindings (`<Button-1>`) to Tcl scripts.
    pub bindings: BTreeMap<String, String>,
    /// Editable content of Entry/Text (not a Tk option).
    pub content: String,
    /// Items of a Listbox, and its selected indices.
    pub items: Vec<String>,
    pub selected: BTreeSet<usize>,
    /// Value of a Scale.
    pub value: f64,
    /// State of a Checkbutton without a `-variable`.
    pub checked: bool,
    /// Insertion cursor of Entry/Text (a character index).
    pub cursor: usize,
    /// Selected character range of an Entry: `(from, to)`, empty when
    /// equal.
    pub selection: (usize, usize),
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
    /// The widget's parent already has slaves managed by the other
    /// geometry manager.
    AlreadyManaged(String),
}

impl std::fmt::Display for TkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadPath(p) => write!(f, "bad window path name \"{p}\""),
            Self::NotFound(p) => write!(f, "bad window path name \"{p}\""),
            Self::AlreadyExists(p) => write!(f, "window name \"{p}\" already exists"),
            Self::CannotDestroyRoot => write!(f, "cannot destroy \".\""),
            Self::DuplicateChild(p) => write!(f, "window \"{p}\" already exists"),
            Self::AlreadyManaged(p) => write!(
                f,
                "cannot use geometry manager inside \"{p}\" which already has slaves managed by another geometry manager"
            ),
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
                place: None,
                grid: None,
                bindings: BTreeMap::new(),
                content: String::new(),
                items: Vec::new(),
                selected: BTreeSet::new(),
                value: 0.0,
                checked: false,
                cursor: 0,
                selection: (0, 0),
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
                place: None,
                grid: None,
                bindings: BTreeMap::new(),
                content: String::new(),
                items: Vec::new(),
                selected: BTreeSet::new(),
                value: 0.0,
                checked: false,
                cursor: 0,
                selection: (0, 0),
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

    /// Pack `path` in its parent.  A parent's slaves all use one manager,
    /// as in Tk.
    pub fn pack(&mut self, path: &str, info: PackInfo) -> Result<(), TkError> {
        let parent = self
            .nodes
            .get(path)
            .and_then(|n| n.parent.clone())
            .ok_or_else(|| TkError::NotFound(path.to_string()))?;
        let conflict = self
            .nodes
            .get(&parent)
            .map(|p| {
                p.children
                    .iter()
                    .any(|c| c != path && self.nodes.get(c).is_some_and(|n| n.grid.is_some()))
            })
            .unwrap_or(false);
        if conflict {
            return Err(TkError::AlreadyManaged(parent));
        }
        let node = self
            .nodes
            .get_mut(path)
            .ok_or_else(|| TkError::NotFound(path.to_string()))?;
        node.pack = Some(info);
        node.grid = None;
        node.place = None;
        Ok(())
    }

    /// Grid `path` in its parent.
    pub fn grid(&mut self, path: &str, info: GridInfo) -> Result<(), TkError> {
        let parent = self
            .nodes
            .get(path)
            .and_then(|n| n.parent.clone())
            .ok_or_else(|| TkError::NotFound(path.to_string()))?;
        let conflict = self
            .nodes
            .get(&parent)
            .map(|p| {
                p.children
                    .iter()
                    .any(|c| c != path && self.nodes.get(c).is_some_and(|n| n.pack.is_some()))
            })
            .unwrap_or(false);
        if conflict {
            return Err(TkError::AlreadyManaged(parent));
        }
        let node = self
            .nodes
            .get_mut(path)
            .ok_or_else(|| TkError::NotFound(path.to_string()))?;
        node.grid = Some(info);
        node.pack = None;
        node.place = None;
        Ok(())
    }

    /// Place `path` with absolute/relative coordinates.
    pub fn place(&mut self, path: &str, info: PlaceInfo) -> Result<(), TkError> {
        let node = self
            .nodes
            .get_mut(path)
            .ok_or_else(|| TkError::NotFound(path.to_string()))?;
        node.place = Some(info);
        node.pack = None;
        node.grid = None;
        Ok(())
    }

    /// Move `path` to the end of its parent's children (top of the
    /// stacking order, as `raise`).
    pub fn raise(&mut self, path: &str) -> Result<(), TkError> {
        let parent = self
            .nodes
            .get(path)
            .and_then(|n| n.parent.clone())
            .ok_or_else(|| TkError::NotFound(path.to_string()))?;
        if let Some(node) = self.nodes.get_mut(&parent) {
            node.children.retain(|c| c != path);
            node.children.push(path.to_string());
        }
        Ok(())
    }

    /// Move `path` to the start of its parent's children (`lower`).
    pub fn lower(&mut self, path: &str) -> Result<(), TkError> {
        let parent = self
            .nodes
            .get(path)
            .and_then(|n| n.parent.clone())
            .ok_or_else(|| TkError::NotFound(path.to_string()))?;
        if let Some(node) = self.nodes.get_mut(&parent) {
            node.children.retain(|c| c != path);
            node.children.insert(0, path.to_string());
        }
        Ok(())
    }

    /// Move `child` to `index` among `parent`'s children.
    pub fn insert_child_at(
        &mut self,
        parent: &str,
        child: &str,
        index: usize,
    ) -> Result<(), TkError> {
        let node = self
            .nodes
            .get_mut(parent)
            .ok_or_else(|| TkError::NotFound(parent.to_string()))?;
        node.children.retain(|c| c != child);
        let at = index.min(node.children.len());
        node.children.insert(at, child.to_string());
        Ok(())
    }

    /// Stop showing `child` under `parent` (notebook `forget`); the widget
    /// itself stays alive.
    pub fn detach(&mut self, parent: &str, child: &str) -> Result<(), TkError> {
        let node = self
            .nodes
            .get_mut(parent)
            .ok_or_else(|| TkError::NotFound(parent.to_string()))?;
        node.children.retain(|c| c != child);
        Ok(())
    }

    /// All widgets below `path`, depth first (for focus traversal).
    pub fn descendants(&self, path: &str) -> Vec<String> {
        let mut out = Vec::new();
        fn walk(tree: &TkTree, path: &str, out: &mut Vec<String>) {
            if let Some(n) = tree.get(path) {
                for c in &n.children {
                    out.push(c.clone());
                    walk(tree, c, out);
                }
            }
        }
        walk(self, path, &mut out);
        out
    }

    /// `place forget`.
    pub fn unplace(&mut self, path: &str) -> Result<(), TkError> {
        let node = self
            .nodes
            .get_mut(path)
            .ok_or_else(|| TkError::NotFound(path.to_string()))?;
        node.place = None;
        Ok(())
    }

    /// `pack forget`: stop managing `path`, keeping its other options.
    pub fn unpack(&mut self, path: &str) -> Result<(), TkError> {
        let node = self
            .nodes
            .get_mut(path)
            .ok_or_else(|| TkError::NotFound(path.to_string()))?;
        node.pack = None;
        Ok(())
    }

    /// `grid forget` / `grid remove`.
    pub fn ungrid(&mut self, path: &str) -> Result<(), TkError> {
        let node = self
            .nodes
            .get_mut(path)
            .ok_or_else(|| TkError::NotFound(path.to_string()))?;
        node.grid = None;
        Ok(())
    }

    /// The parent's children managed by `pack` or by `grid`.
    pub fn slaves(&self, path: &str, manager: &str) -> Result<Vec<String>, TkError> {
        let node = self
            .nodes
            .get(path)
            .ok_or_else(|| TkError::NotFound(path.to_string()))?;
        Ok(node
            .children
            .iter()
            .filter(|c| {
                self.nodes.get(*c).is_some_and(|n| match manager {
                    "pack" => n.pack.is_some(),
                    "grid" => n.grid.is_some(),
                    "place" => n.place.is_some(),
                    _ => false,
                })
            })
            .cloned()
            .collect())
    }

    pub fn children(&self, path: &str) -> Result<&[String], TkError> {
        self.nodes
            .get(path)
            .map(|n| n.children.as_slice())
            .ok_or_else(|| TkError::NotFound(path.to_string()))
    }

    /// Set (or clear, with an empty script) a binding on `path`.  `"all"`
    /// and class bindings are not part of the v1 model.
    pub fn bind(&mut self, path: &str, sequence: &str, script: &str) -> Result<(), TkError> {
        let node = self
            .nodes
            .get_mut(path)
            .ok_or_else(|| TkError::NotFound(path.to_string()))?;
        if script.is_empty() {
            node.bindings.remove(sequence);
        } else {
            node.bindings
                .insert(sequence.to_string(), script.to_string());
        }
        Ok(())
    }

    pub fn binding(&self, path: &str, sequence: &str) -> Result<Option<&str>, TkError> {
        let node = self
            .nodes
            .get(path)
            .ok_or_else(|| TkError::NotFound(path.to_string()))?;
        Ok(node.bindings.get(sequence).map(String::as_str))
    }

    pub fn binding_sequences(&self, path: &str) -> Result<Vec<String>, TkError> {
        let node = self
            .nodes
            .get(path)
            .ok_or_else(|| TkError::NotFound(path.to_string()))?;
        Ok(node.bindings.keys().cloned().collect())
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

const TOPLEVEL_OPTIONS: &[&str] = &[
    "-background",
    "-borderwidth",
    "-class",
    "-colormap",
    "-container",
    "-cursor",
    "-height",
    "-highlightbackground",
    "-highlightcolor",
    "-highlightthickness",
    "-menu",
    "-padx",
    "-pady",
    "-relief",
    "-screen",
    "-takefocus",
    "-use",
    "-visual",
    "-width",
    // `wm` pseudo-options kept on the root node.
    "-title",
    "-geometry",
    "-resizable",
    "-minsize",
    "-maxsize",
    "-state",
    "-withdrawn",
    "-iconify",
    "-protocol",
];

const FRAME_OPTIONS: &[&str] = &[
    "-background",
    "-borderwidth",
    "-class",
    "-colormap",
    "-container",
    "-cursor",
    "-height",
    "-highlightbackground",
    "-highlightcolor",
    "-highlightthickness",
    "-padx",
    "-pady",
    "-padding",
    "-relief",
    "-state",
    "-style",
    "-takefocus",
    "-text",
    "-visual",
    "-width",
];

const LABEL_OPTIONS: &[&str] = &[
    "-anchor",
    "-background",
    "-bitmap",
    "-borderwidth",
    "-compound",
    "-cursor",
    "-font",
    "-foreground",
    "-height",
    "-highlightbackground",
    "-highlightcolor",
    "-highlightthickness",
    "-image",
    "-justify",
    "-padx",
    "-pady",
    "-relief",
    "-state",
    "-takefocus",
    "-text",
    "-textvariable",
    "-underline",
    "-width",
    "-wraplength",
];

const BUTTON_OPTIONS: &[&str] = &[
    "-anchor",
    "-background",
    "-bitmap",
    "-borderwidth",
    "-command",
    "-compound",
    "-cursor",
    "-default",
    "-font",
    "-foreground",
    "-height",
    "-highlightbackground",
    "-highlightcolor",
    "-highlightthickness",
    "-image",
    "-justify",
    "-overrelief",
    "-padx",
    "-pady",
    "-relief",
    "-state",
    "-takefocus",
    "-text",
    "-textvariable",
    "-underline",
    "-width",
    "-wraplength",
];

const ENTRY_OPTIONS: &[&str] = &[
    "-background",
    "-borderwidth",
    "-cursor",
    "-exportselection",
    "-font",
    "-foreground",
    "-highlightbackground",
    "-highlightcolor",
    "-highlightthickness",
    "-invalidcommand",
    "-justify",
    "-padx",
    "-pady",
    "-relief",
    "-show",
    "-state",
    "-takefocus",
    "-textvariable",
    "-validate",
    "-validatecommand",
    "-width",
    "-xscrollcommand",
];

const CHECKBUTTON_OPTIONS: &[&str] = &[
    "-anchor",
    "-background",
    "-bitmap",
    "-borderwidth",
    "-command",
    "-compound",
    "-cursor",
    "-font",
    "-foreground",
    "-height",
    "-highlightbackground",
    "-highlightcolor",
    "-highlightthickness",
    "-image",
    "-indicatoron",
    "-justify",
    "-offvalue",
    "-onvalue",
    "-padx",
    "-pady",
    "-relief",
    "-selectcolor",
    "-selectimage",
    "-state",
    "-takefocus",
    "-text",
    "-textvariable",
    "-underline",
    "-variable",
    "-width",
    "-wraplength",
];

const RADIOBUTTON_OPTIONS: &[&str] = &[
    "-anchor",
    "-background",
    "-bitmap",
    "-borderwidth",
    "-command",
    "-compound",
    "-cursor",
    "-font",
    "-foreground",
    "-height",
    "-highlightbackground",
    "-highlightcolor",
    "-highlightthickness",
    "-image",
    "-indicatoron",
    "-justify",
    "-padx",
    "-pady",
    "-relief",
    "-selectcolor",
    "-selectimage",
    "-state",
    "-takefocus",
    "-text",
    "-textvariable",
    "-underline",
    "-value",
    "-variable",
    "-width",
    "-wraplength",
];

const SCALE_OPTIONS: &[&str] = &[
    "-activebackground",
    "-background",
    "-bigincrement",
    "-borderwidth",
    "-command",
    "-cursor",
    "-digits",
    "-font",
    "-foreground",
    "-from",
    "-highlightbackground",
    "-highlightcolor",
    "-highlightthickness",
    "-label",
    "-length",
    "-orient",
    "-relief",
    "-resolution",
    "-showvalue",
    "-sliderlength",
    "-sliderrelief",
    "-state",
    "-takefocus",
    "-tickinterval",
    "-to",
    "-troughcolor",
    "-variable",
    "-width",
];

const TEXT_OPTIONS: &[&str] = &[
    "-background",
    "-borderwidth",
    "-cursor",
    "-font",
    "-foreground",
    "-height",
    "-highlightbackground",
    "-highlightcolor",
    "-highlightthickness",
    "-insertbackground",
    "-maxundo",
    "-padx",
    "-pady",
    "-relief",
    "-selectbackground",
    "-spacing1",
    "-spacing2",
    "-spacing3",
    "-state",
    "-tabs",
    "-takefocus",
    "-undo",
    "-width",
    "-wrap",
    "-xscrollcommand",
    "-yscrollcommand",
];

const LISTBOX_OPTIONS: &[&str] = &[
    "-activestyle",
    "-background",
    "-borderwidth",
    "-cursor",
    "-exportselection",
    "-font",
    "-foreground",
    "-height",
    "-highlightbackground",
    "-highlightcolor",
    "-highlightthickness",
    "-listvariable",
    "-relief",
    "-selectbackground",
    "-selectborderwidth",
    "-selectforeground",
    "-selectmode",
    "-setgrid",
    "-state",
    "-takefocus",
    "-width",
    "-xscrollcommand",
    "-yscrollcommand",
];

const MESSAGE_OPTIONS: &[&str] = &[
    "-anchor",
    "-aspect",
    "-background",
    "-borderwidth",
    "-cursor",
    "-font",
    "-foreground",
    "-highlightbackground",
    "-highlightcolor",
    "-highlightthickness",
    "-justify",
    "-padx",
    "-pady",
    "-relief",
    "-takefocus",
    "-text",
    "-textvariable",
    "-width",
];

const SCROLLBAR_OPTIONS: &[&str] = &[
    "-activebackground",
    "-activerelief",
    "-background",
    "-borderwidth",
    "-command",
    "-cursor",
    "-elementborderwidth",
    "-highlightbackground",
    "-highlightcolor",
    "-highlightthickness",
    "-jump",
    "-orient",
    "-relief",
    "-repeatdelay",
    "-repeatinterval",
    "-takefocus",
    "-troughcolor",
    "-width",
];

const SEPARATOR_OPTIONS: &[&str] = &["-cursor", "-orient", "-takefocus"];

const PROGRESSBAR_OPTIONS: &[&str] = &[
    "-cursor",
    "-length",
    "-maximum",
    "-mode",
    "-orient",
    "-phase",
    "-takefocus",
    "-value",
    "-variable",
];

const NOTEBOOK_OPTIONS: &[&str] = &[
    "-cursor",
    "-height",
    "-padding",
    "-style",
    "-takefocus",
    "-width",
];

const COMBOBOX_OPTIONS: &[&str] = &[
    "-background",
    "-cursor",
    "-exportselection",
    "-font",
    "-foreground",
    "-height",
    "-invalidcommand",
    "-justify",
    "-postcommand",
    "-state",
    "-style",
    "-takefocus",
    "-textvariable",
    "-validate",
    "-validatecommand",
    "-values",
    "-width",
];

/// Options Tk 8.6 accepts for `kind` (including the ones kept on the node
/// as `wm` pseudo-options for toplevels).
pub fn known_options(kind: WidgetKind) -> &'static [&'static str] {
    match kind {
        WidgetKind::Toplevel => TOPLEVEL_OPTIONS,
        WidgetKind::Frame => FRAME_OPTIONS,
        WidgetKind::Label => LABEL_OPTIONS,
        WidgetKind::Button => BUTTON_OPTIONS,
        WidgetKind::Entry => ENTRY_OPTIONS,
        WidgetKind::Checkbutton => CHECKBUTTON_OPTIONS,
        WidgetKind::Radiobutton => RADIOBUTTON_OPTIONS,
        WidgetKind::Scale => SCALE_OPTIONS,
        WidgetKind::Text => TEXT_OPTIONS,
        WidgetKind::Listbox => LISTBOX_OPTIONS,
        WidgetKind::Message => MESSAGE_OPTIONS,
        WidgetKind::Scrollbar => SCROLLBAR_OPTIONS,
        WidgetKind::Separator => SEPARATOR_OPTIONS,
        WidgetKind::Progressbar => PROGRESSBAR_OPTIONS,
        WidgetKind::Notebook => NOTEBOOK_OPTIONS,
        WidgetKind::Combobox => COMBOBOX_OPTIONS,
    }
}

/// The value Tk reports for an option a script never configured.
pub fn option_default(kind: WidgetKind, name: &str) -> Option<&'static str> {
    Some(match name {
        "-background" => "#d9d9d9",
        "-foreground" => "#000000",
        "-highlightbackground" => "#d9d9d9",
        "-highlightcolor" => "#000000",
        "-font" => "TkDefaultFont",
        "-relief" => "flat",
        "-borderwidth" | "-highlightthickness" => "0",
        "-cursor" | "-takefocus" => "",
        "-padx" | "-pady" => "0",
        "-width" => match kind {
            WidgetKind::Entry => "20",
            WidgetKind::Text => "80",
            WidgetKind::Listbox => "20",
            WidgetKind::Scale => "100",
            WidgetKind::Combobox => "20",
            _ => "0",
        },
        "-height" => match kind {
            WidgetKind::Text => "24",
            WidgetKind::Listbox => "10",
            WidgetKind::Combobox => "10",
            _ => "0",
        },
        "-text" | "-label" | "-textvariable" | "-variable" | "-command" => "",
        "-anchor" => "center",
        "-justify" => match kind {
            WidgetKind::Button | WidgetKind::Checkbutton | WidgetKind::Radiobutton => "center",
            _ => "left",
        },
        "-wraplength" | "-overrelief" | "-image" | "-bitmap" | "-show" => "",
        "-compound" => "none",
        "-underline" => "-1",
        "-state" => "normal",
        "-default" => "disabled",
        "-onvalue" => "1",
        "-offvalue" => "0",
        "-value" => "",
        "-from" => "0",
        "-to" => "100",
        "-resolution" => "1",
        "-orient" => "vertical",
        "-showvalue" => "1",
        "-digits" => "0",
        "-tickinterval" => "0",
        "-sliderlength" => "30",
        "-sliderrelief" => "raised",
        "-bigincrement" => "0",
        "-length" => "100",
        "-troughcolor" | "-activebackground" => "#d9d9d9",
        "-activerelief" => "raised",
        "-elementborderwidth" => "1",
        "-jump" | "-repeatdelay" | "-repeatinterval" => "0",
        "-aspect" => "150",
        "-maximum" => "100",
        "-mode" => "determinate",
        "-phase" => "0",
        "-validate" => "none",
        "-validatecommand" | "-invalidcommand" | "-xscrollcommand" | "-yscrollcommand"
        | "-values" | "-postcommand" | "-style" => "",
        "-padding" => "0",
        "-exportselection" => "1",
        "-wrap" => "char",
        "-undo" | "-maxundo" | "-spacing1" | "-spacing2" | "-spacing3" | "-tabs" => "0",
        "-insertbackground" | "-selectbackground" | "-selectforeground" => "",
        "-selectmode" => "browse",
        "-activestyle" => "underline",
        "-listvariable" | "-selectborderwidth" | "-setgrid" => "",
        "-indicatoron" => "1",
        "-selectcolor" => "#b03060",
        "-selectimage" => "",
        "-class" => "Frame",
        "-colormap" | "-container" | "-screen" | "-use" | "-visual" | "-menu" => "",
        "-title" | "-geometry" | "-maxsize" => "",
        "-resizable" => "1 1",
        "-minsize" => "0 0",
        "-withdrawn" | "-iconify" => "0",
        "-protocol" => "",
        _ => return None,
    })
}

/// How an option abbreviation resolves against a widget's known options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionLookup {
    Found(String),
    Ambiguous(Vec<String>),
    Unknown,
}

/// Resolve Tk's unique-prefix option abbreviations (`-tex` → `-text`).
/// `stored` are the keys already set on the node (all canonical).
pub fn lookup_option<'a>(
    kind: WidgetKind,
    name: &str,
    stored: impl IntoIterator<Item = &'a str>,
) -> OptionLookup {
    let mut all: Vec<String> = known_options(kind)
        .iter()
        .copied()
        .chain(stored)
        .map(|s| s.to_string())
        .collect();
    if let Some(exact) = all.iter().find(|o| o.as_str() == name) {
        return OptionLookup::Found(exact.clone());
    }
    all.sort();
    all.dedup();
    let matches: Vec<String> = all.into_iter().filter(|o| o.starts_with(name)).collect();
    match matches.len() {
        0 => OptionLookup::Unknown,
        1 => OptionLookup::Found(matches.into_iter().next().unwrap()),
        _ => OptionLookup::Ambiguous(matches),
    }
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
