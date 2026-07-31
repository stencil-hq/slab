//! Per-node style resolution.
//!
//! Base attributes are overlaid by active conditional patches in document
//! order (last wins). Parameter and list-property references resolve next.
//! The inherited text-style whitelist includes color, family, size, weight,
//! leading, tracking, italic, underline, and strike.
//!
//! Runtime layers include node states maintained by dispatch, scroll offsets
//! owned by the host and dispatch, field text overrides maintained by editing,
//! and motion's per-solve interpolated attribute inputs. Motion values precede
//! patches and base attributes so layout always re-solves from interpolated
//! inputs. Overlay tuples are stored in [`St::mo_f`] under [`T_OV_TUPLE`].
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Serialize;

pub use crate::slir::ATTR_COUNT;

/// Mutable style-resolution state owned by an instance.
///
/// Parameter values, hole reports, interaction state, and list state persist
/// between solves. Condition results, motion overlays, resolved styles, grid
/// tracks, and diagnostics are rebuilt for each solve.
#[derive(Clone, Debug)]
pub struct St {
	pub lists: crate::list::State,
	pub env: crate::when::Env,
	/// Zero for authored base; otherwise one plus the active theme row.
	pub theme_index: u32,
	/// Global state set, stored as string-pool references.
	pub states: Vec<u32>,
	/// Per-node dispatch states, parallel with [`Self::ns_sym`].
	pub ns_node: Vec<u32>,
	pub ns_sym: Vec<u32>,
	/// Nodes carrying the interned "disabled" state; mirrors matching entries
	/// in [`Self::ns_node`] and [`Self::ns_sym`].
	disabled_nodes: FxHashSet<u32>,
	/// Scroll owners, parallel with [`Self::scroll_off`].
	pub scroll_node: Vec<u32>,
	pub scroll_off: Vec<f64>,
	/// Cross-axis scroll owners, parallel with [`Self::scroll_cross_off`].
	pub scroll_cross_node: Vec<u32>,
	pub scroll_cross_off: Vec<f64>,
	/// Field nodes whose content is overridden by [`Self::field_text`].
	pub field_node: Vec<u32>,
	pub field_text: Vec<crate::text::Text>,
	pub field_scroll_node: Vec<u32>,
	pub field_scroll_off: Vec<f64>,
	/// Keyed divider size overlays.
	pub divider_node: Vec<u32>,
	pub divider_extent: Vec<f64>,
	/// Divider nodes with retained solved main-axis footprints.
	pub divider_footprint_node: Vec<u32>,
	/// Solved extents parallel to [`Self::divider_footprint_node`].
	pub divider_footprint: Vec<f64>,
	/// Whether a retained divider footprint changed enough to require a settle
	/// solve.
	pub divider_footprint_changed: bool,
	/// Retained split-pane sizes keyed by canonical full scene key.
	pub split_key: Vec<String>,
	pub split_extent: Vec<f64>,
	/// Compact retained scalar parameter values.
	pub(crate) params: crate::params::ParamStore,
	/// Stable host-registered image slots; inactive entries retain their unified
	/// indices.
	pub(crate) runtime_images: Vec<crate::frame::RuntimeImage>,
	/// Missing image names already diagnosed by this instance.
	pub(crate) img_missing: std::collections::HashSet<String>,
	/// Append-only per-instance scene string pool; index zero is always empty.
	pub scene_strs: Vec<String>,
	scene_str_index: FxHashMap<String, u32>,
	/// Instance-lifetime normalized runtime paths keyed by authored data.
	pub rt_path_ix: std::collections::HashMap<String, u32>,
	/// Runtime path verb streams, parallel with [`Self::rt_path_coords`].
	pub rt_path_verbs: Vec<Vec<u8>>,
	/// Runtime path coordinate streams, parallel with [`Self::rt_path_verbs`].
	pub rt_path_coords: Vec<Vec<f64>>,
	/// Invalid runtime path strings already diagnosed by this instance.
	pub rt_path_bad: std::collections::HashSet<String>,
	/// Missing icon names already diagnosed by this instance.
	pub icon_missing: std::collections::HashSet<String>,
	/// Host-reported hole widths, parallel with [`Self::hole_h`].
	pub hole_w: Vec<f64>,
	pub hole_h: Vec<f64>,
	/// Solve-global environment and client condition results.
	pub cond_on: Vec<bool>,
	/// Effective patch state; size comparisons are refreshed per node.
	pub patch_on: Vec<bool>,
	base_attr_values: Vec<[i32; ATTR_COUNT]>,
	effective_attr_node: u32,
	effective_attr_values: [i32; ATTR_COUNT],
	font_selection: Vec<Vec<(u32, i32)>>,
	family_index: FxHashMap<String, u32>,
	/// Authored nodes with an activate signal, indexed by document node.
	activate_node: Vec<bool>,
	/// Precomputed keyword codes per string-pool entry, parallel with
	/// `Doc::strs`.
	kw_codes: Vec<KwCodes>,
	/// Per-node motion-overlay presence, indexed by node; nodes beyond the
	/// vector carry no overlay. Skips `mo_index` hashing for the common
	/// overlay-free node.
	mo_node_has: Vec<bool>,
	/// Theme-resolved decoded value per AVAL entry, parallel with
	/// `Doc::aval_tag`. Rebuilt on document init and theme change.
	aval_active: Vec<crate::value::V>,
	/// Synthetic-node size-condition results keyed by node and patch.
	pub wh_node: Vec<u32>,
	pub wh_patch: Vec<i32>,
	pub wh_on: Vec<bool>,
	/// Motion overlay nodes, with later entries taking precedence.
	pub mo_node: Vec<u32>,
	pub mo_attr: Vec<u32>,
	/// Value tags, including missing values and [`T_OV_TUPLE`].
	pub mo_tag: Vec<u32>,
	pub mo_num: Vec<f64>,
	pub mo_h: Vec<u32>,
	/// Overlay tuple slices into [`Self::mo_f`].
	pub mo_off: Vec<i32>,
	pub mo_ln: Vec<i32>,
	pub mo_f: Vec<f64>,
	/// Last overlay slot per (node, attr); mirrors the parallel arrays above.
	mo_index: FxHashMap<(u32, u32), usize>,
	/// Per-node text measurement results, valid only for identical inputs.
	pub text_layout_cache: FxHashMap<u32, crate::textm::TextCacheEntry>,
	/// Previous text-cache generation, probed on miss and dropped at the next
	/// swap.
	pub text_layout_cache_cold: FxHashMap<u32, crate::textm::TextCacheEntry>,
	/// Measured-content lineage per editable field node.
	pub text_rev: FxHashMap<u32, FieldTextRev>,
	pub rs: Vec<crate::style::RStyle>,
	/// Grid track kinds: fixed, hug, fill, or percentage.
	pub track_kind: Vec<u32>,
	pub track_v: Vec<f64>,
	pub diag_code: Vec<String>,
	pub diag_msg: Vec<String>,
	pub diag_line: Vec<u32>,
	pub warned_fill_unbounded: bool,
}

/// Creates empty style state with default environment and list state.
pub fn st_new() -> crate::style::St {
	crate::style::St {
		env: crate::when::env_default(),
		lists: crate::list::state_new(),
		theme_index: 0,
		states: vec![],
		ns_node: vec![],
		ns_sym: vec![],
		disabled_nodes: FxHashSet::default(),
		scroll_node: vec![],
		scroll_off: vec![],
		scroll_cross_node: vec![],
		scroll_cross_off: vec![],
		field_node: vec![],
		field_text: vec![],
		field_scroll_node: vec![],
		field_scroll_off: vec![],
		divider_node: vec![],
		divider_extent: vec![],
		divider_footprint_node: vec![],
		divider_footprint: vec![],
		divider_footprint_changed: false,
		split_key: vec![],
		split_extent: vec![],
		params: crate::params::ParamStore::default(),
		runtime_images: vec![],
		img_missing: std::collections::HashSet::new(),
		scene_strs: vec![String::new()],
		scene_str_index: FxHashMap::default(),
		rt_path_ix: std::collections::HashMap::new(),
		rt_path_verbs: vec![],
		rt_path_coords: vec![],
		rt_path_bad: std::collections::HashSet::new(),
		icon_missing: std::collections::HashSet::new(),
		hole_w: vec![],
		hole_h: vec![],
		cond_on: vec![],
		patch_on: vec![],
		base_attr_values: vec![],
		effective_attr_node: crate::slir::NONE,
		effective_attr_values: [-1; ATTR_COUNT],
		font_selection: vec![],
		family_index: FxHashMap::default(),
		activate_node: vec![],
		kw_codes: vec![],
		wh_node: vec![],
		wh_patch: vec![],
		wh_on: vec![],
		mo_node: vec![],
		mo_attr: vec![],
		mo_tag: vec![],
		mo_num: vec![],
		mo_h: vec![],
		mo_off: vec![],
		mo_ln: vec![],
		mo_f: vec![],
		mo_index: FxHashMap::default(),
		mo_node_has: vec![],
		aval_active: vec![],
		text_layout_cache: FxHashMap::default(),
		text_layout_cache_cold: FxHashMap::default(),
		text_rev: FxHashMap::default(),
		rs: vec![],
		track_kind: vec![],
		track_v: vec![],
		diag_code: vec![],
		diag_msg: vec![],
		diag_line: vec![],
		warned_fill_unbounded: false,
	}
}

/// Appends one diagnostic while preserving diagnostic pool ordering.
pub fn warn(st: &mut crate::style::St, code: &str, msg: &str, line: u32) {
	st.diag_code.push(code.to_string());
	st.diag_msg.push(msg.to_string());
	st.diag_line.push(line);
}

fn authored_attr_values(d: &crate::slir::Doc, node: usize) -> [i32; ATTR_COUNT] {
	let mut values = [-1; ATTR_COUNT];
	let start = index_i32(d.attr_index[node]);
	let end = index_i32(d.attr_index[node + 1]);
	for attr_index in start..end {
		let attr = index_u32(d.attr_id[attr_index]);
		if let Some(value) = values.get_mut(attr)
			&& *value < 0
		{
			*value = i32::from_ne_bytes(d.attr_val[attr_index].to_ne_bytes());
		}
	}
	values
}

/// Initializes persistent parameter defaults and zeroed host hole reports.
pub fn init_params(d: &crate::slir::Doc, st: &mut crate::style::St) {
	st.theme_index = if st.env.theme.is_empty() {
		0
	} else {
		d.theme_name
			.iter()
			.position(|&name| crate::slir::str_at(d, name) == st.env.theme)
			.and_then(|index| u32::try_from(index).ok())
			.and_then(|index| index.checked_add(1))
			.unwrap_or(0)
	};
	st.params.init(d);
	st.scene_str_index.clear();
	st.scene_strs.clear();
	st.scene_strs.push(String::new());
	st.hole_w.clear();
	st.hole_h.clear();
	st.hole_w.resize(d.hole_name.len(), 0.0);
	st.hole_h.resize(d.hole_name.len(), 0.0);
	st.base_attr_values.clear();
	st.font_selection.clear();
	st.family_index.clear();
	// Node ids and font indices alias across a doc swap; drop stale text
	// and its measured-content lineage together.
	st.text_layout_cache.clear();
	st.text_layout_cache_cold.clear();
	st.text_rev.clear();
	st.base_attr_values
		.extend((0..d.node_kind.len()).map(|node| authored_attr_values(d, node)));
	st.kw_codes.clear();
	st.kw_codes.extend(d.strs.iter().map(|s| kw_codes_of(s)));
	st.activate_node.clear();
	st.activate_node.resize(d.node_kind.len(), false);
	for (&node, &trigger) in d.sign_node.iter().zip(&d.sign_trigger) {
		if trigger == crate::dispatch::TR_ACTIVATE
			&& let Some(activates) = st.activate_node.get_mut(index_u32(node))
		{
			*activates = true;
		}
	}
	rebuild_aval_cache(d, st);
	crate::list::init(d, &mut st.lists);
}

/// Returns whether a synthetic node no longer belongs to an active list item.
pub fn stale_synthetic(d: &crate::slir::Doc, st: &crate::style::St, node: u32) -> bool {
	usize::try_from(node).expect("node index exceeds usize") >= d.node_kind.len()
		&& !crate::list::is_split_sash(&st.lists, node)
		&& crate::list::base(&st.lists, d, node) == crate::slir::NONE
}

/// Removes persistent style and interaction state for vanished synthetic nodes.
pub fn prune_node_state(d: &crate::slir::Doc, st: &mut crate::style::St) {
	let mut index = st.ns_node.len();
	while index > 0 {
		index -= 1;
		if crate::style::stale_synthetic(d, st, st.ns_node[index]) {
			let node = st.ns_node.swap_remove(index);
			let sym = st.ns_sym.swap_remove(index);
			if crate::rt::str_eq(crate::slir::str_at(d, sym), "disabled") {
				st.disabled_nodes.remove(&node);
			}
		}
	}

	let mut index = st.scroll_node.len();
	while index > 0 {
		index -= 1;
		if crate::style::stale_synthetic(d, st, st.scroll_node[index]) {
			st.scroll_node.swap_remove(index);
			st.scroll_off.swap_remove(index);
		}
	}

	let mut index = st.scroll_cross_node.len();
	while index > 0 {
		index -= 1;
		if crate::style::stale_synthetic(d, st, st.scroll_cross_node[index]) {
			st.scroll_cross_node.swap_remove(index);
			st.scroll_cross_off.swap_remove(index);
		}
	}

	let mut index = st.field_node.len();
	while index > 0 {
		index -= 1;
		if crate::style::stale_synthetic(d, st, st.field_node[index]) {
			let node = st.field_node[index];
			st.field_node.swap_remove(index);
			st.field_text.swap_remove(index);
			st.text_rev.remove(&node);
		}
	}

	let mut index = st.field_scroll_node.len();
	while index > 0 {
		index -= 1;
		if crate::style::stale_synthetic(d, st, st.field_scroll_node[index]) {
			st.field_scroll_node.swap_remove(index);
			st.field_scroll_off.swap_remove(index);
		}
	}

	let mut index = st.divider_node.len();
	while index > 0 {
		index -= 1;
		if crate::style::stale_synthetic(d, st, st.divider_node[index]) {
			st.divider_node.swap_remove(index);
			st.divider_extent.swap_remove(index);
		}
	}

	let mut index = st.divider_footprint_node.len();
	while index > 0 {
		index -= 1;
		if crate::style::stale_synthetic(d, st, st.divider_footprint_node[index]) {
			st.divider_footprint_node.swap_remove(index);
			st.divider_footprint.swap_remove(index);
		}
	}
}

fn refresh_virtual_window(d: &crate::slir::Doc, st: &mut crate::style::St, each: u32) {
	let Some((_, _, parent)) = crate::list::virtual_config(d, &st.lists, each) else {
		return;
	};
	let off = crate::style::scroll_get(st, parent);
	crate::list::materialized_window(d, &mut st.lists, each, off);
}

fn refresh_virtual_windows(d: &crate::slir::Doc, st: &mut crate::style::St) {
	for each_index in 0..d.node_kind.len() {
		if d.node_kind[each_index] != crate::slir::K_EACH
			|| d.node_flags[each_index] & crate::slir::F_VIRTUAL == 0
		{
			continue;
		}
		let each = u32::try_from(each_index).expect("node index exceeds u32");
		refresh_virtual_window(d, st, each);
	}
	let materialized_len = crate::list::materialized(&st.lists).len();
	for index in 0..materialized_len {
		let each = crate::list::materialized(&st.lists)[index];
		let base = crate::list::base(&st.lists, d, each);
		let Ok(base_index) = usize::try_from(base) else {
			continue;
		};
		if d.node_kind.get(base_index) != Some(&crate::slir::K_EACH)
			|| d.node_flags.get(base_index).copied().unwrap_or(0) & crate::slir::F_VIRTUAL == 0
		{
			continue;
		}
		refresh_virtual_window(d, st, each);
	}
}

/// Prepares style state for a solve and evaluates node-independent conditions.
///
/// State conditions are evaluated against each patch's node. Width and height
/// comparisons remain false until [`set_patch_flags`] receives that node's
/// incoming constraints. The motion overlay is cleared here and rebuilt before
/// layout reads attributes.
pub fn begin_solve(d: &crate::slir::Doc, st: &mut crate::style::St) {
	st.effective_attr_node = crate::slir::NONE;
	// Key and recursive-length setters form a host-side batch. Prune only at
	// the solve boundary, after transient reorder duplicates and child writes
	// are complete and before style or scene traversal can observe removals.
	crate::list::prune(d, &mut st.lists);
	// Motion samples before layout, so advance virtual windows from retained
	// geometry and the latest scroll input before collecting live identities.
	refresh_virtual_windows(d, st);
	crate::list::sync(d, &mut st.lists);
	crate::style::prune_node_state(d, st);
	st.disabled_nodes.clear();
	for (&node, &sym) in st.ns_node.iter().zip(&st.ns_sym) {
		if crate::rt::str_eq(crate::slir::str_at(d, sym), "disabled") {
			st.disabled_nodes.insert(node);
		}
	}
	st.divider_footprint_changed = false;
	st.cond_on.clear();
	st.patch_on.clear();
	st.wh_node.clear();
	st.wh_patch.clear();
	st.wh_on.clear();
	st.mo_node.clear();
	st.mo_attr.clear();
	st.mo_tag.clear();
	st.mo_num.clear();
	st.mo_h.clear();
	st.mo_off.clear();
	st.mo_ln.clear();
	st.mo_f.clear();
	st.mo_index.clear();
	st.mo_node_has.clear();
	st.rs.clear();
	st.track_kind.clear();
	st.track_v.clear();
	st.diag_code.clear();
	st.diag_msg.clear();
	st.diag_line.clear();
	st.warned_fill_unbounded = false;
	for (condition, &kind) in d.cond_kind.iter().enumerate() {
		if matches!(
			kind,
			crate::slir::C_WCMP | crate::slir::C_HCMP | crate::slir::C_STATE | crate::slir::C_PROP
		) {
			st.cond_on.push(false);
		} else {
			st.cond_on.push(crate::when::eval_cond(
				d,
				i32::try_from(condition).expect("condition index exceeds i32"),
				0,
				&st.env,
				&st.states,
				&st.params,
				0.0,
				0.0,
			));
		}
	}
	for (&condition, &node) in d.patch_cond.iter().zip(&d.patch_node) {
		let condition_index = usize::try_from(condition).expect("condition index exceeds usize");
		if d.cond_kind[condition_index] == crate::slir::C_STATE {
			st.patch_on.push(crate::when::eval_cond_ns(
				d,
				i32::try_from(condition).expect("condition index exceeds i32"),
				node,
				&st.env,
				&st.states,
				&st.ns_node,
				&st.ns_sym,
				&st.params,
				0.0,
				0.0,
			));
		} else {
			st.patch_on.push(st.cond_on[condition_index]);
		}
	}
}

/// Records a width/height condition result for a synthetic node and patch.
pub fn wh_set(st: &mut crate::style::St, node: u32, pi: i32, on: bool) {
	if let Some(index) = st
		.wh_node
		.iter()
		.zip(&st.wh_patch)
		.position(|(&candidate, &patch)| candidate == node && patch == pi)
	{
		st.wh_on[index] = on;
		return;
	}
	st.wh_node.push(node);
	st.wh_patch.push(pi);
	st.wh_on.push(on);
}

/// Refreshes a node's width/height comparison patches from incoming limits.
///
/// Layout calls this before building the resolved style for the node.
pub fn set_patch_flags(
	d: &crate::slir::Doc,
	st: &mut crate::style::St,
	node: u32,
	cw: f64,
	ch: f64,
) {
	st.effective_attr_node = crate::slir::NONE;
	let b = crate::list::base(&st.lists, d, node);
	let synthetic = crate::list::each_of(&st.lists, d, node) != crate::slir::NONE;
	let base_index = index_u32(b);
	let patch_count = st.lists.patches_by_node.get(base_index).map_or(0, Vec::len);
	for patch_position in 0..patch_count {
		let patch = st.lists.patches_by_node[base_index][patch_position];
		let condition = d.patch_cond[patch];
		let condition_index = usize::try_from(condition).expect("condition index exceeds usize");
		let kind = d.cond_kind[condition_index];
		if kind == crate::slir::C_WCMP || kind == crate::slir::C_HCMP {
			let patch_i32 = i32::try_from(patch).expect("patch index exceeds i32");
			let on = crate::when::eval_cond(
				d,
				i32::try_from(condition).expect("condition index exceeds i32"),
				node,
				&st.env,
				&st.states,
				&st.params,
				cw,
				ch,
			);
			if synthetic {
				crate::style::wh_set(st, node, patch_i32, on);
			} else {
				st.patch_on[patch] = on;
			}
		}
	}
}

/// Flips a named dispatch state on one node.
///
/// Names absent from the document cannot affect a condition and are ignored.
/// Returns whether the state set changed.
pub fn set_node_state(
	d: &crate::slir::Doc,
	st: &mut crate::style::St,
	node: u32,
	name: &str,
	on: bool,
) -> bool {
	let Some(sym) = d
		.strs
		.iter()
		.rposition(|candidate| crate::rt::str_eq(candidate, name))
		.map(|index| u32::try_from(index).expect("string index exceeds u32"))
	else {
		return false;
	};
	let index = st
		.ns_node
		.iter()
		.zip(&st.ns_sym)
		.rposition(|(&candidate, &candidate_sym)| candidate == node && candidate_sym == sym);
	match (on, index) {
		(true, None) => {
			st.ns_node.push(node);
			st.ns_sym.push(sym);
			if crate::rt::str_eq(name, "disabled") {
				st.disabled_nodes.insert(node);
			}
			true
		},
		(false, Some(index)) => {
			st.ns_node.swap_remove(index);
			st.ns_sym.swap_remove(index);
			if crate::rt::str_eq(name, "disabled") {
				st.disabled_nodes.remove(&node);
			}
			true
		},
		_ => false,
	}
}

/// Returns whether `node` carries the named, interned state.
pub fn node_state_on(d: &crate::slir::Doc, st: &crate::style::St, node: u32, name: &str) -> bool {
	st.ns_node.iter().zip(&st.ns_sym).any(|(&candidate, &sym)| {
		candidate == node && crate::rt::str_eq(crate::slir::str_at(d, sym), name)
	})
}

/// Returns whether `node` carries the interned "disabled" state.
pub fn node_disabled(st: &crate::style::St, node: u32) -> bool {
	st.disabled_nodes.contains(&node)
}

/// Returns a node's scroll offset, or zero when unset.
pub fn scroll_get(st: &crate::style::St, node: u32) -> f64 {
	st.scroll_node
		.iter()
		.zip(&st.scroll_off)
		.find_map(|(&candidate, &offset)| (candidate == node).then_some(offset))
		.unwrap_or(0.0)
}

/// Sets a node's scroll offset and returns whether it changed.
pub fn scroll_set(st: &mut crate::style::St, node: u32, off: f64) -> bool {
	if let Some(index) = st
		.scroll_node
		.iter()
		.position(|&candidate| candidate == node)
	{
		if st.scroll_off[index] == off {
			return false;
		}
		st.scroll_off[index] = off;
		return true;
	}
	if off == 0.0 {
		return false;
	}
	st.scroll_node.push(node);
	st.scroll_off.push(off);
	true
}

/// Returns a node's cross-axis scroll offset, or zero when unset.
pub fn scroll_cross_get(st: &crate::style::St, node: u32) -> f64 {
	st.scroll_cross_node
		.iter()
		.zip(&st.scroll_cross_off)
		.find_map(|(&candidate, &offset)| (candidate == node).then_some(offset))
		.unwrap_or(0.0)
}

/// Sets a node's cross-axis scroll offset and returns whether it changed.
pub fn scroll_cross_set(st: &mut crate::style::St, node: u32, off: f64) -> bool {
	if let Some(index) = st
		.scroll_cross_node
		.iter()
		.position(|&candidate| candidate == node)
	{
		if st.scroll_cross_off[index] == off {
			return false;
		}
		st.scroll_cross_off[index] = off;
		return true;
	}
	if off == 0.0 {
		return false;
	}
	st.scroll_cross_node.push(node);
	st.scroll_cross_off.push(off);
	true
}

/// Returns one scroll offset selected by `axis` (`0` main, `1` cross).
pub fn scroll_get_axis(st: &crate::style::St, node: u32, axis: u32) -> f64 {
	if axis == 1 {
		scroll_cross_get(st, node)
	} else {
		scroll_get(st, node)
	}
}

/// Sets one scroll offset selected by `axis` (`0` main, `1` cross).
pub fn scroll_set_axis(st: &mut crate::style::St, node: u32, axis: u32, off: f64) -> bool {
	if axis == 1 {
		scroll_cross_set(st, node, off)
	} else {
		scroll_set(st, node, off)
	}
}

/// Returns the persistent extent overlay owned by `node`, if one is set.
pub fn divider_get(st: &crate::style::St, node: u32) -> Option<f64> {
	st.divider_node
		.iter()
		.zip(&st.divider_extent)
		.find_map(|(&candidate, &extent)| (candidate == node).then_some(extent))
}

/// Returns the last solved main-axis footprint of a divider.
pub fn divider_footprint_get(st: &crate::style::St, node: u32) -> Option<f64> {
	st.divider_footprint_node
		.iter()
		.zip(&st.divider_footprint)
		.find_map(|(&candidate, &extent)| (candidate == node).then_some(extent))
}

/// Records a solved divider footprint and requests a settle when an overlay
/// uses it.
pub fn divider_footprint_set(st: &mut crate::style::St, node: u32, extent: f64) -> bool {
	let has_overlay = divider_get(st, node).is_some();
	if let Some(index) = st
		.divider_footprint_node
		.iter()
		.position(|&candidate| candidate == node)
	{
		let changed = (st.divider_footprint[index] - extent).abs() > crate::layout::EPS;
		st.divider_footprint[index] = extent;
		st.divider_footprint_changed |= has_overlay && changed;
		return changed;
	}
	st.divider_footprint_node.push(node);
	st.divider_footprint.push(extent);
	st.divider_footprint_changed |= has_overlay;
	true
}

/// Sets a divider extent overlay and returns whether its stored value changed.
pub fn divider_set(st: &mut crate::style::St, node: u32, extent: f64) -> bool {
	if let Some(index) = st
		.divider_node
		.iter()
		.position(|&candidate| candidate == node)
	{
		if st.divider_extent[index] == extent {
			return false;
		}
		st.divider_extent[index] = extent;
		return true;
	}
	st.divider_node.push(node);
	st.divider_extent.push(extent);
	true
}

/// Clears a divider extent overlay and returns whether one was present.
pub fn divider_clear(st: &mut crate::style::St, node: u32) -> bool {
	let Some(index) = st
		.divider_node
		.iter()
		.position(|&candidate| candidate == node)
	else {
		return false;
	};
	st.divider_node.remove(index);
	st.divider_extent.remove(index);
	true
}

/// Clamps a requested pane extent to its authored bounds and available budget.
pub const fn divider_clamp(requested: f64, min: f64, max: f64, budget_max: f64) -> f64 {
	min.max(requested.min(max).min(budget_max.max(min)))
}

/// Per-field measured-content lineage: a monotonic revision plus the splice
/// that produced the latest content, when one exists.
#[derive(Clone, Copy, Debug)]
pub struct FieldTextRev {
	/// Monotonic revision; the layout cache stores the revision it measured.
	pub rev:   u64,
	/// Forward splice from revision `rev - 1`, or `None` for a full change.
	pub delta: Option<crate::textm::TextDelta>,
}

/// Replaces the content override for an editable field node.
///
/// The change is recorded as a full (non-spliceable) transition; edit-driven
/// keystrokes go through [`field_set_spliced`] instead.
pub fn field_set(st: &mut crate::style::St, node: u32, text: &crate::text::Text) {
	field_set_with(st, node, text, None);
}

/// Replaces the content override with a contiguous-splice lineage, letting
/// layout re-measure only the hard lines the edit touched.
pub fn field_set_spliced(
	st: &mut crate::style::St,
	node: u32,
	text: &crate::text::Text,
	delta: crate::textm::TextDelta,
) {
	field_set_with(st, node, text, Some(delta));
}

fn field_set_with(
	st: &mut crate::style::St,
	node: u32,
	text: &crate::text::Text,
	delta: Option<crate::textm::TextDelta>,
) {
	let lineage = st
		.text_rev
		.entry(node)
		.or_insert(FieldTextRev { rev: 0, delta: None });
	lineage.rev = lineage.rev.wrapping_add(1);
	lineage.delta = delta;
	if let Some(index) = st
		.field_node
		.iter()
		.position(|&candidate| candidate == node)
	{
		st.field_text[index] = text.clone();
		return;
	}
	st.field_node.push(node);
	st.field_text.push(text.clone());
}

/// Returns an editable field's horizontal text scroll, or zero when unset.
pub fn field_scroll_x(st: &crate::style::St, node: u32) -> f64 {
	st.field_scroll_node
		.iter()
		.zip(&st.field_scroll_off)
		.find_map(|(&candidate, &offset)| (candidate == node).then_some(offset))
		.unwrap_or(0.0)
}

/// Sets an editable field's non-negative horizontal text scroll.
pub fn field_scroll_set(st: &mut crate::style::St, node: u32, x: f64) {
	let next = 0.0f64.max(x);
	if let Some(index) = st
		.field_scroll_node
		.iter()
		.position(|&candidate| candidate == node)
	{
		st.field_scroll_off[index] = next;
		return;
	}
	if next != 0.0 {
		st.field_scroll_node.push(node);
		st.field_scroll_off.push(next);
	}
}

/// Value tag whose tuple offset and length index [`St::mo_f`].
pub const T_OV_TUPLE: u32 = 100u32;

fn index_u32(index: u32) -> usize {
	usize::try_from(index).expect("index exceeds usize")
}

fn index_i32(index: i32) -> usize {
	usize::try_from(index).expect("negative index")
}

fn f64_to_u32(value: f64) -> u32 {
	if value.is_nan() || value <= 0.0 {
		return 0;
	}
	if value >= 4_294_967_295.0 {
		return u32::MAX;
	}
	let bits = value.to_bits();
	let exponent = i32::try_from((bits >> 52) & 0x7ff).expect("f64 exponent exceeds i32") - 1023;
	if exponent < 0 {
		return 0;
	}
	let significand = (bits & 0x000f_ffff_ffff_ffff) | (1_u64 << 52);
	let integer = if exponent >= 52 {
		significand << u32::try_from(exponent - 52).expect("negative shift")
	} else {
		significand >> u32::try_from(52 - exponent).expect("negative shift")
	};
	u32::try_from(integer).expect("clamped f64 exceeds u32")
}

fn f64_to_i32(value: f64) -> i32 {
	if value.is_nan() {
		return 0;
	}
	if value >= 2_147_483_647.0 {
		return i32::MAX;
	}
	if value <= -2_147_483_648.0 {
		return i32::MIN;
	}
	if value < 0.0 {
		-i32::try_from(f64_to_u32(-value)).expect("negative f64 magnitude exceeds i32")
	} else {
		i32::try_from(f64_to_u32(value)).expect("positive f64 exceeds i32")
	}
}

/// Appends one motion overlay entry; the last matching write wins.
pub fn ov_push(
	st: &mut crate::style::St,
	node: u32,
	attr: u32,
	tag: u32,
	num: f64,
	h: u32,
	off: i32,
	ln: i32,
) {
	st.mo_node.push(node);
	st.mo_attr.push(attr);
	st.mo_tag.push(tag);
	st.mo_num.push(num);
	st.mo_h.push(h);
	st.mo_off.push(off);
	st.mo_ln.push(ln);
	st.mo_index.insert((node, attr), st.mo_node.len() - 1);
	let node_index = index_u32(node);
	if st.mo_node_has.len() <= node_index {
		st.mo_node_has.resize(node_index + 1, false);
	}
	st.mo_node_has[node_index] = true;
}

/// Rebuilds the theme-resolved decoded-value cache for every AVAL entry.
///
/// Attribute reads resolve values through this cache instead of re-decoding
/// and re-following token references per lookup. Runs on document init and
/// theme change; crate-internal tests that edit a bound document's value
/// pools in place must call it before the next attribute read.
pub(crate) fn rebuild_aval_cache(d: &crate::slir::Doc, st: &mut crate::style::St) {
	st.aval_active.clear();
	st.aval_active.extend((0..d.aval_tag.len()).map(|ix| {
		crate::value::decode_active(d, st.theme_index, i32::try_from(ix).expect("aval index"))
	}));
}

/// Returns the theme-resolved decoded value for AVAL entry `ix`.
///
/// Negative and out-of-range indices read as missing, matching
/// [`crate::value::decode_active`].
#[inline]
pub fn aval_active(d: &crate::slir::Doc, st: &crate::style::St, ix: i32) -> crate::value::V {
	let Ok(index) = usize::try_from(ix) else {
		return crate::value::missing();
	};
	match st.aval_active.get(index) {
		Some(v) => *v,
		// A cache rebuilt from the same document covers every entry; the
		// fallback keeps host-supplied stale indices well-defined.
		None => crate::value::decode_active(d, st.theme_index, ix),
	}
}

/// Reads a tuple element from the document, the motion overlay, or a
/// dynamic tuple (literal members and current num/pct param values).
pub fn tup_at(d: &crate::slir::Doc, st: &crate::style::St, v: &crate::value::V, k: i32) -> f64 {
	if v.tag == crate::style::T_OV_TUPLE {
		if (k < 0i32) || (k >= v.ln) {
			return 0.0f64;
		}
		return st.mo_f[index_i32(v.off.wrapping_add(k))];
	}
	if v.tag == crate::slir::T_TUPLE_DYN {
		if (k < 0i32) || (k >= v.ln) {
			return 0.0f64;
		}
		let member = index_i32(v.off.wrapping_add(k));
		if d.tup_dyn_tag[member] == 1u32 {
			return st.params.number(index_u32(d.tup_dyn_param[member]));
		}
		return d.tup_dyn_num[member];
	}
	crate::value::tuple_at(d, v, k)
}

/// Reports whether a value tag is any tuple variant readable by [`tup_at`].
pub const fn is_tuple_v(tag: u32) -> bool {
	(tag == crate::slir::T_TUPLE)
		|| (tag == crate::style::T_OV_TUPLE)
		|| (tag == crate::slir::T_TUPLE_DYN)
}

/// Reports whether a compiled patch is active for a real or synthetic node.
///
/// State, property, and size conditions on synthetic list items are evaluated
/// per item.
pub fn patch_on_for(d: &crate::slir::Doc, st: &crate::style::St, pi: i32, node: u32) -> bool {
	let patch = index_i32(pi);
	if crate::list::each_of(&st.lists, d, node) == crate::slir::NONE {
		return st.patch_on[patch];
	}

	let condition = i32::try_from(d.patch_cond[patch]).expect("condition index exceeds i32");
	let kind = d.cond_kind[index_i32(condition)];
	if kind == crate::slir::C_WCMP || kind == crate::slir::C_HCMP {
		return st
			.wh_node
			.iter()
			.zip(&st.wh_patch)
			.zip(&st.wh_on)
			.find_map(|((&candidate, &patch), &on)| (candidate == node && patch == pi).then_some(on))
			.unwrap_or(false);
	}

	crate::when::eval_cond_item(
		d,
		condition,
		node,
		&st.env,
		&st.states,
		&st.ns_node,
		&st.ns_sym,
		&st.params,
		&st.lists,
		0.0,
		0.0,
	)
}

fn apply_effective_patch(d: &crate::slir::Doc, st: &mut crate::style::St, patch: usize) {
	let start = index_i32(d.patch_attr_off[patch]);
	let end = index_i32(d.patch_attr_off[patch].wrapping_add(d.patch_attr_len[patch]));
	for attr_index in start..end {
		let attr = index_u32(d.wattr_id[attr_index]);
		if let Some(value) = st.effective_attr_values.get_mut(attr) {
			*value = i32::from_ne_bytes(d.wattr_val[attr_index].to_ne_bytes());
		}
	}
}

fn prepare_attrs(d: &crate::slir::Doc, st: &mut crate::style::St, node: u32) -> u32 {
	let base = crate::list::base(&st.lists, d, node);
	let base_index = index_u32(base);
	st.effective_attr_node = crate::slir::NONE;
	st.effective_attr_values = st
		.base_attr_values
		.get(base_index)
		.copied()
		.unwrap_or_else(|| authored_attr_values(d, base_index));

	if base_index < st.lists.patches_by_node.len() {
		let patch_count = st.lists.patches_by_node[base_index].len();
		for patch_position in 0..patch_count {
			let patch = st.lists.patches_by_node[base_index][patch_position];
			let patch_i32 = i32::try_from(patch).expect("patch index exceeds i32");
			if patch_on_for(d, st, patch_i32, node) {
				apply_effective_patch(d, st, patch);
			}
		}
	} else {
		for (patch, &patch_node) in d.patch_node.iter().enumerate() {
			let patch_i32 = i32::try_from(patch).expect("patch index exceeds i32");
			if patch_node == base && patch_on_for(d, st, patch_i32, node) {
				apply_effective_patch(d, st, patch);
			}
		}
	}
	st.effective_attr_node = node;
	base
}

// Returns the last matching value within one active patch.
#[inline]
fn patch_attr(
	d: &crate::slir::Doc,
	st: &crate::style::St,
	patch: usize,
	node: u32,
	attr: u32,
) -> Option<i32> {
	let patch_i32 = i32::try_from(patch).expect("patch index exceeds i32");
	if !patch_on_for(d, st, patch_i32, node) {
		return None;
	}
	let start = index_i32(d.patch_attr_off[patch]);
	let end = index_i32(d.patch_attr_off[patch].wrapping_add(d.patch_attr_len[patch]));
	d.wattr_id[start..end]
		.iter()
		.zip(&d.wattr_val[start..end])
		.rev()
		.find_map(|(&entry_attr, &encoded)| {
			(entry_attr == attr).then(|| i32::from_ne_bytes(encoded.to_ne_bytes()))
		})
}

#[inline(never)]
fn attr_ix_slow(d: &crate::slir::Doc, st: &crate::style::St, node: u32, attr: u32) -> i32 {
	let base = crate::list::base(&st.lists, d, node);
	if base == crate::slir::NONE {
		return -1;
	}
	let base_index = index_u32(base);
	let mut value = st
		.base_attr_values
		.get(base_index)
		.and_then(|values| values.get(index_u32(attr)))
		.copied()
		.unwrap_or_else(|| crate::slir::base_attr(d, base, attr));

	if let Some(patches) = st.lists.patches_by_node.get(index_u32(base)) {
		for &patch in patches {
			if let Some(patch_value) = patch_attr(d, st, patch, node, attr) {
				value = patch_value;
			}
		}
	} else {
		for (patch, &patch_node) in d.patch_node.iter().enumerate() {
			if patch_node == base
				&& let Some(patch_value) = patch_attr(d, st, patch, node, attr)
			{
				value = patch_value;
			}
		}
	}
	value
}

/// Finds the winning encoded value for an attribute.
///
/// Active patches are visited in document order and later declarations replace
/// earlier declarations, preserving the cascade's last-wins rule.
#[inline(always)]
pub fn attr_ix(d: &crate::slir::Doc, st: &crate::style::St, node: u32, attr: u32) -> i32 {
	if st.effective_attr_node == node {
		return st
			.effective_attr_values
			.get(index_u32(attr))
			.copied()
			.unwrap_or(-1);
	}
	attr_ix_slow(d, st, node, attr)
}

#[inline(never)]
fn overlay_present(st: &crate::style::St, node: u32, attr: u32) -> crate::value::V {
	let Some(&index) = st.mo_index.get(&(node, attr)) else {
		return crate::value::missing();
	};
	crate::value::V {
		tag: st.mo_tag[index],
		num: st.mo_num[index],
		h:   st.mo_h[index],
		off: st.mo_off[index],
		ln:  st.mo_ln[index],
	}
}

/// Returns the last motion overlay value for a node attribute.
#[inline(always)]
pub fn overlay_val(st: &crate::style::St, node: u32, attr: u32) -> crate::value::V {
	if st
		.mo_node_has
		.get(index_u32(node))
		.copied()
		.unwrap_or(false)
	{
		overlay_present(st, node, attr)
	} else {
		crate::value::missing()
	}
}

const fn resolved_value(tag: u32, num: f64, h: u32) -> crate::value::V {
	crate::value::V { tag, num, h, off: 0, ln: 0 }
}

#[inline(never)]
fn resolve_attr_ref(
	d: &crate::slir::Doc,
	st: &crate::style::St,
	node: u32,
	v: crate::value::V,
) -> crate::value::V {
	if v.tag == crate::slir::T_PARAM_REF {
		let parameter = index_u32(v.h);
		return match d.parm_type[parameter] {
			crate::slir::PARAM_NUM => {
				resolved_value(crate::slir::T_NUM, st.params.number(parameter), 0)
			},
			crate::slir::PARAM_PCT => {
				resolved_value(crate::slir::T_PCT, st.params.number(parameter), 0)
			},
			crate::slir::PARAM_COLOR => {
				resolved_value(crate::slir::T_COLOR, 0.0, st.params.color(parameter))
			},
			crate::slir::PARAM_BOOL => {
				resolved_value(crate::slir::T_NUM, f64::from(u8::from(st.params.boolean(parameter))), 0)
			},
			_ => v,
		};
	}
	if v.tag == crate::slir::T_PROP_REF {
		let parameter = u32::from_ne_bytes(crate::list::param_of(&st.lists, d, node).to_ne_bytes());
		let item = crate::list::item_ix(&st.lists, d, node);
		let value = crate::list::get_ref(&st.lists, parameter, item, v.h);
		return match value {
			crate::list::ValueRef::Num(value) => resolved_value(crate::slir::T_NUM, value, 0),
			crate::list::ValueRef::Pct(value) => resolved_value(crate::slir::T_PCT, value, 0),
			crate::list::ValueRef::Color(value) => resolved_value(crate::slir::T_COLOR, 0.0, value),
			crate::list::ValueRef::Bool(value) => {
				resolved_value(crate::slir::T_NUM, f64::from(u8::from(value)), 0)
			},
			crate::list::ValueRef::Missing
			| crate::list::ValueRef::Text(_)
			| crate::list::ValueRef::Enum(_) => v,
		};
	}
	v
}

/// Decodes an attribute and substitutes numeric, color, and list properties.
///
/// Motion inputs supersede patches and base values and are already parameter
/// resolved. Enum and text references are resolved by [`attr_enum`] and
/// [`content_str`] because they use separate string channels.
#[inline(always)]
pub fn attr_val(
	d: &crate::slir::Doc,
	st: &crate::style::St,
	node: u32,
	attr: u32,
) -> crate::value::V {
	let motion = crate::style::overlay_val(st, node, attr);
	if motion.tag != crate::value::V_MISSING {
		return motion;
	}
	let value = crate::style::aval_active(d, st, crate::style::attr_ix(d, st, node, attr));
	if matches!(value.tag, crate::slir::T_PARAM_REF | crate::slir::T_PROP_REF) {
		resolve_attr_ref(d, st, node, value)
	} else {
		value
	}
}

/// Returns a retained split-pane extent by canonical key.
pub fn split_get(st: &crate::style::St, key: &str) -> Option<f64> {
	st.split_key
		.iter()
		.position(|candidate| candidate == key)
		.map(|index| st.split_extent[index])
}

/// Writes a retained split-pane extent, reporting whether it changed.
pub fn split_set(st: &mut crate::style::St, key: &str, extent: f64) -> bool {
	if let Some(index) = st.split_key.iter().position(|candidate| candidate == key) {
		if (st.split_extent[index] - extent).abs() <= crate::layout::EPS {
			return false;
		}
		st.split_extent[index] = extent;
		return true;
	}
	st.split_key.push(key.to_owned());
	st.split_extent.push(extent);
	true
}

/// Resolves the current selection highlight paint for a `select` root.
///
/// The default is `#3B82F640`. An explicit `none` remains a transparent solid
/// so authoring it suppresses the default rather than falling back to it.
pub fn select_paint(d: &crate::slir::Doc, st: &crate::style::St, node: u32) -> (u32, u32) {
	let value = attr_val(d, st, node, crate::slir::A_SELECT_BG);
	match value.tag {
		crate::slir::T_COLOR | crate::slir::T_PAINT_SOLID => (1, value.h),
		crate::slir::T_PAINT_GRADIENT => (2, value.h),
		crate::slir::T_PAINT_NONE => (1, 0),
		_ => (1, 0x40f6_823b),
	}
}
/// Returns a frame-runtime normalized verb stream for a negative path
/// reference.
pub fn runtime_path_verbs(st: &crate::style::St, path: i32) -> Option<&[u8]> {
	if path >= 0 || path == crate::style::PATH_NONE {
		return None;
	}
	st.rt_path_verbs
		.get(usize::try_from(!path).ok()?)
		.map(Vec::as_slice)
}

/// Returns the normalized coordinate stream selected by a compiled or runtime
/// path reference.
pub fn path_coords<'a>(
	d: &'a crate::slir::Doc,
	st: &'a crate::style::St,
	path: i32,
) -> Option<&'a [f64]> {
	if path == crate::style::PATH_NONE {
		return None;
	}
	if path < 0 {
		return st
			.rt_path_coords
			.get(usize::try_from(!path).ok()?)
			.map(Vec::as_slice);
	}
	let index = usize::try_from(path).ok()?;
	let offset = usize::try_from(*d.path_coord_off.get(index)?).ok()?;
	let length = usize::try_from(*d.path_coord_len.get(index)?).ok()?;
	d.path_coords.get(offset..offset.checked_add(length)?)
}

fn resolve_runtime_path(st: &mut crate::style::St, text: &str, line: u32) -> i32 {
	if let Some(&index) = st.rt_path_ix.get(text) {
		return !i32::try_from(index).expect("runtime path index exceeds i32");
	}
	if st.rt_path_bad.contains(text) {
		return crate::style::PATH_NONE;
	}
	let Some((verbs, coords)) = crate::pathdata::normalize(text) else {
		if st.rt_path_bad.insert(text.to_owned()) {
			crate::style::warn(st, "attr", &format!("invalid runtime path data '{text}'"), line);
		}
		return crate::style::PATH_NONE;
	};
	let index = u32::try_from(st.rt_path_verbs.len()).expect("runtime path pool exceeds u32");
	st.rt_path_ix.insert(text.to_owned(), index);
	st.rt_path_verbs.push(verbs);
	st.rt_path_coords.push(coords);
	!i32::try_from(index).expect("runtime path index exceeds i32")
}

/// Returns a numeric attribute or `dflt` when the value is not numeric.
pub fn attr_num(
	d: &crate::slir::Doc,
	st: &crate::style::St,
	node: u32,
	attr: u32,
	dflt: f64,
) -> f64 {
	crate::value::num_of(&crate::style::attr_val(d, st, node, attr), dflt)
}

/// Returns an enum keyword, or an empty string when absent or not an enum.
pub fn attr_enum(d: &crate::slir::Doc, st: &crate::style::St, node: u32, attr: u32) -> String {
	crate::style::attr_enum_ref(d, st, node, attr).into_owned()
}

/// Borrows an enum keyword, or an empty string when absent or not an enum.
pub fn attr_enum_ref<'a>(
	d: &'a crate::slir::Doc,
	st: &'a crate::style::St,
	node: u32,
	attr: u32,
) -> std::borrow::Cow<'a, str> {
	let v = crate::style::attr_val(d, st, node, attr);
	if v.tag == crate::slir::T_ENUM_SYM {
		return std::borrow::Cow::Borrowed(crate::slir::str_ref(d, v.h));
	}
	if v.tag == crate::slir::T_PARAM_REF && d.parm_type[index_u32(v.h)] == 5 {
		return std::borrow::Cow::Borrowed(st.params.symbol(index_u32(v.h)));
	}
	if v.tag == crate::slir::T_PROP_REF {
		let x = crate::list::get_ref(
			&st.lists,
			u32::from_ne_bytes(crate::list::param_of(&st.lists, d, node).to_ne_bytes()),
			crate::list::item_ix(&st.lists, d, node),
			v.h,
		);
		if let crate::list::ValueRef::Enum(value) = x {
			return std::borrow::Cow::Borrowed(value);
		}
	}
	std::borrow::Cow::Borrowed("")
}

/// Returns a string attribute, or an empty string when absent or not a string.
pub fn attr_str(d: &crate::slir::Doc, st: &crate::style::St, node: u32, attr: u32) -> String {
	crate::style::attr_str_ref(d, st, node, attr).into_owned()
}

/// Borrows a string attribute, or an empty string when absent or not a string.
pub fn attr_str_ref<'a>(
	d: &'a crate::slir::Doc,
	st: &'a crate::style::St,
	node: u32,
	attr: u32,
) -> std::borrow::Cow<'a, str> {
	let v = crate::style::attr_val(d, st, node, attr);
	if v.tag == crate::slir::T_STR {
		return std::borrow::Cow::Borrowed(crate::slir::str_ref(d, v.h));
	}
	if v.tag == crate::slir::T_PARAM_REF && d.parm_type[index_u32(v.h)] == 0 {
		return std::borrow::Cow::Borrowed(st.params.text(index_u32(v.h)));
	}
	if v.tag == crate::slir::T_PROP_REF {
		let x = crate::list::get_ref(
			&st.lists,
			u32::from_ne_bytes(crate::list::param_of(&st.lists, d, node).to_ne_bytes()),
			crate::list::item_ix(&st.lists, d, node),
			v.h,
		);
		if let crate::list::ValueRef::Text(value) = x {
			return std::borrow::Cow::Borrowed(value);
		}
	}
	std::borrow::Cow::Borrowed("")
}

/// Resolves one authored image name against the runtime table before compiled
/// sources.
fn resolve_image(d: &crate::slir::Doc, st: &mut crate::style::St, node: u32, line: u32) -> i32 {
	let name = crate::style::attr_str(d, st, node, crate::slir::A_SRC);
	let runtime = st
		.runtime_images
		.iter()
		.enumerate()
		.rposition(|(_, image)| image.active && image.name == name)
		.map(|runtime_index| {
			d.img_src
				.len()
				.checked_add(runtime_index)
				.and_then(|index| i32::try_from(index).ok())
				.expect("image index exceeds i32")
		});
	let compiled = d
		.img_src
		.iter()
		.rposition(|&source| d.strs[index_u32(source)] == name)
		.map(|index| i32::try_from(index).expect("image index exceeds i32"));
	let image = runtime.or(compiled).unwrap_or(-1);
	if image < 0 && !name.is_empty() && st.img_missing.insert(name.clone()) {
		crate::style::warn(
			st,
			"img-missing",
			&format!("image source '{name}' is not registered or compiled"),
			line,
		);
	}
	image
}

fn intern_scene_str(st: &mut crate::style::St, value: String) -> u32 {
	if value.is_empty() {
		return 0;
	}
	if let Some(&index) = st.scene_str_index.get(&value) {
		return index;
	}
	let index = u32::try_from(st.scene_strs.len()).expect("scene string pool exceeds u32");
	st.scene_strs.push(value.clone());
	st.scene_str_index.insert(value, index);
	index
}

fn a11y_ref(
	d: &crate::slir::Doc,
	st: &mut crate::style::St,
	node: u32,
	attr: u32,
	allow_enum: bool,
) -> u32 {
	let mut value = if allow_enum {
		crate::style::attr_enum(d, st, node, attr)
	} else {
		String::new()
	};
	if value.is_empty() {
		value = crate::style::attr_str(d, st, node, attr);
	}
	intern_scene_str(st, value)
}

fn semantic_bool_code(d: &crate::slir::Doc, st: &crate::style::St, node: u32, attr: u32) -> u32 {
	let value = crate::style::attr_val(d, st, node, attr);
	if value.tag != crate::slir::T_NUM {
		return 0;
	}
	if value.num == 0.0 { 1 } else { 2 }
}

fn checked_code(d: &crate::slir::Doc, st: &crate::style::St, node: u32) -> u32 {
	let boolean = semantic_bool_code(d, st, node, crate::slir::A_CHECKED);
	if boolean != 0 {
		return boolean;
	}
	let mut value = crate::style::attr_enum_ref(d, st, node, crate::slir::A_CHECKED);
	if value.is_empty() {
		value = crate::style::attr_str_ref(d, st, node, crate::slir::A_CHECKED);
	}
	if crate::rt::str_eq(&value, "false") {
		1
	} else if crate::rt::str_eq(&value, "true") {
		2
	} else if crate::rt::str_eq(&value, "mixed") {
		3
	} else {
		0
	}
}

fn live_code(d: &crate::slir::Doc, st: &crate::style::St, node: u32) -> u32 {
	let mut value = crate::style::attr_enum_ref(d, st, node, crate::slir::A_LIVE);
	if value.is_empty() {
		value = crate::style::attr_str_ref(d, st, node, crate::slir::A_LIVE);
	}
	if crate::rt::str_eq(&value, "off") {
		1
	} else if crate::rt::str_eq(&value, "polite") {
		2
	} else if crate::rt::str_eq(&value, "assertive") {
		3
	} else {
		0
	}
}

fn semantic_number(
	d: &crate::slir::Doc,
	st: &crate::style::St,
	node: u32,
	attr: u32,
) -> Option<f64> {
	let value = crate::style::attr_val(d, st, node, attr);
	if value.tag != crate::slir::T_NUM || !value.num.is_finite() {
		return None;
	}
	let valid = if matches!(attr, crate::slir::A_LEVEL | crate::slir::A_POS_IN_SET) {
		value.num >= 1.0 && value.num.fract() == 0.0
	} else if attr == crate::slir::A_SET_SIZE {
		value.num == -1.0 || (value.num >= 1.0 && value.num.fract() == 0.0)
	} else {
		true
	};
	valid.then_some(value.num)
}

/// Resolves text content like [`content_str`], as shared codepoint text.
///
/// The editable-field branch returns the retained buffer by reference count,
/// so re-solving a large field never copies its content.
pub fn content_text(d: &crate::slir::Doc, st: &crate::style::St, node: u32) -> crate::text::Text {
	let mv = crate::style::overlay_val(st, node, crate::slir::A_CONTENT);
	if mv.tag == crate::slir::T_STR {
		return crate::text::Text::from(crate::slir::str_at(d, mv.h));
	}
	if mv.tag == crate::slir::T_PARAM_REF && d.parm_type[index_u32(mv.h)] == 0 {
		return crate::text::Text::from(st.params.text(index_u32(mv.h)));
	}
	if let Some(index) = st
		.field_node
		.iter()
		.position(|&candidate| candidate == node)
	{
		return st.field_text[index].clone();
	}
	let v = crate::value::decode_active(
		d,
		st.theme_index,
		crate::style::attr_ix(d, st, node, crate::slir::A_CONTENT),
	);
	if v.tag == crate::slir::T_STR {
		return crate::text::Text::from(crate::slir::str_at(d, v.h));
	}
	if v.tag == crate::slir::T_PARAM_REF && d.parm_type[index_u32(v.h)] == 0 {
		return crate::text::Text::from(st.params.text(index_u32(v.h)));
	}
	if v.tag == crate::slir::T_PROP_REF {
		let x = crate::list::get_ref(
			&st.lists,
			u32::from_ne_bytes(crate::list::param_of(&st.lists, d, node).to_ne_bytes()),
			crate::list::item_ix(&st.lists, d, node),
			v.h,
		);
		if let crate::list::ValueRef::Text(value) = x {
			return crate::text::Text::from(value);
		}
	}
	crate::text::Text::default()
}

/// Resolves text content in motion, edit override, patch, then base order.
pub fn content_str(d: &crate::slir::Doc, st: &crate::style::St, node: u32) -> String {
	let mv = crate::style::overlay_val(st, node, crate::slir::A_CONTENT);
	if mv.tag == crate::slir::T_STR {
		return crate::slir::str_at(d, mv.h).to_owned();
	}
	if mv.tag == crate::slir::T_PARAM_REF && d.parm_type[index_u32(mv.h)] == 0 {
		return st.params.text(index_u32(mv.h)).to_owned();
	}
	if let Some(index) = st
		.field_node
		.iter()
		.position(|&candidate| candidate == node)
	{
		return st.field_text[index].to_utf8();
	}
	let v = crate::value::decode_active(
		d,
		st.theme_index,
		crate::style::attr_ix(d, st, node, crate::slir::A_CONTENT),
	);
	if v.tag == crate::slir::T_STR {
		return crate::slir::str_at(d, v.h).to_owned();
	}
	if v.tag == crate::slir::T_PARAM_REF && d.parm_type[index_u32(v.h)] == 0 {
		return st.params.text(index_u32(v.h)).to_owned();
	}
	if v.tag == crate::slir::T_PROP_REF {
		let x = crate::list::get_ref(
			&st.lists,
			u32::from_ne_bytes(crate::list::param_of(&st.lists, d, node).to_ne_bytes()),
			crate::list::item_ix(&st.lists, d, node),
			v.h,
		);
		if let crate::list::ValueRef::Text(value) = x {
			return value.to_owned();
		}
	}
	String::new()
}

/// Combines base node flags with every active patch's flag mask.
pub fn eff_flags(d: &crate::slir::Doc, st: &crate::style::St, node: u32) -> u32 {
	let base = crate::list::base(&st.lists, d, node);
	let mut flags = d.node_flags[index_u32(base)];
	let base_index = index_u32(base);
	let indexed = base_index < st.lists.patches_by_node.len();
	let patch_count = if indexed {
		st.lists.patches_by_node[base_index].len()
	} else {
		d.patch_node.len()
	};
	for position in 0..patch_count {
		let patch = if indexed {
			st.lists.patches_by_node[base_index][position]
		} else {
			position
		};
		if !indexed && d.patch_node[patch] != base {
			continue;
		}
		let patch_i32 = i32::try_from(patch).expect("patch index exceeds i32");
		if !crate::style::patch_on_for(d, st, patch_i32, node) {
			continue;
		}
		let start = index_i32(d.patch_attr_off[patch]);
		let end = index_i32(d.patch_attr_off[patch].wrapping_add(d.patch_attr_len[patch]));
		for (&entry_attr, &encoded) in d.wattr_id[start..end].iter().zip(&d.wattr_val[start..end]) {
			if entry_attr == crate::slir::A_FLAGS {
				flags |= f64_to_u32(crate::value::num_of(
					&crate::value::decode_active(
						d,
						st.theme_index,
						i32::from_ne_bytes(encoded.to_ne_bytes()),
					),
					0.0,
				));
			}
		}
	}
	flags
}

/// Reports whether every detached ancestor is selected by an active patch.
pub fn attached(d: &crate::slir::Doc, st: &crate::style::St, node: u32) -> bool {
	let first_base = crate::list::base(&st.lists, d, node);
	let mut materialized = first_base != crate::slir::NONE && node != first_base;
	let mut current = node;
	while current != crate::slir::NONE {
		let base = crate::list::base(&st.lists, d, current);
		if base == crate::slir::NONE {
			return false;
		}
		materialized |= current != base;
		let parent = crate::list::parent(&st.lists, d, current);
		if d.node_flags[index_u32(base)] & crate::slir::F_DETACHED != 0 {
			if parent == crate::slir::NONE {
				return false;
			}
			let parent_base = crate::list::base(&st.lists, d, parent);
			if !materialized {
				let selected = d.patch_node.iter().enumerate().any(|(patch, &owner)| {
					let patch_i32 = i32::try_from(patch).expect("patch index exceeds i32");
					owner == parent_base && patch_on_for(d, st, patch_i32, parent)
				});
				if !selected {
					return false;
				}
			}
		}
		current = parent;
	}
	true
}

/// Collects this solve's children in deterministic document order.
///
/// The base sibling chain contributes non-detached children first, followed by
/// active patches' detached children in patch document order.
pub fn children(d: &crate::slir::Doc, st: &mut crate::style::St, node: u32, out: &mut Vec<u32>) {
	out.clear();
	let base = crate::list::base(&st.lists, d, node);
	let base_index = index_u32(base);
	if d.node_kind[base_index] == crate::slir::K_EACH {
		let list = crate::list::each_list(d, &st.lists, node);
		if list < 0 {
			return;
		}
		let list = u32::try_from(list).expect("negative list handle");
		let range = if let Some((_, _, parent)) = crate::list::virtual_config(d, &st.lists, node) {
			let off = crate::style::scroll_get(st, parent);
			crate::list::materialized_window(d, &mut st.lists, node, off).unwrap_or((0, 0))
		} else {
			(0, crate::list::length(d, &st.lists, list))
		};
		let mut key = String::new();
		for item in range.0..range.1 {
			crate::list::key_at_into(d, &st.lists, list, item, &mut key);
			let key_hash = crate::list::identity_hash(&key);
			let mut template = crate::list::template_first(d, &st.lists, node);
			while template != crate::slir::NONE {
				out.push(crate::list::synthetic_hashed(&mut st.lists, node, template, &key, key_hash));
				template = d.node_next[index_u32(template)];
			}
		}
		return;
	}
	let each = crate::list::each_of(&st.lists, d, node);
	let mut child = d.node_first[base_index];
	while child != crate::slir::NONE {
		if d.node_flags[index_u32(child)] & crate::slir::F_DETACHED == 0 {
			if each == crate::slir::NONE {
				out.push(child);
			} else {
				out.push(crate::list::synthetic_from(&mut st.lists, node, each, child));
			}
		}
		child = d.node_next[index_u32(child)];
	}
	let indexed = base_index < st.lists.patches_by_node.len();
	let patch_count = if indexed {
		st.lists.patches_by_node[base_index].len()
	} else {
		d.patch_node.len()
	};
	for position in 0..patch_count {
		let patch = if indexed {
			st.lists.patches_by_node[base_index][position]
		} else {
			position
		};
		if !indexed && d.patch_node[patch] != base {
			continue;
		}
		let patch_i32 = i32::try_from(patch).expect("patch index exceeds i32");
		if !crate::style::patch_on_for(d, st, patch_i32, node) {
			continue;
		}
		let start = index_i32(d.patch_child_off[patch]);
		let end = index_i32(d.patch_child_off[patch].wrapping_add(d.patch_child_len[patch]));
		for &patch_child in &d.patch_children[start..end] {
			if each == crate::slir::NONE {
				out.push(patch_child);
			} else {
				out.push(crate::list::synthetic_from(&mut st.lists, node, each, patch_child));
			}
		}
	}
}

/// Derives a control's accessible name from its rendered descendant text.
///
/// Concatenates the resolved content of attached descendant text and para
/// nodes (string literals, params, item props, and edit overrides) in
/// document order, single-space separated with whitespace-only runs skipped.
/// `each` subtrees do not contribute: a collection's items are content, not
/// the collection control's name.
pub fn name_from_content(d: &crate::slir::Doc, st: &mut crate::style::St, root: u32) -> String {
	let mut name = String::new();
	let mut stack: Vec<u32> = Vec::new();
	let mut scratch: Vec<u32> = Vec::new();
	crate::style::children(d, st, root, &mut scratch);
	stack.extend(scratch.iter().rev());
	while let Some(node) = stack.pop() {
		let base = crate::list::base(&st.lists, d, node);
		if base == crate::slir::NONE {
			continue;
		}
		let kind = d.node_kind[index_u32(base)];
		if kind == crate::slir::K_EACH {
			continue;
		}
		if kind == crate::slir::K_TEXT || kind == crate::slir::K_PARA {
			let text = crate::style::content_str(d, st, node);
			let trimmed = text.trim();
			if !trimmed.is_empty() {
				if !name.is_empty() {
					name.push(' ');
				}
				name.push_str(trimmed);
			}
		}
		crate::style::children(d, st, node, &mut scratch);
		stack.extend(scratch.iter().rev());
	}
	name
}

/// Maps the unified alignment vocabulary to its stable integer codes.
///
/// Codes are start 0, center 1, end 2, baseline 3, stretch 4, top-start 5,
/// top 6, top-end 7, bottom-start 8, bottom 9, and bottom-end 10. Unknown
/// values map to -1.
pub fn align_code(s: &str) -> i32 {
	if crate::rt::str_eq(s, "start") {
		return 0i32;
	}
	if crate::rt::str_eq(s, "center") {
		return 1i32;
	}
	if crate::rt::str_eq(s, "end") {
		return 2i32;
	}
	if crate::rt::str_eq(s, "baseline") {
		return 3i32;
	}
	if crate::rt::str_eq(s, "stretch") {
		return 4i32;
	}
	if crate::rt::str_eq(s, "top-start") {
		return 5i32;
	}
	if crate::rt::str_eq(s, "top") {
		return 6i32;
	}
	if crate::rt::str_eq(s, "top-end") {
		return 7i32;
	}
	if crate::rt::str_eq(s, "bottom-start") {
		return 8i32;
	}
	if crate::rt::str_eq(s, "bottom") {
		return 9i32;
	}
	if crate::rt::str_eq(s, "bottom-end") {
		return 10i32;
	}
	-1i32
}

/// Returns whether a code belongs to the nine-position alignment vocabulary.
pub fn is_nine(code: i32) -> bool {
	(((code == 0i32) || (code == 1i32)) || (code == 2i32)) || (5i32..=10i32).contains(&code)
}

/// Returns the horizontal factor for a nine-position alignment code.
pub const fn nine_fx(code: i32) -> f64 {
	if ((code == 6i32) || (code == 1i32)) || (code == 9i32) {
		return 0.5f64;
	}
	if ((code == 7i32) || (code == 2i32)) || (code == 10i32) {
		return 1.0f64;
	}
	0.0f64
}

/// Returns the vertical factor for a nine-position alignment code.
pub fn nine_fy(code: i32) -> f64 {
	if ((code == 0i32) || (code == 1i32)) || (code == 2i32) {
		return 0.5f64;
	}
	if (8i32..=10i32).contains(&code) {
		return 1.0f64;
	}
	0.0f64
}

/// Returns the cross-axis factor: start/baseline/stretch map to zero, center
/// to one half, and end to one.
pub const fn cross_f(code: i32) -> f64 {
	if code == 1i32 {
		return 0.5f64;
	}
	if code == 2i32 {
		return 1.0f64;
	}
	0.0f64
}

/// Maps main-axis packing names to stable integer codes.
pub fn pack_code(s: &str) -> u32 {
	if crate::rt::str_eq(s, "center") {
		return 1u32;
	}
	if crate::rt::str_eq(s, "end") {
		return 2u32;
	}
	if crate::rt::str_eq(s, "between") {
		return 3u32;
	}
	0u32
}

/// Maps stroke alignment names to stable integer codes.
pub fn stroke_align_code(s: &str) -> u32 {
	if crate::rt::str_eq(s, "inside") {
		return 1u32;
	}
	if crate::rt::str_eq(s, "outside") {
		return 2u32;
	}
	0u32
}

/// Maps image fitting names to stable integer codes.
pub fn fit_code(s: &str) -> u32 {
	if crate::rt::str_eq(s, "contain") {
		return 1u32;
	}
	if crate::rt::str_eq(s, "stretch") {
		return 2u32;
	}
	0u32
}

/// Maps text alignment names to stable integer codes.
pub fn talign_code(s: &str) -> u32 {
	if crate::rt::str_eq(s, "center") {
		return 1u32;
	}
	if crate::rt::str_eq(s, "end") {
		return 2u32;
	}
	0u32
}

/// Maps scrollbar display names to stable integer codes.
pub fn scrollbar_code(s: &str) -> u32 {
	if crate::rt::str_eq(s, "auto") {
		return 1u32;
	}
	if crate::rt::str_eq(s, "always") {
		return 2u32;
	}
	0u32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Collision {
	None,
	Auto,
}

fn collision_of(value: &str) -> Collision {
	if value == "none" {
		Collision::None
	} else {
		Collision::Auto
	}
}

/// Precomputed keyword codes for one string-pool entry.
///
/// Built once per document by [`init_params`] from the keyword parsers above;
/// [`build_rstyle`] indexes it by pool reference instead of re-parsing keyword
/// strings per node per solve.
#[derive(Clone, Copy, Debug)]
struct KwCodes {
	pack:      u8,
	align:     i8,
	stroke:    u8,
	fit:       u8,
	talign:    u8,
	scrollbar: u8,
	gravity:   Gravity,
	collision: Collision,
}

fn kw_codes_of(s: &str) -> KwCodes {
	KwCodes {
		pack:      u8::try_from(crate::style::pack_code(s)).expect("pack code exceeds u8"),
		align:     i8::try_from(crate::style::align_code(s)).expect("align code exceeds i8"),
		stroke:    u8::try_from(crate::style::stroke_align_code(s))
			.expect("stroke-align code exceeds u8"),
		fit:       u8::try_from(crate::style::fit_code(s)).expect("fit code exceeds u8"),
		talign:    u8::try_from(crate::style::talign_code(s)).expect("talign code exceeds u8"),
		scrollbar: u8::try_from(crate::style::scrollbar_code(s)).expect("scrollbar code exceeds u8"),
		gravity:   gravity_of(s),
		collision: collision_of(s),
	}
}

/// Resolves a keyword attribute to its precomputed pool codes when possible.
///
/// Parameter- and property-sourced strings (and pool references beyond the
/// table, which cannot occur for a table built from the same document) fall
/// back to the keyword string for the parsers.
#[inline]
fn kw_at<'a>(
	d: &'a crate::slir::Doc,
	st: &'a crate::style::St,
	node: u32,
	attr: u32,
) -> Result<&'a KwCodes, &'a str> {
	let v = crate::style::attr_val(d, st, node, attr);
	if v.tag == crate::slir::T_ENUM_SYM {
		if let Some(kw) = st.kw_codes.get(index_u32(v.h)) {
			return Ok(kw);
		}
		return Err(crate::slir::str_ref(d, v.h));
	}
	if v.tag == crate::slir::T_PARAM_REF && d.parm_type[index_u32(v.h)] == 5 {
		return Err(st.params.symbol(index_u32(v.h)));
	}
	if v.tag == crate::slir::T_PROP_REF {
		let x = crate::list::get_ref(
			&st.lists,
			u32::from_ne_bytes(crate::list::param_of(&st.lists, d, node).to_ne_bytes()),
			crate::list::item_ix(&st.lists, d, node),
			v.h,
		);
		if let crate::list::ValueRef::Enum(value) = x {
			return Err(value);
		}
	}
	Err("")
}

/// Returns the stable diagnostic name for a node kind.
pub fn kind_name(kind: u32) -> String {
	if ((kind == crate::slir::K_ROW) || (kind == crate::slir::K_COL))
		|| (kind == crate::slir::K_GROUP)
	{
		return "box".to_string();
	}
	if kind == crate::slir::K_WRAP {
		return "wrap".to_string();
	}
	if kind == crate::slir::K_GRID {
		return "grid".to_string();
	}
	if kind == crate::slir::K_STACK {
		return "stack".to_string();
	}
	if kind == crate::slir::K_CANVAS {
		return "canvas".to_string();
	}
	if kind == crate::slir::K_PARA {
		return "para".to_string();
	}
	if kind == crate::slir::K_TEXT {
		return "text".to_string();
	}
	if kind == crate::slir::K_SPAN {
		return "span".to_string();
	}
	if kind == crate::slir::K_RECT {
		return "rect".to_string();
	}
	if kind == crate::slir::K_IMG {
		return "img".to_string();
	}
	if kind == crate::slir::K_PATH {
		return "path".to_string();
	}
	if kind == crate::slir::K_ICON {
		return "icon".to_string();
	}
	if kind == crate::slir::K_SPACER {
		return "spacer".to_string();
	}
	if kind == crate::slir::K_EACH {
		return "each".to_string();
	}
	"hole".to_string()
}

/// Formats a diagnostic label as `<kind>#<id>` or `<kind>`.
pub fn label(d: &crate::slir::Doc, st: &crate::style::St, node: u32) -> String {
	let b = crate::list::base(&st.lists, d, node);
	let base = index_u32(b);
	let kn = crate::style::kind_name(d.node_kind[base]);
	let id = d.node_id[base];
	if id != 0u32 {
		return crate::rt::str_concat(&crate::rt::str_concat(&kn, "#"), crate::slir::str_at(d, id));
	}
	kn
}

/// Intrinsic-content sizing.
pub const S_HUG: u32 = 0u32;

/// Fixed absolute sizing.
pub const S_FIXED: u32 = 1u32;

/// Flexible fill sizing.
pub const S_FILL: u32 = 2u32;

/// Parent-relative percentage sizing.
pub const S_PCT: u32 = 3u32;
/// Preferred side and alignment for an attached overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gravity {
	BelowStart,
	BelowCenter,
	BelowEnd,
	AboveStart,
	AboveCenter,
	AboveEnd,
	LeftStart,
	LeftCenter,
	LeftEnd,
	RightStart,
	RightCenter,
	RightEnd,
}

fn gravity_of(value: &str) -> Gravity {
	match value {
		"below-center" => Gravity::BelowCenter,
		"below-end" => Gravity::BelowEnd,
		"above-start" => Gravity::AboveStart,
		"above-center" => Gravity::AboveCenter,
		"above-end" => Gravity::AboveEnd,
		"left-start" => Gravity::LeftStart,
		"left-center" => Gravity::LeftCenter,
		"left-end" => Gravity::LeftEnd,
		"right-start" => Gravity::RightStart,
		"right-center" => Gravity::RightCenter,
		"right-end" => Gravity::RightEnd,
		_ => Gravity::BelowStart,
	}
}

/// Resolved accessibility semantics for one node.
///
/// Defined once and carried unchanged from [`RStyle`] through
/// [`crate::flatten::SceneNode`] into the retained scene, where exporters
/// (frame JSON, SDP `sem.node`, host accessibility trees) read it row-wise.
/// Kernel hit-testing and focus paths never touch these fields.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Semantics {
	pub role:              u32,
	/// Reference into [`St::scene_strs`] for the accessible label.
	pub label:             u32,
	/// Reference into [`St::scene_strs`] for the accessible description.
	pub desc:              u32,
	/// Optional checked state: 0 absent, 1 false, 2 true, 3 mixed.
	pub checked:           u32,
	/// Optional expanded state: 0 absent, 1 false, 2 true.
	pub expanded:          u32,
	/// Optional selected state: 0 absent, 1 false, 2 true.
	pub selected:          u32,
	/// Scene-string reference for the active descendant's full key.
	pub active_descendant: u32,
	/// Scene-string reference for the controlled node's full key.
	pub controls:          u32,
	/// Optional current range value.
	pub value_now:         Option<f64>,
	/// Optional minimum range value.
	pub value_min:         Option<f64>,
	/// Optional maximum range value.
	pub value_max:         Option<f64>,
	/// Scene-string reference for a human-readable range value.
	pub value_text:        u32,
	/// Optional modal state: 0 absent, 1 false, 2 true.
	pub modal:             u32,
	/// Optional live-region mode: 0 absent, 1 off, 2 polite, 3 assertive.
	pub live:              u32,
	/// Optional live-region atomicity: 0 absent, 1 false, 2 true.
	pub live_atomic:       u32,
	/// Optional semantic hierarchy level.
	pub level:             Option<f64>,
	/// Optional one-based position within a semantic set.
	pub pos_in_set:        Option<f64>,
	/// Optional semantic set size; -1 means unknown.
	pub set_size:          Option<f64>,
}

/// Fully resolved style for one node.
#[derive(Clone, Debug)]
pub struct RStyle {
	pub node:            u32,
	pub kind:            u32,
	pub line:            u32,
	pub flags:           u32,
	pub is_row:          bool,
	pub w_kind:          u32,
	pub w_v:             f64,
	pub h_kind:          u32,
	pub h_v:             f64,
	pub min_w:           f64,
	pub max_w:           f64,
	pub min_h:           f64,
	pub max_h:           f64,
	pub pad_t:           f64,
	pub pad_r:           f64,
	pub pad_b:           f64,
	pub pad_l:           f64,
	pub gap:             f64,
	pub gap_cross:       f64,
	pub has_gap_cross:   bool,
	pub pack:            u32,
	pub align:           i32,
	pub self_align:      i32,
	pub offset_x:        f64,
	pub offset_y:        f64,
	pub at_x:            f64,
	pub at_y:            f64,
	pub has_at:          bool,
	pub anchor:          i32,
	/// Whether the effective style contains `attach`, even when it resolves
	/// empty.
	pub has_attach:      bool,
	/// Runtime-resolved full scene key named by `attach`.
	pub attach:          String,
	/// Preferred side and alignment around the attachment target.
	pub gravity:         Gravity,
	/// Whether main-side flipping and alignment-axis sliding are enabled.
	pub collide_auto:    bool,
	pub rotate:          f64,
	/// Ink-only zoom factors about the node center; `1,1` means none.
	pub scale_x:         f64,
	pub scale_y:         f64,
	/// Ink-only 3D perspective tilt (degrees); active when either angle is
	/// nonzero.
	pub has_tilt:        bool,
	pub tilt_rx:         f64,
	pub tilt_ry:         f64,
	pub tilt_depth:      f64,
	pub bg_kind:         u32,
	pub bg_h:            u32,
	pub stroke_kind:     u32,
	pub stroke_h:        u32,
	pub stroke_w:        f64,
	pub stroke_align:    u32,
	pub stroke_sides:    u32,
	pub dash_on:         f64,
	pub dash_off:        f64,
	pub has_dash:        bool,
	pub radius:          f64,
	/// Figma-style corner smoothing 0..1; a no-op unless `radius > 0`.
	pub smooth:          f64,
	pub shadow_off:      i32,
	pub shadow_len:      i32,
	pub opacity:         f64,
	pub blur:            f64,
	/// Deterministic speckle overlay: amount 0..1 and speckle cell size in u.
	pub grain_amount:    f64,
	pub grain_size:      f64,
	/// Alpha fade mask over the border box: 0 none, 1 solid, 2 gradient.
	pub mask_kind:       u32,
	pub mask_h:          u32,
	pub has_backdrop:    bool,
	pub backdrop_blur:   f64,
	pub backdrop_sat:    f64,
	pub backdrop_bright: f64,
	/// Progressive-blur mask on the backdrop: 0 none, 1 solid, 2 gradient.
	pub bmask_kind:      u32,
	pub bmask_h:         u32,
	pub scrollbar:       u32,
	pub scrollbar_w:     f64,
	pub scrollbar_fg:    u32,
	pub scrollbar_bg:    u32,
	pub split_w:         f64,
	/// Active sash paint: 0 none, 1 solid, 2 gradient.
	pub split_fg_kind:   u32,
	pub split_fg:        u32,
	pub fam:             u32,
	pub font:            i32,
	pub size:            f64,
	pub weight:          f64,
	pub leading:         f64,
	pub tracking:        f64,
	pub strike:          bool,
	pub italic:          bool,
	pub underline:       bool,
	pub color:           u32,
	/// 1 when `color` is packed RGBA, 2 when it is a gradient handle.
	pub color_kind:      u32,
	/// Optional rich inline-code text paint; kind 0 inherits `color`.
	pub code_color:      u32,
	pub code_color_kind: u32,
	/// Optional rich inline-code background paint; kind 0 is none.
	pub code_bg:         u32,
	pub code_bg_kind:    u32,
	pub talign:          u32,
	pub content:         crate::text::Text,
	/// Resolved accessibility semantics copied into the scene entry.
	pub sem:             Semantics,
	pub img:             i32,
	pub fit:             u32,
	pub path:            i32,
	/// Minimum x coordinate in the selected path geometry.
	pub path_min_x:      f64,
	/// Minimum y coordinate in the selected path geometry.
	pub path_min_y:      f64,
	/// Index into the document icon declarations, or -1 when unresolved.
	pub icon:            i32,
	pub track_off:       i32,
	pub track_len:       i32,
	pub span:            i32,
}

/// Sentinel used for an unbounded maximum size.
pub const INF: f64 = 1e30f64;

/// Sentinel for a path node with no valid geometry.
pub const PATH_NONE: i32 = i32::MIN;

/// Resolves an encoded size value, applying the supplied context default.
pub fn size_of(v: &crate::value::V, dflt_kind: u32, dflt_v: f64) -> crate::style::Size {
	if (v.tag == crate::slir::T_SIZE_FIXED) || (v.tag == crate::slir::T_NUM) {
		return crate::style::Size { kind: crate::style::S_FIXED, v: v.num };
	}
	if v.tag == crate::slir::T_SIZE_HUG {
		return crate::style::Size { kind: crate::style::S_HUG, v: 0.0f64 };
	}
	if v.tag == crate::slir::T_SIZE_FILL {
		// A zero fill weight means the conventional default weight of one.
		if v.num == 0.0f64 {
			return crate::style::Size { kind: crate::style::S_FILL, v: 1.0f64 };
		}
		return crate::style::Size { kind: crate::style::S_FILL, v: v.num };
	}
	if (v.tag == crate::slir::T_SIZE_PCT) || (v.tag == crate::slir::T_PCT) {
		return crate::style::Size { kind: crate::style::S_PCT, v: v.num };
	}
	crate::style::Size { kind: dflt_kind, v: dflt_v }
}

/// A resolved size kind and its numeric value.
#[derive(Clone, Debug)]
pub struct Size {
	pub kind: u32,
	pub v:    f64,
}

/// Returns whether `kind` receives cross-axis stretch defaults.
///
/// Rectangles and divider handles are intentionally container-like, so an empty
/// styled box with a fixed main thickness stretches across its parent.
pub const fn is_container(kind: u32) -> bool {
	matches!(
		kind,
		crate::slir::K_ROW
			| crate::slir::K_COL
			| crate::slir::K_GROUP
			| crate::slir::K_WRAP
			| crate::slir::K_GRID
			| crate::slir::K_STACK
			| crate::slir::K_CANVAS
			| crate::slir::K_PARA
			| crate::slir::K_RECT
			| crate::slir::K_DIVIDER
	)
}

/// Clears cached family and weight matches after the document font table
/// changes.
pub(crate) fn invalidate_font_selection(st: &mut crate::style::St) {
	st.font_selection.clear();
	st.family_index.clear();
	// Font indices may alias different metrics after a table rebuild.
	st.text_layout_cache.clear();
	st.text_layout_cache_cold.clear();
}

fn cached_family(d: &crate::slir::Doc, st: &mut crate::style::St, name: &str) -> Option<u32> {
	if let Some(&family) = st.family_index.get(name) {
		return (family != crate::slir::NONE).then_some(family);
	}
	let family = d
		.strs
		.iter()
		.position(|candidate| crate::slir::family_eq(candidate, name))
		.map_or(crate::slir::NONE, |index| {
			u32::try_from(index).expect("family string index exceeds u32")
		});
	st.family_index.insert(name.to_owned(), family);
	(family != crate::slir::NONE).then_some(family)
}

fn cached_font(d: &crate::slir::Doc, st: &mut crate::style::St, family: u32, weight: u32) -> i32 {
	let family = index_u32(family);
	if st.font_selection.len() <= family {
		st.font_selection.resize_with(family + 1, Vec::new);
	}
	if let Some((_, font)) = st.font_selection[family]
		.iter()
		.find(|&&(cached_weight, _)| cached_weight == weight)
	{
		return *font;
	}
	let font =
		crate::slir::font_select(d, u32::try_from(family).expect("family index exceeds u32"), weight);
	st.font_selection[family].push((weight, font));
	font
}
/// Builds the resolved style for `node` and returns its pool index.
///
/// `parent_kind` is the layout parent's node kind; [`crate::slir::NONE`] marks
/// the root, whose context is a column. The `inh_*` arguments are the parent's
/// inherited text-style whitelist.
#[allow(
	clippy::fn_params_excessive_bools,
	reason = "style builder receives parent whitelist flags"
)]
pub fn build_rstyle(
	d: &crate::slir::Doc,
	st: &mut crate::style::St,
	node: u32,
	parent_kind: u32,
	parent_is_row: bool,
	inh_color: u32,
	inh_color_kind: u32,
	inh_fam: u32,
	inh_size: f64,
	inh_weight: f64,
	inh_leading: f64,
	inh_tracking: f64,
	inh_strike: bool,
	inh_italic: bool,
	inh_underline: bool,
) -> i32 {
	let b = prepare_attrs(d, st, node);
	let base = index_u32(b);
	let kind = d.node_kind[base];
	st.rs
		.push(crate::style::rstyle_default(node, kind, d.node_line[base]));
	let ri = st.rs.len() - 1;
	st.rs[ri].flags = crate::style::eff_flags(d, st, node);
	// Node kind supplies the axis default; an active axis attribute may
	// override it.
	let mut is_row = kind == crate::slir::K_ROW;
	let ax = crate::style::attr_enum_ref(d, st, node, crate::slir::A_AXIS);
	if crate::rt::str_eq(&ax, "row") {
		is_row = true;
	} else if crate::rt::str_eq(&ax, "col") {
		is_row = false;
	}
	st.rs[ri].is_row = is_row;
	// Main-axis sizing defaults to hug. Containers and holes stretch on the
	// cross axis, except within stacks and canvases; spacers fill the main axis.
	let in_layer = (parent_kind == crate::slir::K_STACK) || (parent_kind == crate::slir::K_CANVAS);
	let mut dw_kind = crate::style::S_HUG;
	let mut dw_v = 0.0f64;
	let mut dh_kind = crate::style::S_HUG;
	let mut dh_v = 0.0f64;
	if (crate::style::is_container(kind) || (kind == crate::slir::K_HOLE)) && (!in_layer) {
		if parent_is_row {
			dh_kind = crate::style::S_FILL;
			dh_v = 1.0f64;
		} else {
			dw_kind = crate::style::S_FILL;
			dw_v = 1.0f64;
		}
	}
	if kind == crate::slir::K_SPACER {
		if parent_is_row {
			dw_kind = crate::style::S_FILL;
			dw_v = 1.0f64;
		} else {
			dh_kind = crate::style::S_FILL;
			dh_v = 1.0f64;
		}
	}
	let ws =
		crate::style::size_of(&crate::style::attr_val(d, st, node, crate::slir::A_W), dw_kind, dw_v);
	st.rs[ri].w_kind = ws.kind;
	st.rs[ri].w_v = ws.v;
	let hs =
		crate::style::size_of(&crate::style::attr_val(d, st, node, crate::slir::A_H), dh_kind, dh_v);
	st.rs[ri].h_kind = hs.kind;
	st.rs[ri].h_v = hs.v;
	st.rs[ri].min_w = crate::style::attr_num(d, st, node, crate::slir::A_MIN_W, 0.0f64);
	st.rs[ri].max_w = crate::style::attr_num(d, st, node, crate::slir::A_MAX_W, crate::style::INF);
	st.rs[ri].min_h = crate::style::attr_num(d, st, node, crate::slir::A_MIN_H, 0.0f64);
	st.rs[ri].max_h = crate::style::attr_num(d, st, node, crate::slir::A_MAX_H, crate::style::INF);
	// Padding is compiled as a normalized (top, right, bottom, left) tuple.
	let pad = crate::style::attr_val(d, st, node, crate::slir::A_PAD);
	if crate::style::is_tuple_v(pad.tag) && (pad.ln == 4i32) {
		st.rs[ri].pad_t = crate::style::tup_at(d, st, &pad, 0i32);
		st.rs[ri].pad_r = crate::style::tup_at(d, st, &pad, 1i32);
		st.rs[ri].pad_b = crate::style::tup_at(d, st, &pad, 2i32);
		st.rs[ri].pad_l = crate::style::tup_at(d, st, &pad, 3i32);
	}
	// Per-side overrides apply after the tuple (and are prop-drivable).
	for (attr, side) in [
		(crate::slir::A_PAD_T, 0i32),
		(crate::slir::A_PAD_R, 1i32),
		(crate::slir::A_PAD_B, 2i32),
		(crate::slir::A_PAD_L, 3i32),
	] {
		if crate::style::attr_ix(d, st, node, attr) >= 0 {
			let value = crate::style::attr_num(d, st, node, attr, f64::NAN);
			match side {
				0 => st.rs[ri].pad_t = value,
				1 => st.rs[ri].pad_r = value,
				2 => st.rs[ri].pad_b = value,
				_ => st.rs[ri].pad_l = value,
			}
		}
	}
	// Gap is either one number or a (main, cross) tuple.
	let gap = crate::style::attr_val(d, st, node, crate::slir::A_GAP);
	if crate::style::is_tuple_v(gap.tag) {
		st.rs[ri].gap = crate::style::tup_at(d, st, &gap, 0i32);
		st.rs[ri].gap_cross = crate::style::tup_at(d, st, &gap, 1i32);
		st.rs[ri].has_gap_cross = true;
	} else {
		st.rs[ri].gap = crate::value::num_of(&gap, 0.0f64);
	}
	st.rs[ri].pack = match kw_at(d, st, node, crate::slir::A_PACK) {
		Ok(kw) => u32::from(kw.pack),
		Err(s) => crate::style::pack_code(s),
	};
	st.rs[ri].align = match kw_at(d, st, node, crate::slir::A_ALIGN) {
		Ok(kw) => i32::from(kw.align),
		Err(s) => crate::style::align_code(s),
	};
	st.rs[ri].self_align = match kw_at(d, st, node, crate::slir::A_SELF) {
		Ok(kw) => i32::from(kw.align),
		Err(s) => crate::style::align_code(s),
	};
	let off = crate::style::attr_val(d, st, node, crate::slir::A_OFFSET);
	if crate::style::is_tuple_v(off.tag) {
		st.rs[ri].offset_x = crate::style::tup_at(d, st, &off, 0i32);
		st.rs[ri].offset_y = crate::style::tup_at(d, st, &off, 1i32);
	}
	let at = crate::style::attr_val(d, st, node, crate::slir::A_AT);
	if crate::style::is_tuple_v(at.tag) && (at.ln >= 2i32) {
		st.rs[ri].at_x = crate::style::tup_at(d, st, &at, 0i32);
		st.rs[ri].at_y = crate::style::tup_at(d, st, &at, 1i32);
		st.rs[ri].has_at = true;
	}
	st.rs[ri].anchor = match kw_at(d, st, node, crate::slir::A_ANCHOR) {
		Ok(kw) => i32::from(kw.align),
		Err(s) => crate::style::align_code(s),
	};
	let has_attach = crate::style::attr_ix(d, st, node, crate::slir::A_ATTACH) >= 0;
	let attach = crate::style::attr_str(d, st, node, crate::slir::A_ATTACH);
	let gravity = match kw_at(d, st, node, crate::slir::A_GRAVITY) {
		Ok(kw) => kw.gravity,
		Err(s) => gravity_of(s),
	};
	let collision = match kw_at(d, st, node, crate::slir::A_COLLIDE) {
		Ok(kw) => kw.collision,
		Err(s) => collision_of(s),
	};
	st.rs[ri].has_attach = has_attach;
	st.rs[ri].attach = attach;
	st.rs[ri].gravity = gravity;
	st.rs[ri].collide_auto = collision == Collision::Auto;
	st.rs[ri].rotate = crate::style::attr_num(d, st, node, crate::slir::A_ROTATE, 0.0f64);
	// Ink-only transforms: scale is a factor or an (sx, sy) tuple; tilt is
	// rx or (rx, ry[, depth]) in degrees with depth in u (default 800).
	let sc = crate::style::attr_val(d, st, node, crate::slir::A_SCALE);
	if crate::style::is_tuple_v(sc.tag) && (sc.ln >= 2i32) {
		st.rs[ri].scale_x = crate::style::tup_at(d, st, &sc, 0i32);
		st.rs[ri].scale_y = crate::style::tup_at(d, st, &sc, 1i32);
	} else {
		let factor = crate::value::num_of(&sc, 1.0f64);
		st.rs[ri].scale_x = factor;
		st.rs[ri].scale_y = factor;
	}
	let tilt = crate::style::attr_val(d, st, node, crate::slir::A_TILT);
	if crate::style::is_tuple_v(tilt.tag) && (tilt.ln >= 2i32) {
		st.rs[ri].tilt_rx = crate::style::tup_at(d, st, &tilt, 0i32);
		st.rs[ri].tilt_ry = crate::style::tup_at(d, st, &tilt, 1i32);
		if tilt.ln >= 3i32 {
			let depth = crate::style::tup_at(d, st, &tilt, 2i32);
			if depth > 0.0f64 {
				st.rs[ri].tilt_depth = depth;
			}
		}
	} else {
		st.rs[ri].tilt_rx = crate::value::num_of(&tilt, 0.0f64);
	}
	st.rs[ri].has_tilt = (st.rs[ri].tilt_rx != 0.0f64) || (st.rs[ri].tilt_ry != 0.0f64);
	// Resolve paints after geometry and positioning.
	let bg = crate::style::attr_val(d, st, node, crate::slir::A_BG);
	if (bg.tag == crate::slir::T_PAINT_SOLID) || (bg.tag == crate::slir::T_COLOR) {
		st.rs[ri].bg_kind = 1u32;
		st.rs[ri].bg_h = bg.h;
	} else if bg.tag == crate::slir::T_PAINT_GRADIENT {
		st.rs[ri].bg_kind = 2u32;
		st.rs[ri].bg_h = bg.h;
	}
	let sk = crate::style::attr_val(d, st, node, crate::slir::A_STROKE);
	if (sk.tag == crate::slir::T_PAINT_SOLID) || (sk.tag == crate::slir::T_COLOR) {
		st.rs[ri].stroke_kind = 1u32;
		st.rs[ri].stroke_h = sk.h;
	} else if sk.tag == crate::slir::T_PAINT_GRADIENT {
		st.rs[ri].stroke_kind = 2u32;
		st.rs[ri].stroke_h = sk.h;
	}
	st.rs[ri].stroke_w = crate::style::attr_num(d, st, node, crate::slir::A_STROKE_W, 1.0f64);
	st.rs[ri].stroke_align = match kw_at(d, st, node, crate::slir::A_STROKE_ALIGN) {
		Ok(kw) => u32::from(kw.stroke),
		Err(s) => crate::style::stroke_align_code(s),
	};
	st.rs[ri].stroke_sides =
		f64_to_u32(crate::style::attr_num(d, st, node, crate::slir::A_STROKE_SIDES, 15.0));
	let dash = crate::style::attr_val(d, st, node, crate::slir::A_STROKE_DASH);
	if crate::style::is_tuple_v(dash.tag) && (dash.ln >= 2i32) {
		st.rs[ri].dash_on = crate::style::tup_at(d, st, &dash, 0i32);
		st.rs[ri].dash_off = crate::style::tup_at(d, st, &dash, 1i32);
		st.rs[ri].has_dash = true;
	}
	st.rs[ri].radius = crate::style::attr_num(d, st, node, crate::slir::A_RADIUS, 0.0f64);
	st.rs[ri].smooth =
		crate::style::attr_num(d, st, node, crate::slir::A_SMOOTH, 0.0f64).clamp(0.0f64, 1.0f64);
	let sh = crate::style::attr_val(d, st, node, crate::slir::A_SHADOW);
	if sh.tag == crate::slir::T_SHADOW_LIST {
		st.rs[ri].shadow_off = sh.off;
		st.rs[ri].shadow_len = sh.ln;
	}
	st.rs[ri].opacity = crate::style::attr_num(d, st, node, crate::slir::A_OPACITY, 1.0f64);
	st.rs[ri].blur = crate::style::attr_num(d, st, node, crate::slir::A_BLUR, 0.0f64);
	// Grain is compiled as an (amount, size) tuple; a bare number is amount.
	let gr = crate::style::attr_val(d, st, node, crate::slir::A_GRAIN);
	if crate::style::is_tuple_v(gr.tag) && (gr.ln >= 2i32) {
		st.rs[ri].grain_amount = crate::style::tup_at(d, st, &gr, 0i32).clamp(0.0f64, 1.0f64);
		st.rs[ri].grain_size = crate::style::tup_at(d, st, &gr, 1i32).max(0.01f64);
	} else {
		st.rs[ri].grain_amount = crate::value::num_of(&gr, 0.0f64).clamp(0.0f64, 1.0f64);
	}
	let mask = crate::style::attr_val(d, st, node, crate::slir::A_MASK);
	if (mask.tag == crate::slir::T_PAINT_SOLID) || (mask.tag == crate::slir::T_COLOR) {
		st.rs[ri].mask_kind = 1u32;
		st.rs[ri].mask_h = mask.h;
	} else if mask.tag == crate::slir::T_PAINT_GRADIENT {
		st.rs[ri].mask_kind = 2u32;
		st.rs[ri].mask_h = mask.h;
	}
	let bd = crate::style::attr_val(d, st, node, crate::slir::A_BACKDROP);
	if crate::style::is_tuple_v(bd.tag) && (bd.ln >= 2i32) {
		st.rs[ri].has_backdrop = true;
		st.rs[ri].backdrop_blur = crate::style::tup_at(d, st, &bd, 0i32);
		st.rs[ri].backdrop_sat = crate::style::tup_at(d, st, &bd, 1i32);
		if bd.ln >= 3i32 {
			st.rs[ri].backdrop_bright = crate::style::tup_at(d, st, &bd, 2i32);
		}
		let bmask = crate::style::attr_val(d, st, node, crate::slir::A_BACKDROP_MASK);
		if (bmask.tag == crate::slir::T_PAINT_SOLID) || (bmask.tag == crate::slir::T_COLOR) {
			st.rs[ri].bmask_kind = 1u32;
			st.rs[ri].bmask_h = bmask.h;
		} else if bmask.tag == crate::slir::T_PAINT_GRADIENT {
			st.rs[ri].bmask_kind = 2u32;
			st.rs[ri].bmask_h = bmask.h;
		}
	}
	st.rs[ri].scrollbar = match kw_at(d, st, node, crate::slir::A_SCROLLBAR) {
		Ok(kw) => u32::from(kw.scrollbar),
		Err(s) => crate::style::scrollbar_code(s),
	};
	st.rs[ri].scrollbar_w =
		(0.0f64).max(crate::style::attr_num(d, st, node, crate::slir::A_SCROLLBAR_W, 4.0f64));
	let scrollbar_fg = crate::style::attr_val(d, st, node, crate::slir::A_SCROLLBAR_FG);
	if scrollbar_fg.tag == crate::slir::T_COLOR {
		st.rs[ri].scrollbar_fg = scrollbar_fg.h;
	}
	let scrollbar_bg = crate::style::attr_val(d, st, node, crate::slir::A_SCROLLBAR_BG);
	if scrollbar_bg.tag == crate::slir::T_COLOR {
		st.rs[ri].scrollbar_bg = scrollbar_bg.h;
	}
	st.rs[ri].split_w =
		(0.0f64).max(crate::style::attr_num(d, st, node, crate::slir::A_SPLIT_W, 4.0f64));
	let split_fg = crate::style::attr_val(d, st, node, crate::slir::A_SPLIT_FG);
	match split_fg.tag {
		crate::slir::T_COLOR | crate::slir::T_PAINT_SOLID => {
			st.rs[ri].split_fg_kind = 1;
			st.rs[ri].split_fg = split_fg.h;
		},
		crate::slir::T_PAINT_GRADIENT => {
			st.rs[ri].split_fg_kind = 2;
			st.rs[ri].split_fg = split_fg.h;
		},
		_ => {},
	}
	// Only the text-style whitelist inherits from the parent.
	let family = crate::style::attr_val(d, st, node, crate::slir::A_FAMILY);
	let mut fam = inh_fam;
	let mut dynamic_family = String::new();
	let mut dynamic_family_uninterned = false;
	if family.tag == crate::slir::T_STR {
		fam = family.h;
	} else if matches!(family.tag, crate::slir::T_PARAM_REF | crate::slir::T_PROP_REF) {
		let name = crate::style::attr_str_ref(d, st, node, crate::slir::A_FAMILY);
		if let Some(&index) = st.family_index.get(name.as_ref()) {
			fam = index;
		} else {
			dynamic_family = name.into_owned();
			if let Some(index) = cached_family(d, st, &dynamic_family) {
				fam = index;
			} else {
				dynamic_family_uninterned = true;
			}
		}
	}
	st.rs[ri].fam = fam;
	st.rs[ri].size = crate::style::attr_num(d, st, node, crate::slir::A_SIZE, inh_size);
	st.rs[ri].weight = crate::style::attr_num(d, st, node, crate::slir::A_WEIGHT, inh_weight);
	st.rs[ri].leading = crate::style::attr_num(d, st, node, crate::slir::A_LEADING, inh_leading);
	st.rs[ri].tracking = crate::style::attr_num(d, st, node, crate::slir::A_TRACKING, inh_tracking);
	st.rs[ri].strike =
		crate::style::attr_num(d, st, node, crate::slir::A_STRIKE, f64::from(inh_strike)) != 0.0;
	st.rs[ri].italic =
		crate::style::attr_num(d, st, node, crate::slir::A_ITALIC, f64::from(inh_italic)) != 0.0;
	st.rs[ri].underline =
		crate::style::attr_num(d, st, node, crate::slir::A_UNDERLINE, f64::from(inh_underline))
			!= 0.0;
	let col = crate::style::attr_val(d, st, node, crate::slir::A_COLOR);
	if (col.tag == crate::slir::T_COLOR) || (col.tag == crate::slir::T_PAINT_SOLID) {
		st.rs[ri].color = col.h;
		st.rs[ri].color_kind = 1u32;
	} else if col.tag == crate::slir::T_PAINT_GRADIENT {
		st.rs[ri].color = col.h;
		st.rs[ri].color_kind = 2u32;
	} else {
		st.rs[ri].color = inh_color;
		st.rs[ri].color_kind = inh_color_kind;
	}
	let code_color = crate::style::attr_val(d, st, node, crate::slir::A_CODE_COLOR);
	if (code_color.tag == crate::slir::T_COLOR) || (code_color.tag == crate::slir::T_PAINT_SOLID) {
		st.rs[ri].code_color = code_color.h;
		st.rs[ri].code_color_kind = 1u32;
	} else if code_color.tag == crate::slir::T_PAINT_GRADIENT {
		st.rs[ri].code_color = code_color.h;
		st.rs[ri].code_color_kind = 2u32;
	}
	let code_bg = crate::style::attr_val(d, st, node, crate::slir::A_CODE_BG);
	if (code_bg.tag == crate::slir::T_COLOR) || (code_bg.tag == crate::slir::T_PAINT_SOLID) {
		st.rs[ri].code_bg = code_bg.h;
		st.rs[ri].code_bg_kind = 1u32;
	} else if code_bg.tag == crate::slir::T_PAINT_GRADIENT {
		st.rs[ri].code_bg = code_bg.h;
		st.rs[ri].code_bg_kind = 2u32;
	}
	let weight = f64_to_u32(st.rs[ri].weight);
	st.rs[ri].font = if dynamic_family_uninterned {
		crate::slir::font_select_name(d, &dynamic_family, weight)
	} else {
		cached_font(d, st, fam, weight)
	};
	st.rs[ri].talign = match kw_at(d, st, node, crate::slir::A_ALIGN_TEXT) {
		Ok(kw) => u32::from(kw.talign),
		Err(s) => crate::style::talign_code(s),
	};
	st.rs[ri].content = crate::style::content_text(d, st, node);
	let role = a11y_ref(d, st, node, crate::slir::A_ROLE, true);
	let label = a11y_ref(d, st, node, crate::slir::A_LABEL, false);
	let desc = a11y_ref(d, st, node, crate::slir::A_DESC, false);
	let label = if label == 0 {
		// Controls without an authored label derive their accessible name
		// from descendant text content (§15.2); adapters inherit it through
		// the ordinary scene label slot.
		let base = crate::list::base(&st.lists, d, node);
		let activates = st
			.activate_node
			.get(index_u32(base))
			.copied()
			.unwrap_or(false);
		if st.rs[ri].flags & crate::slir::F_FOCUSABLE != 0 || activates {
			let name = crate::style::name_from_content(d, st, node);
			intern_scene_str(st, name)
		} else {
			0
		}
	} else {
		label
	};
	let mut value_now = semantic_number(d, st, node, crate::slir::A_VALUE_NOW);
	let mut value_min = semantic_number(d, st, node, crate::slir::A_VALUE_MIN);
	let mut value_max = semantic_number(d, st, node, crate::slir::A_VALUE_MAX);
	if value_min
		.zip(value_max)
		.is_some_and(|(minimum, maximum)| minimum > maximum)
	{
		value_now = None;
		value_min = None;
		value_max = None;
	} else if value_now
		.zip(value_min)
		.is_some_and(|(current, minimum)| current < minimum)
		|| value_now
			.zip(value_max)
			.is_some_and(|(current, maximum)| current > maximum)
	{
		value_now = None;
	}
	let level = semantic_number(d, st, node, crate::slir::A_LEVEL);
	let mut pos_in_set = semantic_number(d, st, node, crate::slir::A_POS_IN_SET);
	let set_size = semantic_number(d, st, node, crate::slir::A_SET_SIZE);
	if pos_in_set
		.zip(set_size)
		.is_some_and(|(position, size)| size != -1.0 && position > size)
	{
		pos_in_set = None;
	}
	st.rs[ri].sem = Semantics {
		role,
		label,
		desc,
		checked: checked_code(d, st, node),
		expanded: semantic_bool_code(d, st, node, crate::slir::A_EXPANDED),
		selected: semantic_bool_code(d, st, node, crate::slir::A_SELECTED),
		active_descendant: a11y_ref(d, st, node, crate::slir::A_ACTIVE_DESCENDANT, false),
		controls: a11y_ref(d, st, node, crate::slir::A_CONTROLS, false),
		value_now,
		value_min,
		value_max,
		value_text: a11y_ref(d, st, node, crate::slir::A_VALUE_TEXT, false),
		modal: semantic_bool_code(d, st, node, crate::slir::A_MODAL),
		live: live_code(d, st, node),
		live_atomic: semantic_bool_code(d, st, node, crate::slir::A_LIVE_ATOMIC),
		level,
		pos_in_set,
		set_size,
	};
	// Resolve leaf-specific image, path, and grid data last.
	if kind == crate::slir::K_IMG {
		let line = st.rs[ri].line;
		st.rs[ri].img = resolve_image(d, st, node, line);
	}
	st.rs[ri].fit = match kw_at(d, st, node, crate::slir::A_FIT) {
		Ok(kw) => u32::from(kw.fit),
		Err(s) => crate::style::fit_code(s),
	};
	let path_attr = crate::style::attr_ix(d, st, node, crate::slir::A_D);
	let encoded_path = crate::style::aval_active(d, st, path_attr);
	let path = if path_attr < 0 {
		crate::style::PATH_NONE
	} else if encoded_path.tag == crate::slir::T_PATH_REF {
		i32::try_from(encoded_path.h).expect("path index exceeds i32")
	} else if matches!(encoded_path.tag, crate::slir::T_PARAM_REF | crate::slir::T_PROP_REF) {
		let data = crate::style::attr_str(d, st, node, crate::slir::A_D);
		let line = st.rs[ri].line;
		resolve_runtime_path(st, &data, line)
	} else {
		crate::style::PATH_NONE
	};
	let path_min = crate::style::path_coords(d, st, path)
		.and_then(crate::pathdata::bounds)
		.map(|(min_x, min_y, ..)| (min_x, min_y));
	st.rs[ri].path = path;
	if let Some((min_x, min_y)) = path_min {
		st.rs[ri].path_min_x = min_x;
		st.rs[ri].path_min_y = min_y;
	}
	if kind == crate::slir::K_ICON {
		let name = crate::style::attr_str_ref(d, st, node, crate::slir::A_SRC);
		if let Some(index) = d
			.icon_name
			.iter()
			.rposition(|&candidate| crate::slir::str_ref(d, candidate) == name)
		{
			st.rs[ri].icon = i32::try_from(index).expect("icon index exceeds i32");
		} else {
			let owned = name.into_owned();
			if st.icon_missing.insert(owned.clone()) {
				let line = st.rs[ri].line;
				crate::style::warn(
					st,
					"icon-missing",
					&format!("icon '{owned}' is not declared"),
					line,
				);
			}
		}
	}
	let cols = crate::style::attr_val(d, st, node, crate::slir::A_COLS);
	if cols.tag == crate::slir::T_TUPLE && cols.ln >= 2 {
		st.rs[ri].track_off = i32::try_from(st.track_kind.len()).expect("track offset exceeds i32");
		st.rs[ri].track_len = cols.ln.wrapping_div(2);
		for tuple_index in (0..cols.ln).step_by(2) {
			st.track_kind
				.push(f64_to_u32(crate::value::tuple_at(d, &cols, tuple_index)));
			st.track_v
				.push(crate::value::tuple_at(d, &cols, tuple_index.wrapping_add(1)));
		}
	}
	st.rs[ri].span =
		f64_to_i32(1.0f64.max(crate::style::attr_num(d, st, node, crate::slir::A_SPAN, 1.0)));
	// The effective-attr fast path memoizes patch selection for this node.
	// Dispatch mutates states/lists between solves without rebuilding it, so
	// it must not outlive this resolution; drop it before returning.
	st.effective_attr_node = crate::slir::NONE;
	i32::try_from(ri).expect("resolved style index exceeds i32")
}

/// Creates the neutral resolved-style value for one node.
pub fn rstyle_default(node: u32, kind: u32, line: u32) -> crate::style::RStyle {
	crate::style::RStyle {
		node,
		kind,
		line,
		flags: 0u32,
		is_row: false,
		w_kind: crate::style::S_HUG,
		w_v: 0.0f64,
		h_kind: crate::style::S_HUG,
		h_v: 0.0f64,
		min_w: 0.0f64,
		max_w: crate::style::INF,
		min_h: 0.0f64,
		max_h: crate::style::INF,
		pad_t: 0.0f64,
		pad_r: 0.0f64,
		pad_b: 0.0f64,
		pad_l: 0.0f64,
		gap: 0.0f64,
		gap_cross: 0.0f64,
		has_gap_cross: false,
		pack: 0u32,
		align: (-1i32),
		self_align: (-1i32),
		offset_x: 0.0f64,
		offset_y: 0.0f64,
		at_x: 0.0f64,
		at_y: 0.0f64,
		has_at: false,
		anchor: (-1i32),
		has_attach: false,
		attach: String::new(),
		gravity: Gravity::BelowStart,
		collide_auto: true,
		rotate: 0.0f64,
		scale_x: 1.0f64,
		scale_y: 1.0f64,
		has_tilt: false,
		tilt_rx: 0.0f64,
		tilt_ry: 0.0f64,
		tilt_depth: 800.0f64,
		bg_kind: 0u32,
		bg_h: 0u32,
		stroke_kind: 0u32,
		stroke_h: 0u32,
		stroke_w: 1.0f64,
		stroke_align: 0u32,
		stroke_sides: 15u32,
		dash_on: 0.0f64,
		dash_off: 0.0f64,
		has_dash: false,
		radius: 0.0f64,
		smooth: 0.0f64,
		shadow_off: 0i32,
		shadow_len: 0i32,
		opacity: 1.0f64,
		blur: 0.0f64,
		grain_amount: 0.0f64,
		grain_size: 1.0f64,
		mask_kind: 0u32,
		mask_h: 0u32,
		has_backdrop: false,
		backdrop_blur: 0.0f64,
		backdrop_sat: 1.0f64,
		backdrop_bright: 1.0f64,
		bmask_kind: 0u32,
		bmask_h: 0u32,
		scrollbar: 0u32,
		scrollbar_w: 4.0f64,
		scrollbar_fg: 0x80808080u32,
		scrollbar_bg: 0x33808080u32,
		split_w: 4.0f64,
		split_fg_kind: 0u32,
		split_fg: 0u32,
		fam: 0u32,
		font: (-1i32),
		size: 14.0f64,
		weight: 400.0f64,
		leading: 1.4f64,
		tracking: 0.0f64,
		strike: false,
		italic: false,
		underline: false,
		color: 0x111111ffu32,
		color_kind: 1u32,
		code_color: 0u32,
		code_color_kind: 0u32,
		code_bg: 0u32,
		code_bg_kind: 0u32,
		talign: 0u32,
		content: crate::text::Text::default(),
		sem: Semantics::default(),
		img: (-1i32),
		fit: 0u32,
		path: crate::style::PATH_NONE,
		path_min_x: 0.0f64,
		path_min_y: 0.0f64,
		icon: (-1i32),
		track_off: 0i32,
		track_len: 0i32,
		span: 1i32,
	}
}

// Clears one width/height comparison patch before layout peeks.
fn reset_wh_patch(
	d: &crate::slir::Doc,
	st: &mut crate::style::St,
	node: u32,
	synthetic: bool,
	patch: usize,
) {
	let condition = d.patch_cond[patch];
	let kind = d.cond_kind[index_u32(condition)];
	if kind != crate::slir::C_WCMP && kind != crate::slir::C_HCMP {
		return;
	}
	if synthetic {
		crate::style::wh_set(st, node, i32::try_from(patch).expect("patch index exceeds i32"), false);
	} else {
		st.patch_on[patch] = false;
	}
}

/// Resets a node's width/height comparison patches before layout peeks.
///
/// Layout reads child size specifications before measuring them. Clearing
/// these flags prevents a previous measure of the same node from leaking into
/// that pass.
pub fn reset_wh_patches(d: &crate::slir::Doc, st: &mut crate::style::St, node: u32) {
	let base = crate::list::base(&st.lists, d, node);
	let base_index = index_u32(base);
	let synthetic = crate::list::each_of(&st.lists, d, node) != crate::slir::NONE;
	if st.lists.patches_by_node.len() == d.node_kind.len() {
		for index in 0..st.lists.patches_by_node[base_index].len() {
			let patch = st.lists.patches_by_node[base_index][index];
			reset_wh_patch(d, st, node, synthetic, patch);
		}
	} else {
		for (patch, &patch_node) in d.patch_node.iter().enumerate() {
			if patch_node == base {
				reset_wh_patch(d, st, node, synthetic, patch);
			}
		}
	}
}

/// Resolves one axis's size with context defaults, without building a style.
///
/// Used by first-pass fill detection and wrap/grid lookahead.
pub fn peek_size(
	d: &crate::slir::Doc,
	st: &crate::style::St,
	node: u32,
	axis_w: bool,
	parent_kind: u32,
	parent_is_row: bool,
) -> crate::style::Size {
	let kind = d.node_kind[index_u32(crate::list::base(&st.lists, d, node))];
	let in_layer = (parent_kind == crate::slir::K_STACK) || (parent_kind == crate::slir::K_CANVAS);
	let mut dk = crate::style::S_HUG;
	let mut dv = 0.0f64;
	if (crate::style::is_container(kind) || (kind == crate::slir::K_HOLE)) && (!in_layer) {
		if parent_is_row && (!axis_w) {
			dk = crate::style::S_FILL;
			dv = 1.0f64;
		}
		if (!parent_is_row) && axis_w {
			dk = crate::style::S_FILL;
			dv = 1.0f64;
		}
	}
	if kind == crate::slir::K_SPACER {
		if parent_is_row && axis_w {
			dk = crate::style::S_FILL;
			dv = 1.0f64;
		}
		if (!parent_is_row) && (!axis_w) {
			dk = crate::style::S_FILL;
			dv = 1.0f64;
		}
	}
	let attr = if axis_w {
		crate::slir::A_W
	} else {
		crate::slir::A_H
	};
	crate::style::size_of(&crate::style::attr_val(d, st, node, attr), dk, dv)
}

#[cfg(test)]
mod vector_path_tests {
	use super::*;

	#[test]
	fn runtime_path_cache_deduplicates_geometry_and_bad_values() {
		let mut state = st_new();
		let first = resolve_runtime_path(&mut state, "m1 2 h3 v4", 7);
		let same = resolve_runtime_path(&mut state, "m1 2 h3 v4", 8);
		assert_eq!(first, same);
		assert_eq!(state.rt_path_verbs.len(), 1);
		assert_eq!(state.rt_path_coords.len(), 1);
		assert_eq!(state.rt_path_coords[0], [1.0, 2.0, 4.0, 2.0, 4.0, 6.0]);

		assert_eq!(resolve_runtime_path(&mut state, "not path data", 9), PATH_NONE);
		assert_eq!(resolve_runtime_path(&mut state, "not path data", 10), PATH_NONE);
		assert_eq!(state.diag_code, ["attr"]);
		assert_eq!(state.diag_line, [9]);
	}
}

#[cfg(test)]
mod attribute_cache_tests {
	use super::*;

	#[test]
	fn attribute_cache_count_matches_canonical_slir_table_and_covers_highest_id() {
		assert_eq!(
			crate::slir::ATTR_COUNT,
			slab_slir::attrs::ATTR_COUNT,
			"kernel ATTR_COUNT must match normative slab-slir table size"
		);
		let highest_id = crate::slir::A_TAB_SIZE;
		assert_eq!(
			(highest_id as usize) + 1,
			crate::slir::ATTR_COUNT,
			"A_TAB_SIZE must be the highest attribute ID"
		);
		for &(id, name) in slab_slir::attrs::ATTRS {
			assert!(
				(id as usize) < crate::slir::ATTR_COUNT,
				"Attribute '{name}' (id {id}) falls outside ATTR_COUNT ({})",
				crate::slir::ATTR_COUNT
			);
		}
	}

	#[test]
	fn effective_attr_cache_resolves_highest_id_underline() {
		let mut doc = crate::slir::doc_new();
		doc.ok = true;
		doc.strs.push(String::new());
		doc.strs.push("cancel_target".to_string());
		doc.node_kind.push(crate::slir::K_RECT);
		doc.node_flags.push(0);
		doc.node_parent.push(crate::slir::NONE);
		doc.node_first.push(crate::slir::NONE);
		doc.node_next.push(crate::slir::NONE);
		doc.node_key.push(0);
		doc.node_id.push(0);
		doc.node_line.push(1);

		// Authored base attribute A_UNDERLINE = 93 set to value 0.
		doc.attr_index.push(0);
		doc.aval_tag.push(crate::slir::T_STR);
		doc.aval_lo.push(1);
		doc.aval_hi.push(0);
		doc.aval_num.push(0.0);
		doc.attr_id.push(crate::slir::A_UNDERLINE);
		doc.attr_val.push(0);
		doc.attr_index.push(1);

		let mut st = st_new();
		init_params(&doc, &mut st);

		// 1. Query attr_ix before prepare_attrs (effective_attr_node is NONE)
		assert_eq!(st.effective_attr_node, crate::slir::NONE);
		assert_eq!(attr_ix(&doc, &st, 0, crate::slir::A_UNDERLINE), 0);

		// 2. Prepare effective attrs for node 0 (active effective_attr_node path)
		prepare_attrs(&doc, &mut st, 0);
		assert_eq!(st.effective_attr_node, 0);

		// Verify the effective cache covers the highest canonical attribute.
		assert_eq!(st.effective_attr_values[crate::slir::A_UNDERLINE as usize], 0);

		// Query attr_ix while effective_attr_node == 0
		assert_eq!(
			attr_ix(&doc, &st, 0, crate::slir::A_UNDERLINE),
			0,
			"attr_ix must return the cached A_UNDERLINE value for the active node"
		);
	}

	/// A state flip between solves must select fresh patch attributes: the
	/// effective-attr fast path may not outlive [`build_rstyle`], or dispatch
	/// would read pre-flip patch selections (e.g. a signal channel gated on
	/// `pressed`).
	#[test]
	fn state_flip_after_build_rstyle_selects_fresh_patch_attrs() {
		let mut doc = crate::slir::doc_new();
		doc.ok = true;
		doc.strs.push(String::new());
		doc.strs.push("pressed".to_string());
		doc.node_kind.push(crate::slir::K_RECT);
		doc.node_flags.push(0);
		doc.node_parent.push(crate::slir::NONE);
		doc.node_first.push(crate::slir::NONE);
		doc.node_next.push(crate::slir::NONE);
		doc.node_key.push(0);
		doc.node_id.push(0);
		doc.node_line.push(1);
		doc.attr_index.extend([0, 0]); // no authored attrs

		// Condition 0: State("pressed"). Patch 0 on node 0 sets A_OPACITY to
		// encoded value 7 while the state is on.
		doc.cond_kind.push(crate::slir::C_STATE);
		doc.cond_neg.push(0);
		doc.cond_op.push(0);
		doc.cond_num.push(0.0);
		doc.cond_sym.push(1);
		doc.patch_node.push(0);
		doc.patch_cond.push(0);
		doc.patch_attr_off.push(0);
		doc.patch_attr_len.push(1);
		doc.patch_child_off.push(0);
		doc.patch_child_len.push(0);
		doc.wattr_id.push(crate::slir::A_OPACITY);
		doc.wattr_val.push(7);

		let mut st = st_new();
		init_params(&doc, &mut st);
		begin_solve(&doc, &mut st);

		// Resolve once with the state off; the fast path must not survive
		// build_rstyle, or post-solve dispatch lookups (sig_of/attr_ix) would
		// reuse this node's pre-flip patch selection.
		build_rstyle(
			&doc,
			&mut st,
			0,
			crate::slir::NONE,
			false,
			0,
			0,
			0,
			16.0,
			400.0,
			1.2,
			0.0,
			false,
			false,
			false,
		);
		assert_eq!(
			st.effective_attr_node,
			crate::slir::NONE,
			"effective-attr fast path must not outlive build_rstyle"
		);
		assert_eq!(attr_ix(&doc, &st, 0, crate::slir::A_OPACITY), -1);

		// Dispatch-time state flip followed by the next solve boundary: the
		// patch selection must reflect the new state, not a cached snapshot.
		assert!(set_node_state(&doc, &mut st, 0, "pressed", true));
		begin_solve(&doc, &mut st);
		assert_eq!(
			attr_ix(&doc, &st, 0, crate::slir::A_OPACITY),
			7,
			"attr_ix must observe the state-selected patch after the flip"
		);
	}
}

#[cfg(test)]
mod pad_side_tests {
	use super::*;

	/// A `pad` tuple resolves all four sides; `pad-t`/`pad-b` override only
	/// their own side of the resolved style.
	#[test]
	fn pad_side_attrs_override_the_tuple() {
		let mut doc = crate::slir::doc_new();
		doc.ok = true;
		doc.strs.push(String::new());
		doc.node_kind.push(crate::slir::K_RECT);
		doc.node_flags.push(0);
		doc.node_parent.push(crate::slir::NONE);
		doc.node_first.push(crate::slir::NONE);
		doc.node_next.push(crate::slir::NONE);
		doc.node_key.push(0);
		doc.node_id.push(0);
		doc.node_line.push(1);
		doc.f64s.extend([8.0, 8.0, 8.0, 8.0]);
		// AVAL 0: pad tuple (8,8,8,8); 1: pad-t 20; 2: pad-b 4.
		doc.aval_tag
			.extend([crate::slir::T_TUPLE, crate::slir::T_NUM, crate::slir::T_NUM]);
		doc.aval_lo.extend([0, 0, 0]);
		doc.aval_hi.extend([4, 0, 0]);
		doc.aval_num.extend([0.0, 20.0, 4.0]);
		doc.attr_index.extend([0, 3]);
		doc.attr_id
			.extend([crate::slir::A_PAD, crate::slir::A_PAD_T, crate::slir::A_PAD_B]);
		doc.attr_val.extend([0, 1, 2]);

		let mut st = st_new();
		init_params(&doc, &mut st);
		begin_solve(&doc, &mut st);
		let ri = build_rstyle(
			&doc,
			&mut st,
			0,
			crate::slir::NONE,
			false,
			0,
			0,
			0,
			16.0,
			400.0,
			1.2,
			0.0,
			false,
			false,
			false,
		);
		let resolved = &st.rs[ri as usize];
		assert_eq!(resolved.pad_t, 20.0, "pad-t overrides the tuple top");
		assert_eq!(resolved.pad_r, 8.0, "tuple right survives");
		assert_eq!(resolved.pad_b, 4.0, "pad-b overrides the tuple bottom");
		assert_eq!(resolved.pad_l, 8.0, "tuple left survives");
	}
}
