//! AccessKit projection of settled kernel scenes and action routing.
//!
//! The OS adapter is deliberately window-only. Offscreen rendering continues to
//! consume kernel frames without constructing an accessibility adapter.

use std::collections::{BTreeMap, HashMap};

use accesskit::{
    Action, ActionData, ActionRequest, Affine, Live, Node, NodeId, Orientation, Rect, Role,
    ScrollUnit, Toggled, Tree, TreeId, TreeUpdate,
};
use accesskit_winit::{Adapter, Event as AdapterEvent, WindowEvent as AdapterWindowEvent};
use slab_kernel::dispatch::{self as kdispatch, Event as KernelEvent};
use slab_kernel::flatten::{Frame, SceneNode};
use slab_kernel::frame::{self as kframe, Instance};
use slab_kernel::{scene, slir};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::window::Window;

/// Event-loop event emitted by the AccessKit winit adapter.
pub type Event = AdapterEvent;
/// Window adapter event carried by [`Event`].
pub type EventKind = AdapterWindowEvent;

const ROOT_ID: NodeId = NodeId(0);
const ITEM_SCROLL: f64 = 40.0;

/// One retained scene mounted in a native window.
pub struct SceneLayer<'a> {
    document: usize,
    instance: &'a Instance,
    frame: &'a Frame,
    offset_x: f64,
    offset_y: f64,
    mount: Option<MountPoint>,
}

impl<'a> SceneLayer<'a> {
    /// Creates one accessibility layer from a settled kernel frame.
    pub fn new(document: usize, instance: &'a Instance, frame: &'a Frame) -> Self {
        Self {
            document,
            instance,
            frame,
            offset_x: 0.0,
            offset_y: 0.0,
            mount: None,
        }
    }

    /// Translates this layer inside the host window.
    pub fn translated(mut self, x: f64, y: f64) -> Self {
        self.offset_x = x;
        self.offset_y = y;
        self
    }

    /// Mounts this layer below a node in another document layer.
    pub fn mounted(mut self, document: usize, node: u32) -> Self {
        self.mount = Some(MountPoint { document, node });
        self
    }
}

#[derive(Clone, Copy)]
pub(crate) struct MountPoint {
    document: usize,
    node: u32,
}

/// An action already validated against the latest published scene.
#[derive(Clone, Debug, PartialEq)]
pub struct RoutedAction {
    /// Host document identifier supplied through [`SceneLayer::new`].
    pub document: usize,
    /// Stable Slab key that receives the action.
    pub key: String,
    kind: RoutedActionKind,
}

#[derive(Clone, Debug, PartialEq)]
enum RoutedActionKind {
    Focus,
    Default,
    DividerKey(&'static str),
    Reveal,
    ScrollBy {
        axis: u32,
        delta: f64,
    },
    SetScroll {
        main: Option<f64>,
        cross: Option<f64>,
    },
}

/// Result of applying the direct part of a routed action.
pub enum ActionResult {
    /// The target did not accept the action.
    Ignored,
    /// The action changed retained kernel state directly.
    Changed,
    /// Dispatch this event through the host wrapper to retain typed signals.
    Dispatch(KernelEvent),
}

impl RoutedAction {
    /// Whether this action moves keyboard focus between mounted documents.
    pub fn moves_focus(&self) -> bool {
        matches!(
            self.kind,
            RoutedActionKind::Focus | RoutedActionKind::Default | RoutedActionKind::DividerKey(_)
        )
    }

    /// Applies this action to its target kernel instance.
    pub fn apply(&self, instance: &mut Instance) -> ActionResult {
        match self.kind {
            RoutedActionKind::Focus => {
                if kframe::inst_set_focus(instance, &self.key, true) {
                    // A successful no-op still matters to a multi-document host:
                    // it must clear focus from every sibling scene.
                    ActionResult::Changed
                } else {
                    ActionResult::Ignored
                }
            }
            RoutedActionKind::Default => {
                if !kframe::inst_set_focus(instance, &self.key, true) {
                    return ActionResult::Ignored;
                }
                ActionResult::Dispatch(key_event("Enter"))
            }
            RoutedActionKind::DividerKey(key) => {
                if !kframe::inst_set_focus(instance, &self.key, true) {
                    return ActionResult::Ignored;
                }
                ActionResult::Dispatch(key_event(key))
            }
            RoutedActionKind::Reveal => {
                if kframe::inst_reveal(instance, &self.key, 0.0) {
                    ActionResult::Changed
                } else {
                    ActionResult::Ignored
                }
            }
            RoutedActionKind::ScrollBy { axis, delta } => {
                let old = kframe::inst_get_scroll(instance, &self.key, axis);
                if !kframe::inst_set_scroll(instance, &self.key, axis, old + delta) {
                    return ActionResult::Ignored;
                }
                if kframe::inst_get_scroll(instance, &self.key, axis) == old {
                    ActionResult::Ignored
                } else {
                    ActionResult::Changed
                }
            }
            RoutedActionKind::SetScroll { main, cross } => {
                let mut changed = false;
                for (axis, requested) in [(0, main), (1, cross)] {
                    let Some(requested) = requested else {
                        continue;
                    };
                    let old = kframe::inst_get_scroll(instance, &self.key, axis);
                    if kframe::inst_set_scroll(instance, &self.key, axis, requested)
                        && kframe::inst_get_scroll(instance, &self.key, axis) != old
                    {
                        changed = true;
                    }
                }
                if changed {
                    ActionResult::Changed
                } else {
                    ActionResult::Ignored
                }
            }
        }
    }
}

fn key_event(key: &str) -> KernelEvent {
    KernelEvent {
        etype: kdispatch::E_KEY_DOWN,
        x: -1.0,
        y: -1.0,
        dx: 0.0,
        dy: 0.0,
        button: 0,
        clicks: 0,
        key: key.to_owned(),
        text: String::new(),
        mods: 0,
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
enum Identity {
    Key { document: usize, key: String },
    Node { document: usize, node: u32 },
}

#[derive(Clone, Debug)]
struct RouteTarget {
    document: usize,
    key: String,
    divider_row: Option<bool>,
    scroll: ScrollTarget,
}

#[derive(Clone, Copy, Debug, Default)]
struct ScrollTarget {
    is_row: bool,
    main: bool,
    cross: bool,
    main_viewport: f64,
    cross_viewport: f64,
}

struct Snapshot {
    valid: bool,
    nodes: BTreeMap<NodeId, Node>,
    routes: HashMap<NodeId, RouteTarget>,
    focus: NodeId,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            valid: false,
            nodes: BTreeMap::new(),
            routes: HashMap::new(),
            focus: ROOT_ID,
        }
    }
}

/// Pure scene-to-AccessKit state. It is tested without constructing an OS adapter.
pub struct Bridge {
    ids: HashMap<Identity, NodeId>,
    next_id: u64,
    current: Snapshot,
    published: Snapshot,
}

impl Default for Bridge {
    fn default() -> Self {
        Self {
            ids: HashMap::new(),
            next_id: 1,
            current: Snapshot::default(),
            published: Snapshot::default(),
        }
    }
}

impl Bridge {
    fn id_for(&mut self, identity: Identity) -> NodeId {
        if let Some(id) = self.ids.get(&identity) {
            return *id;
        }
        let id = NodeId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("native accessibility node ID space exhausted");
        self.ids.insert(identity, id);
        id
    }

    /// Projects the latest settled scene layers into an unpublished tree.
    pub fn refresh(
        &mut self,
        title: &str,
        width: f64,
        height: f64,
        scale: f64,
        layers: &[SceneLayer<'_>],
    ) {
        let mut ids_by_layer = Vec::with_capacity(layers.len());
        let mut keys_by_layer = Vec::with_capacity(layers.len());
        let mut ids_by_scene_node = HashMap::new();

        for layer in layers {
            let mut scene_ids = Vec::with_capacity(layer.frame.scene.len());
            let mut key_ids = HashMap::new();
            for source in &layer.frame.scene {
                let key = scene::key_of(&layer.instance.doc, &layer.instance.st.lists, source.node);
                let identity = if key.is_empty() {
                    Identity::Node {
                        document: layer.document,
                        node: source.node,
                    }
                } else {
                    Identity::Key {
                        document: layer.document,
                        key: key.clone(),
                    }
                };
                let id = self.id_for(identity);
                scene_ids.push(id);
                ids_by_scene_node.insert((layer.document, source.node), id);
                if !key.is_empty() {
                    key_ids.entry(key).or_insert(id);
                }
            }
            ids_by_layer.push(scene_ids);
            keys_by_layer.push(key_ids);
        }

        let mut children: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        let mut root_children = Vec::new();
        for (layer_index, layer) in layers.iter().enumerate() {
            for (scene_index, source) in layer.frame.scene.iter().enumerate() {
                let id = ids_by_layer[layer_index][scene_index];
                if let Ok(parent_index) = usize::try_from(source.parent_ix)
                    && let Some(parent) = ids_by_layer[layer_index].get(parent_index)
                {
                    children.entry(*parent).or_default().push(id);
                } else if let Some(mount) = layer.mount
                    && let Some(parent) = ids_by_scene_node.get(&(mount.document, mount.node))
                {
                    children.entry(*parent).or_default().push(id);
                } else {
                    root_children.push(id);
                }
            }
        }

        let mut nodes = BTreeMap::new();
        let mut routes = HashMap::new();
        let mut focus = ROOT_ID;
        let mut found_focus = false;

        let mut root = Node::new(Role::Window);
        root.set_label(title);
        root.set_bounds(Rect {
            x0: 0.0,
            y0: 0.0,
            x1: width.max(0.0),
            y1: height.max(0.0),
        });
        root.set_transform(Affine::scale(if scale.is_finite() {
            scale.max(f64::EPSILON)
        } else {
            1.0
        }));
        root.set_children(root_children);
        nodes.insert(ROOT_ID, root);

        for (layer_index, layer) in layers.iter().enumerate() {
            for (scene_index, source) in layer.frame.scene.iter().enumerate() {
                let id = ids_by_layer[layer_index][scene_index];
                let role_name = scene_string(layer.instance, source.role);
                let key = scene::key_of(&layer.instance.doc, &layer.instance.st.lists, source.node);
                let inert = source.flags & slir::F_INERT != 0;
                let enabled = !inert && !source.disabled;
                let focusable = enabled && source.flags & slir::F_FOCUSABLE != 0;
                let activates = focusable
                    && kdispatch::sig_of(
                        &layer.instance.doc,
                        &layer.instance.st,
                        source.node,
                        kdispatch::TR_ACTIVATE,
                    ) >= 0;
                let mut role = role_from_name(role_name, source.kind, source.flags);
                if role_name.is_empty()
                    && role == Role::GenericContainer
                    && activates
                    && !key.is_empty()
                {
                    role = Role::Button;
                }
                let mut node = Node::new(role);
                let bounds = transformed_bounds(
                    &layer.frame.scene,
                    scene_index,
                    layer.offset_x,
                    layer.offset_y,
                );
                apply_properties(&mut node, layer.instance, source, bounds);
                if let Some(node_children) = children.remove(&id) {
                    node.set_children(node_children);
                }

                if !key.is_empty() {
                    node.set_author_id(key.clone());
                    if focusable {
                        node.add_action(Action::Focus);
                    }
                    if activates {
                        node.add_action(Action::Click);
                    }
                    let divider_row = if focusable && source.kind == slir::K_DIVIDER {
                        let row = usize::try_from(source.parent_ix)
                            .ok()
                            .and_then(|parent| layer.frame.scene.get(parent))
                            .is_some_and(|parent| parent.is_row);
                        node.set_orientation(if row {
                            Orientation::Vertical
                        } else {
                            Orientation::Horizontal
                        });
                        node.add_action(Action::Decrement);
                        node.add_action(Action::Increment);
                        Some(row)
                    } else {
                        None
                    };
                    let scroll = apply_scroll_properties(&mut node, source, enabled);
                    node.add_action(Action::ScrollIntoView);
                    routes.insert(
                        id,
                        RouteTarget {
                            document: layer.document,
                            key,
                            divider_row,
                            scroll,
                        },
                    );
                } else {
                    let _ = apply_scroll_properties(&mut node, source, false);
                }

                let key_ids = &keys_by_layer[layer_index];
                if let Some(target) =
                    key_ids.get(scene_string(layer.instance, source.active_descendant))
                {
                    node.set_active_descendant(*target);
                }
                if let Some(target) = key_ids.get(scene_string(layer.instance, source.controls)) {
                    node.set_controls(vec![*target]);
                }
                if source.focused && focusable && !found_focus {
                    focus = id;
                    found_focus = true;
                }
                nodes.insert(id, node);
            }
        }

        self.current = Snapshot {
            valid: true,
            nodes,
            routes,
            focus,
        };
    }

    fn source(&self) -> Option<&Snapshot> {
        if self.current.valid {
            Some(&self.current)
        } else if self.published.valid {
            Some(&self.published)
        } else {
            None
        }
    }

    /// Builds a full or changed-node update without publishing it.
    pub fn prepare_update(&self, force_full: bool) -> Option<TreeUpdate> {
        let source = self.source()?;
        let full = force_full || !self.published.valid;
        let nodes = if full {
            source
                .nodes
                .iter()
                .map(|(id, node)| (*id, node.clone()))
                .collect()
        } else {
            source
                .nodes
                .iter()
                .filter(|(id, node)| self.published.nodes.get(id) != Some(*node))
                .map(|(id, node)| (*id, node.clone()))
                .collect::<Vec<_>>()
        };
        if !full && nodes.is_empty() && source.focus == self.published.focus {
            return None;
        }
        let tree = full.then(|| {
            let mut tree = Tree::new(ROOT_ID);
            tree.toolkit_name = Some("Slab".to_owned());
            tree.toolkit_version = Some(env!("CARGO_PKG_VERSION").to_owned());
            tree
        });
        Some(TreeUpdate {
            nodes,
            tree,
            tree_id: TreeId::ROOT,
            focus: source.focus,
        })
    }

    /// Marks the pending tree as published after the host applies its update.
    pub fn commit(&mut self) {
        if self.current.valid {
            self.published = std::mem::take(&mut self.current);
        }
    }

    /// Resolves an AccessKit request against the latest projected tree.
    pub fn resolve_action(&self, request: &ActionRequest) -> Option<RoutedAction> {
        if request.target_tree != TreeId::ROOT {
            return None;
        }
        let source = self.source()?;
        let node = source.nodes.get(&request.target_node)?;
        if !node.supports_action(request.action) {
            return None;
        }
        let target = source.routes.get(&request.target_node)?;
        let kind = match request.action {
            Action::Focus => RoutedActionKind::Focus,
            Action::Click => RoutedActionKind::Default,
            Action::Decrement => RoutedActionKind::DividerKey(if target.divider_row? {
                "ArrowLeft"
            } else {
                "ArrowUp"
            }),
            Action::Increment => RoutedActionKind::DividerKey(if target.divider_row? {
                "ArrowRight"
            } else {
                "ArrowDown"
            }),
            Action::ScrollIntoView => RoutedActionKind::Reveal,
            Action::ScrollUp => scroll_action(target.scroll, false, -1.0, request.data.as_ref())?,
            Action::ScrollDown => scroll_action(target.scroll, false, 1.0, request.data.as_ref())?,
            Action::ScrollLeft => scroll_action(target.scroll, true, -1.0, request.data.as_ref())?,
            Action::ScrollRight => scroll_action(target.scroll, true, 1.0, request.data.as_ref())?,
            Action::SetScrollOffset => {
                let Some(ActionData::SetScrollOffset(point)) = request.data.as_ref() else {
                    return None;
                };
                let (main, cross) = if target.scroll.is_row {
                    (
                        target.scroll.main.then_some(point.x),
                        target.scroll.cross.then_some(point.y),
                    )
                } else {
                    (
                        target.scroll.main.then_some(point.y),
                        target.scroll.cross.then_some(point.x),
                    )
                };
                RoutedActionKind::SetScroll { main, cross }
            }
            _ => return None,
        };
        Some(RoutedAction {
            document: target.document,
            key: target.key.clone(),
            kind,
        })
    }
}

fn scroll_action(
    target: ScrollTarget,
    horizontal: bool,
    direction: f64,
    data: Option<&ActionData>,
) -> Option<RoutedActionKind> {
    let main_is_horizontal = target.is_row;
    let (axis, viewport) = if target.main && main_is_horizontal == horizontal {
        (0, target.main_viewport)
    } else if target.cross && main_is_horizontal != horizontal {
        (1, target.cross_viewport)
    } else {
        return None;
    };
    let amount = if matches!(data, Some(ActionData::ScrollUnit(ScrollUnit::Page))) {
        viewport.max(0.0)
    } else {
        ITEM_SCROLL
    };
    Some(RoutedActionKind::ScrollBy {
        axis,
        delta: direction * amount,
    })
}

fn apply_properties(node: &mut Node, instance: &Instance, source: &SceneNode, bounds: Rect) {
    node.set_bounds(bounds);
    if source.flags & slir::F_INERT != 0 {
        node.set_hidden();
    }
    if source.disabled {
        node.set_disabled();
    }
    let label = scene_string(instance, source.label);
    if !label.is_empty() {
        node.set_label(label);
    }
    let description = scene_string(instance, source.desc);
    if !description.is_empty() {
        node.set_description(description);
    }
    match source.checked {
        1 => node.set_toggled(Toggled::False),
        2 => node.set_toggled(Toggled::True),
        3 => node.set_toggled(Toggled::Mixed),
        _ => {}
    }
    match source.expanded {
        1 => node.set_expanded(false),
        2 => node.set_expanded(true),
        _ => {}
    }
    match source.selected {
        1 => node.set_selected(false),
        2 => node.set_selected(true),
        _ => {}
    }
    if let Some(value) = source.value_now.filter(|value| value.is_finite()) {
        node.set_numeric_value(value);
    }
    if let Some(value) = source.value_min.filter(|value| value.is_finite()) {
        node.set_min_numeric_value(value);
    }
    if let Some(value) = source.value_max.filter(|value| value.is_finite()) {
        node.set_max_numeric_value(value);
    }
    let value_text = scene_string(instance, source.value_text);
    if !value_text.is_empty() {
        node.set_value(value_text);
    }
    if source.modal == 2 {
        node.set_modal();
    }
    match source.live {
        1 => node.set_live(Live::Off),
        2 => node.set_live(Live::Polite),
        3 => node.set_live(Live::Assertive),
        _ => {}
    }
    if source.live_atomic == 2 {
        node.set_live_atomic();
    }
    if let Some(level) = positive_usize(source.level) {
        node.set_level(level);
    }
    if let Some(position) = positive_usize(source.pos_in_set) {
        node.set_position_in_set(position);
    }
    if let Some(size) = positive_usize(source.set_size) {
        node.set_size_of_set(size);
    }
}

fn positive_usize(value: Option<f64>) -> Option<usize> {
    let value = value?;
    if !value.is_finite() || value < 1.0 || value.fract() != 0.0 || value > usize::MAX as f64 {
        return None;
    }
    Some(value as usize)
}

fn apply_scroll_properties(node: &mut Node, source: &SceneNode, enabled: bool) -> ScrollTarget {
    let main = source.flags & slir::F_SCROLL != 0;
    let cross = source.flags & slir::F_SCROLL_CROSS != 0;
    let main_viewport = if source.is_row { source.w } else { source.h }.max(0.0);
    let cross_viewport = if source.is_row { source.h } else { source.w }.max(0.0);
    let main_max = (source.content_main - main_viewport).max(0.0);
    let cross_max = (source.content_cross - cross_viewport).max(0.0);

    let (x, x_max, y, y_max) = if source.is_row {
        (
            main.then_some(source.scroll_off),
            main.then_some(main_max),
            cross.then_some(source.scroll_cross),
            cross.then_some(cross_max),
        )
    } else {
        (
            cross.then_some(source.scroll_cross),
            cross.then_some(cross_max),
            main.then_some(source.scroll_off),
            main.then_some(main_max),
        )
    };
    if let (Some(value), Some(max)) = (x, x_max) {
        node.set_scroll_x(value);
        node.set_scroll_x_min(0.0);
        node.set_scroll_x_max(max);
        if enabled && max > 0.0 {
            node.add_action(Action::ScrollLeft);
            node.add_action(Action::ScrollRight);
        }
    }
    if let (Some(value), Some(max)) = (y, y_max) {
        node.set_scroll_y(value);
        node.set_scroll_y_min(0.0);
        node.set_scroll_y_max(max);
        if enabled && max > 0.0 {
            node.add_action(Action::ScrollUp);
            node.add_action(Action::ScrollDown);
        }
    }
    if enabled && (main || cross) {
        node.add_action(Action::SetScrollOffset);
    }
    ScrollTarget {
        is_row: source.is_row,
        main,
        cross,
        main_viewport,
        cross_viewport,
    }
}

fn scene_string(instance: &Instance, reference: u32) -> &str {
    usize::try_from(reference)
        .ok()
        .and_then(|index| instance.st.scene_strs.get(index))
        .map_or("", String::as_str)
}

fn transformed_bounds(
    scene: &[SceneNode],
    scene_index: usize,
    offset_x: f64,
    offset_y: f64,
) -> Rect {
    let source = &scene[scene_index];
    let x0 = source.x;
    let y0 = source.y;
    let x1 = x0 + source.w.max(0.0);
    let y1 = y0 + source.h.max(0.0);
    let mut corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    let mut current = Some(scene_index);
    for _ in 0..scene.len() {
        let Some(index) = current else {
            break;
        };
        let ancestor = &scene[index];
        if ancestor.rot_deg != 0.0 && ancestor.rot_deg.is_finite() {
            let radians = ancestor.rot_deg.to_radians();
            let cosine = radians.cos();
            let sine = radians.sin();
            for (x, y) in &mut corners {
                let dx = *x - ancestor.rot_cx;
                let dy = *y - ancestor.rot_cy;
                *x = ancestor.rot_cx + dx * cosine - dy * sine;
                *y = ancestor.rot_cy + dx * sine + dy * cosine;
            }
        }
        current = usize::try_from(ancestor.parent_ix)
            .ok()
            .filter(|parent| *parent < scene.len());
    }
    let mut bounds = Rect {
        x0: f64::INFINITY,
        y0: f64::INFINITY,
        x1: f64::NEG_INFINITY,
        y1: f64::NEG_INFINITY,
    };
    for (x, y) in corners {
        bounds.x0 = bounds.x0.min(x + offset_x);
        bounds.y0 = bounds.y0.min(y + offset_y);
        bounds.x1 = bounds.x1.max(x + offset_x);
        bounds.y1 = bounds.y1.max(y + offset_y);
    }
    bounds
}

fn role_from_name(name: &str, kind: u32, flags: u32) -> Role {
    match name {
        "none" | "presentation" | "generic" | "generic-container" => Role::GenericContainer,
        "text" | "text-run" => Role::TextRun,
        "cell" => Role::Cell,
        "label" => Role::Label,
        "image" | "img" => Role::Image,
        "link" => Role::Link,
        "row" => Role::Row,
        "listitem" | "list-item" => Role::ListItem,
        "treeitem" | "tree-item" => Role::TreeItem,
        "option" | "listbox-option" => Role::ListBoxOption,
        "menuitem" | "menu-item" => Role::MenuItem,
        "paragraph" => Role::Paragraph,
        "checkbox" => Role::CheckBox,
        "radio" | "radio-button" => Role::RadioButton,
        "textbox" | "text-input" => {
            if flags & slir::F_MULTILINE != 0 {
                Role::MultilineTextInput
            } else {
                Role::TextInput
            }
        }
        "button" => Role::Button,
        "default-button" => Role::DefaultButton,
        "pane" => Role::Pane,
        "rowheader" | "row-header" => Role::RowHeader,
        "columnheader" | "column-header" => Role::ColumnHeader,
        "rowgroup" | "row-group" => Role::RowGroup,
        "list" => Role::List,
        "table" => Role::Table,
        "switch" => Role::Switch,
        "menu" => Role::Menu,
        "searchbox" | "search-input" => Role::SearchInput,
        "number-input" => Role::NumberInput,
        "password-input" => Role::PasswordInput,
        "abbr" => Role::Abbr,
        "alert" => Role::Alert,
        "alertdialog" | "alert-dialog" => Role::AlertDialog,
        "application" => Role::Application,
        "article" => Role::Article,
        "banner" => Role::Banner,
        "blockquote" => Role::Blockquote,
        "canvas" => Role::Canvas,
        "caption" => Role::Caption,
        "code" => Role::Code,
        "combobox" | "combo-box" => Role::ComboBox,
        "complementary" => Role::Complementary,
        "contentinfo" | "content-info" => Role::ContentInfo,
        "definition" => Role::Definition,
        "description-list" => Role::DescriptionList,
        "details" => Role::Details,
        "dialog" => Role::Dialog,
        "document" => Role::Document,
        "emphasis" => Role::Emphasis,
        "feed" => Role::Feed,
        "figure" => Role::Figure,
        "form" => Role::Form,
        "grid" => Role::Grid,
        "gridcell" | "grid-cell" => Role::GridCell,
        "group" => Role::Group,
        "heading" => Role::Heading,
        "listbox" | "list-box" => Role::ListBox,
        "log" => Role::Log,
        "main" => Role::Main,
        "marquee" => Role::Marquee,
        "math" => Role::Math,
        "menubar" | "menu-bar" => Role::MenuBar,
        "menuitemcheckbox" | "menu-item-checkbox" => Role::MenuItemCheckBox,
        "menuitemradio" | "menu-item-radio" => Role::MenuItemRadio,
        "meter" => Role::Meter,
        "navigation" => Role::Navigation,
        "note" => Role::Note,
        "progressbar" | "progress-bar" => Role::ProgressIndicator,
        "radiogroup" | "radio-group" => Role::RadioGroup,
        "region" => Role::Region,
        "scrollbar" | "scroll-bar" => Role::ScrollBar,
        "scrollview" | "scroll-view" => Role::ScrollView,
        "search" => Role::Search,
        "separator" | "splitter" => Role::Splitter,
        "slider" => Role::Slider,
        "spinbutton" | "spin-button" => Role::SpinButton,
        "status" => Role::Status,
        "strong" => Role::Strong,
        "tab" => Role::Tab,
        "tablist" | "tab-list" => Role::TabList,
        "tabpanel" | "tab-panel" => Role::TabPanel,
        "term" => Role::Term,
        "timer" => Role::Timer,
        "toolbar" => Role::Toolbar,
        "tooltip" => Role::Tooltip,
        "tree" => Role::Tree,
        "treegrid" | "tree-grid" => Role::TreeGrid,
        "window" => Role::Window,
        "" if flags & (slir::F_SCROLL | slir::F_SCROLL_CROSS) != 0 => Role::ScrollView,
        "" => match kind {
            slir::K_TEXT | slir::K_SPAN => Role::TextRun,
            slir::K_PARA => Role::Paragraph,
            slir::K_IMG => Role::Image,
            slir::K_CANVAS => Role::Canvas,
            slir::K_DIVIDER => Role::Splitter,
            _ => Role::GenericContainer,
        },
        _ => Role::Unknown,
    }
}

/// Owns the platform adapter and delivers only changed tree snapshots.
///
/// Create one bridge after the winit window. Pass every window event to
/// [`Self::process_event`]. After each settled frame, call [`Self::refresh`]
/// and then [`Self::update`]. Route adapter actions through
/// [`Self::resolve_action`] and [`RoutedAction::apply`].
pub struct WindowAccessibility {
    adapter: Adapter,
    bridge: Bridge,
    full_pending: bool,
}

impl WindowAccessibility {
    /// Mounts AccessKit for one winit window and an event-loop proxy whose
    /// user-event type can carry accessibility alongside application events.
    pub fn new<T>(event_loop: &ActiveEventLoop, window: &Window, proxy: EventLoopProxy<T>) -> Self
    where
        T: From<Event> + Send + 'static,
    {
        Self {
            adapter: Adapter::with_event_loop_proxy(event_loop, window, proxy),
            bridge: Bridge::default(),
            full_pending: false,
        }
    }

    /// Forwards one winit window event to the platform adapter.
    pub fn process_event(&mut self, window: &Window, event: &WindowEvent) {
        self.adapter.process_event(window, event);
    }

    /// Projects settled kernel frames into the pending accessibility tree.
    pub fn refresh(
        &mut self,
        title: &str,
        width: f64,
        height: f64,
        scale: f64,
        layers: &[SceneLayer<'_>],
    ) {
        self.bridge.refresh(title, width, height, scale, layers);
    }

    /// Publishes changed nodes, or a full tree when `force_full` is true.
    pub fn update(&mut self, force_full: bool) {
        self.full_pending |= force_full;
        let Some(update) = self.bridge.prepare_update(self.full_pending) else {
            return;
        };
        let mut update = Some(update);
        let mut applied = false;
        self.adapter.update_if_active(|| {
            applied = true;
            update
                .take()
                .expect("AccessKit update closure called more than once")
        });
        if applied {
            self.bridge.commit();
            self.full_pending = false;
        }
    }

    /// Resolves one platform action against the latest published scene.
    pub fn resolve_action(&self, request: &ActionRequest) -> Option<RoutedAction> {
        self.bridge.resolve_action(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use accesskit::Point;
    use slab_kernel::flatten::{Frame, SceneNode};

    fn source(node: u32, parent_ix: i32, kind: u32, flags: u32) -> SceneNode {
        SceneNode {
            node,
            parent_ix,
            authored_order: 0,
            kind,
            x: f64::from(node) * 10.0,
            y: 4.0,
            w: 40.0,
            h: 20.0,
            radius: 0.0,
            rot_deg: 0.0,
            rot_cx: 0.0,
            rot_cy: 0.0,
            flags,
            content_main: 0.0,
            scroll_off: 0.0,
            scroll_cross: 0.0,
            content_cross: 0.0,
            is_row: false,
            src_line: 1,
            role: 0,
            label: 0,
            desc: 0,
            checked: 0,
            expanded: 0,
            selected: 0,
            active_descendant: 0,
            controls: 0,
            value_now: None,
            value_min: None,
            value_max: None,
            value_text: 0,
            modal: 0,
            live: 0,
            live_atomic: 0,
            level: None,
            pos_in_set: None,
            set_size: None,
            disabled: false,
            focused: false,
        }
    }

    fn instance_with_keys(keys: &[&str]) -> Instance {
        let mut instance = kframe::inst_shell();
        instance.doc.strs = vec![String::new()];
        for key in keys {
            instance.doc.strs.push((*key).to_owned());
        }
        instance.doc.node_key = (1..=keys.len())
            .map(|index| u32::try_from(index).unwrap())
            .collect();
        instance.doc.node_kind = vec![slir::K_COL; keys.len()];
        instance.doc.node_flags = vec![0; keys.len()];
        instance.doc.node_parent = vec![slir::NONE; keys.len()];
        instance.doc.attr_index = vec![0; keys.len() + 1];
        instance.st.scene_strs = vec![String::new()];
        instance
    }

    fn frame(scene: Vec<SceneNode>) -> Frame {
        Frame {
            width: 200.0,
            height: 100.0,
            ops: Vec::new(),
            scene,
            strings: Vec::new(),
            paths_rt: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn role_and_state_mapping_preserves_absent_false_true_and_mixed() {
        assert_eq!(role_from_name("checkbox", slir::K_RECT, 0), Role::CheckBox);
        assert_eq!(role_from_name("separator", slir::K_RECT, 0), Role::Splitter);
        assert_eq!(
            role_from_name("vendor-widget", slir::K_RECT, 0),
            Role::Unknown
        );

        let mut instance = instance_with_keys(&["toggle"]);
        instance.st.scene_strs.extend([
            "Toggle".to_owned(),
            "details".to_owned(),
            "mixed".to_owned(),
        ]);
        let mut source = source(0, -1, slir::K_RECT, slir::F_FOCUSABLE);
        source.label = 1;
        source.desc = 2;
        source.value_text = 3;
        source.checked = 3;
        source.expanded = 1;
        source.selected = 2;
        source.modal = 2;
        source.live = 2;
        source.live_atomic = 2;
        source.value_now = Some(4.0);
        source.value_min = Some(0.0);
        source.value_max = Some(10.0);
        source.level = Some(2.0);
        source.pos_in_set = Some(3.0);
        source.set_size = Some(5.0);
        source.disabled = true;
        source.flags |= slir::F_INERT;
        let mut node = Node::new(Role::CheckBox);
        apply_properties(
            &mut node,
            &instance,
            &source,
            Rect::new(source.x, source.y, source.x + source.w, source.y + source.h),
        );

        assert_eq!(node.label(), Some("Toggle"));
        assert_eq!(node.description(), Some("details"));
        assert_eq!(node.value(), Some("mixed"));
        assert_eq!(node.toggled(), Some(Toggled::Mixed));
        assert_eq!(node.is_expanded(), Some(false));
        assert_eq!(node.is_selected(), Some(true));
        assert!(node.is_modal());
        assert_eq!(node.live(), Some(Live::Polite));
        assert!(node.is_live_atomic());
        assert!(node.is_disabled());
        assert!(node.is_hidden());
        assert_eq!(node.numeric_value(), Some(4.0));
        assert_eq!(node.min_numeric_value(), Some(0.0));
        assert_eq!(node.max_numeric_value(), Some(10.0));
        assert_eq!(node.level(), Some(2));
        assert_eq!(node.position_in_set(), Some(3));
        assert_eq!(node.size_of_set(), Some(5));
    }

    #[test]
    fn nested_rotation_and_layer_offset_are_projected_before_root_physical_scale() {
        let instance = instance_with_keys(&["parent", "child"]);
        let mut parent = source(0, -1, slir::K_COL, 0);
        parent.x = 0.0;
        parent.y = 0.0;
        parent.w = 30.0;
        parent.h = 30.0;
        parent.rot_deg = 90.0;
        parent.rot_cx = 0.0;
        parent.rot_cy = 0.0;
        let mut child = source(1, 0, slir::K_TEXT, 0);
        child.x = 10.0;
        child.y = 0.0;
        child.w = 10.0;
        child.h = 5.0;
        let frame = frame(vec![parent, child]);
        let mut bridge = Bridge::default();
        let layer = SceneLayer::new(5, &instance, &frame).translated(100.0, 50.0);
        bridge.refresh("test", 200.0, 100.0, 2.0, &[layer]);

        let snapshot = bridge.source().unwrap();
        let root = snapshot.nodes.get(&ROOT_ID).unwrap();
        assert_eq!(root.transform(), Some(&Affine::scale(2.0)));
        let child = snapshot
            .nodes
            .values()
            .find(|node| node.author_id() == Some("child"))
            .unwrap();
        let bounds = child.bounds().unwrap();
        for (actual, expected) in [
            (bounds.x0, 95.0),
            (bounds.y0, 60.0),
            (bounds.x1, 100.0),
            (bounds.y1, 70.0),
        ] {
            assert!((actual - expected).abs() < 1.0e-9);
        }
    }

    #[test]
    fn stale_disabled_inert_or_nonfocusable_kernel_focus_normalizes_to_root() {
        let instance = instance_with_keys(&["target"]);
        for (flags, disabled) in [
            (slir::F_FOCUSABLE, true),
            (slir::F_FOCUSABLE | slir::F_INERT, false),
            (0, false),
        ] {
            let mut target = source(0, -1, slir::K_RECT, flags);
            target.disabled = disabled;
            target.focused = true;
            let frame = frame(vec![target]);
            let mut bridge = Bridge::default();
            bridge.refresh(
                "test",
                100.0,
                100.0,
                1.0,
                &[SceneLayer::new(3, &instance, &frame)],
            );
            assert_eq!(bridge.prepare_update(false).unwrap().focus, ROOT_ID);
        }
    }

    #[test]
    fn stable_ids_hierarchy_and_incremental_updates_follow_scene_keys() {
        let mut instance = instance_with_keys(&["root", "child"]);
        instance.st.scene_strs.push("child".to_owned());
        let mut root_source = source(0, -1, slir::K_COL, 0);
        root_source.active_descendant = 1;
        root_source.controls = 1;
        let mut child_source = source(1, 0, slir::K_TEXT, slir::F_FOCUSABLE);
        child_source.focused = true;
        let first = frame(vec![root_source.clone(), child_source.clone()]);
        let mut bridge = Bridge::default();
        bridge.refresh(
            "test",
            200.0,
            100.0,
            2.0,
            &[SceneLayer::new(7, &instance, &first)],
        );
        let initial = bridge.prepare_update(false).unwrap();
        assert!(initial.tree.is_some());
        let root_scene_id = initial
            .nodes
            .iter()
            .find(|(_, node)| node.author_id() == Some("root"))
            .map(|(id, _)| *id)
            .unwrap();
        let child_id = initial
            .nodes
            .iter()
            .find(|(_, node)| node.author_id() == Some("child"))
            .map(|(id, _)| *id)
            .unwrap();
        assert_eq!(initial.nodes[0].0, ROOT_ID);
        assert_eq!(initial.focus, child_id);
        let root_scene = initial
            .nodes
            .iter()
            .find(|(id, _)| *id == root_scene_id)
            .unwrap();
        assert_eq!(root_scene.1.children(), &[child_id]);
        assert_eq!(root_scene.1.active_descendant(), Some(child_id));
        assert_eq!(root_scene.1.controls(), &[child_id]);
        bridge.commit();

        bridge.refresh(
            "test",
            200.0,
            100.0,
            2.0,
            &[SceneLayer::new(7, &instance, &first)],
        );
        assert!(bridge.prepare_update(false).is_none());

        let absent = frame(vec![root_source.clone()]);
        bridge.refresh(
            "test",
            200.0,
            100.0,
            2.0,
            &[SceneLayer::new(7, &instance, &absent)],
        );
        assert!(bridge.prepare_update(false).is_some());
        assert!(
            bridge
                .resolve_action(&ActionRequest {
                    action: Action::Focus,
                    target_tree: TreeId::ROOT,
                    target_node: child_id,
                    data: None,
                })
                .is_none(),
            "a stale platform ID must stop resolving as soon as its scene key disappears"
        );
        bridge.commit();

        bridge.refresh(
            "test",
            200.0,
            100.0,
            2.0,
            &[SceneLayer::new(7, &instance, &first)],
        );
        let rematerialized = bridge.prepare_update(false).unwrap();
        assert!(
            rematerialized.nodes.iter().any(|(id, _)| *id == child_id),
            "a rematerialized key must recover its original platform ID"
        );
        bridge.commit();

        let mut moved_child = child_source;
        moved_child.x = 99.0;
        let moved = frame(vec![root_source, moved_child]);
        bridge.refresh(
            "test",
            200.0,
            100.0,
            2.0,
            &[SceneLayer::new(7, &instance, &moved)],
        );
        let update = bridge.prepare_update(false).unwrap();
        assert_eq!(update.nodes.len(), 1);
        assert_eq!(update.nodes[0].0, child_id);
    }

    #[test]
    fn stable_queue_document_identity_survives_backing_instance_selection() {
        let first_instance = instance_with_keys(&["queue-row"]);
        let second_instance = instance_with_keys(&["queue-row"]);
        let first_frame = frame(vec![source(0, -1, slir::K_RECT, slir::F_FOCUSABLE)]);
        let second_frame = frame(vec![source(0, -1, slir::K_RECT, slir::F_FOCUSABLE)]);
        let mut bridge = Bridge::default();
        bridge.refresh(
            "test",
            100.0,
            100.0,
            1.0,
            &[SceneLayer::new(1, &first_instance, &first_frame)],
        );
        let first_id = bridge
            .source()
            .unwrap()
            .nodes
            .iter()
            .find_map(|(id, node)| (node.author_id() == Some("queue-row")).then_some(*id))
            .unwrap();
        bridge.commit();

        bridge.refresh(
            "test",
            100.0,
            100.0,
            1.0,
            &[SceneLayer::new(1, &second_instance, &second_frame)],
        );
        let second_id = bridge
            .source()
            .unwrap()
            .nodes
            .iter()
            .find_map(|(id, node)| (node.author_id() == Some("queue-row")).then_some(*id))
            .unwrap();
        assert_eq!(second_id, first_id);
        assert!(bridge.prepare_update(false).is_none());
    }

    #[test]
    fn action_resolution_returns_exact_document_and_key_without_os_adapter() {
        let mut instance = instance_with_keys(&["split", "scroll"]);
        instance.doc.node_kind[0] = slir::K_DIVIDER;
        instance.doc.sign_node = vec![0];
        instance.doc.sign_trigger = vec![kdispatch::TR_ACTIVATE];
        instance.doc.sign_name = vec![0];
        let divider = source(0, -1, slir::K_DIVIDER, slir::F_FOCUSABLE);
        let mut scroller = source(1, -1, slir::K_COL, slir::F_SCROLL);
        scroller.is_row = true;
        scroller.content_main = 300.0;
        scroller.w = 100.0;
        let frame = frame(vec![divider, scroller]);
        let mut bridge = Bridge::default();
        bridge.refresh(
            "test",
            200.0,
            100.0,
            1.0,
            &[SceneLayer::new(42, &instance, &frame)],
        );
        let snapshot = bridge.source().unwrap();
        let split_id = snapshot
            .routes
            .iter()
            .find(|(_, target)| target.key == "split")
            .map(|(id, _)| *id)
            .unwrap();
        let scroll_id = snapshot
            .routes
            .iter()
            .find(|(_, target)| target.key == "scroll")
            .map(|(id, _)| *id)
            .unwrap();

        let focus = bridge
            .resolve_action(&ActionRequest {
                action: Action::Focus,
                target_tree: TreeId::ROOT,
                target_node: split_id,
                data: None,
            })
            .unwrap();
        assert_eq!(focus.document, 42);
        assert_eq!(focus.key, "split");
        assert_eq!(focus.kind, RoutedActionKind::Focus);

        let default = bridge
            .resolve_action(&ActionRequest {
                action: Action::Click,
                target_tree: TreeId::ROOT,
                target_node: split_id,
                data: None,
            })
            .unwrap();
        assert_eq!(default.kind, RoutedActionKind::Default);

        let increment = bridge
            .resolve_action(&ActionRequest {
                action: Action::Increment,
                target_tree: TreeId::ROOT,
                target_node: split_id,
                data: None,
            })
            .unwrap();
        assert_eq!(increment.kind, RoutedActionKind::DividerKey("ArrowDown"));

        let page_right = bridge
            .resolve_action(&ActionRequest {
                action: Action::ScrollRight,
                target_tree: TreeId::ROOT,
                target_node: scroll_id,
                data: Some(ActionData::ScrollUnit(ScrollUnit::Page)),
            })
            .unwrap();
        assert_eq!(
            page_right.kind,
            RoutedActionKind::ScrollBy {
                axis: 0,
                delta: 100.0,
            }
        );

        let set_scroll = bridge
            .resolve_action(&ActionRequest {
                action: Action::SetScrollOffset,
                target_tree: TreeId::ROOT,
                target_node: scroll_id,
                data: Some(ActionData::SetScrollOffset(Point::new(12.0, 34.0))),
            })
            .unwrap();
        assert_eq!(
            set_scroll.kind,
            RoutedActionKind::SetScroll {
                main: Some(12.0),
                cross: None,
            }
        );

        assert!(
            bridge
                .resolve_action(&ActionRequest {
                    action: Action::SetValue,
                    target_tree: TreeId::ROOT,
                    target_node: split_id,
                    data: Some(ActionData::NumericValue(10.0)),
                })
                .is_none()
        );
    }
}
