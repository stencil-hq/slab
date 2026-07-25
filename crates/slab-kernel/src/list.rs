//! Persistent typed recursive-list values and synthetic node identity.
//!
//! List instances are flat runtime records linked by `(owner, item, field)`.
//! This keeps recursive schemas data-bounded while preserving stable item and
//! synthetic-node identity across keyed reorders and virtual windows.

use crate::{slir, value};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

const NONE: u32 = u32::MAX;

#[derive(Clone, Debug)]
enum KeyItems {
    One(i32),
    Many(Vec<i32>),
}

impl KeyItems {
    fn first(&self) -> i32 {
        match self {
            Self::One(item) => *item,
            Self::Many(items) => items[0],
        }
    }
}

/// Runtime storage for root and nested lists, keys, field values, windows, and synthetic nodes.
#[derive(Clone, Debug)]
pub struct State {
    pub li_id: Vec<u32>,
    pub li_param: Vec<u32>,
    pub li_schema: Vec<u32>,
    pub li_owner: Vec<u32>,
    pub li_owner_index: Vec<i32>,
    pub li_owner_field: Vec<u32>,
    pub li_len: Vec<i32>,
    pub li_next: u32,
    li_slot: HashMap<u32, usize>,
    li_child: HashMap<(u32, i32, u32), u32>,
    pub lk_param: Vec<u32>,
    pub lk_index: Vec<i32>,
    pub lk_key: Vec<String>,
    lk_slot: HashMap<(u32, i32), usize>,
    lk_key_index: HashMap<u32, HashMap<String, KeyItems>>,
    pub lv_param: Vec<u32>,
    pub lv_index: Vec<i32>,
    pub lv_field: Vec<u32>,
    pub lv_kind: Vec<u32>,
    pub lv_num: Vec<f64>,
    pub lv_str: Vec<String>,
    pub lv_h: Vec<u32>,
    pub lv_sym: Vec<String>,
    lv_slot: HashMap<(u32, i32, u32), usize>,
    pub sy_id: Vec<u32>,
    pub sy_each: Vec<u32>,
    pub sy_tpl: Vec<u32>,
    pub sy_key: Vec<String>,
    sy_item: Vec<i32>,
    sy_list: Vec<u32>,
    pub sy_next: u32,
    sy_slot: HashMap<u32, usize>,
    sy_identity: HashMap<(u32, u32), HashMap<String, u32>>,
    sy_materialized: Vec<u32>,
    sy_materialized_set: HashSet<u32>,
    sy_deleted: Vec<u32>,
    prune_pending: bool,
    pub win_each: Vec<u32>,
    pub win_start: Vec<i32>,
    pub win_end: Vec<i32>,
    pub win_viewport: Vec<f64>,
    pub win_origin: Vec<f64>,
}

/// A normalized value stored in a typed scalar list field.
#[derive(Clone, Debug)]
pub struct Val {
    pub kind: u32,
    pub num: f64,
    pub s: String,
    pub rgba: u32,
    pub sym: String,
}

fn index(value: i32) -> usize {
    usize::try_from(value).expect("nonnegative list index")
}

fn signed(value: u32) -> i32 {
    i32::from_ne_bytes(value.to_ne_bytes())
}

fn unsigned(value: i32) -> u32 {
    u32::from_ne_bytes(value.to_ne_bytes())
}

fn len_i32<T>(values: &[T]) -> i32 {
    i32::try_from(values.len()).expect("list state exceeds i32 capacity")
}

#[cfg(test)]
thread_local! {
    static LOOKUP_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[inline]
fn note_lookup() {
    #[cfg(test)]
    LOOKUP_WORK.with(|work| work.set(work.get().saturating_add(1)));
}

#[cfg(test)]
pub fn reset_lookup_work() {
    LOOKUP_WORK.with(|work| work.set(0));
}

#[cfg(test)]
pub fn lookup_work() -> usize {
    LOOKUP_WORK.with(std::cell::Cell::get)
}

fn truncate_i32(value: f64) -> i32 {
    if value.is_nan() {
        return 0;
    }
    if value >= f64::from(i32::MAX) {
        return i32::MAX;
    }
    if value <= f64::from(i32::MIN) {
        return i32::MIN;
    }
    value.trunc() as i32
}

fn empty_val(kind: u32) -> Val {
    Val {
        kind,
        num: 0.0,
        s: String::new(),
        rgba: 0,
        sym: String::new(),
    }
}

/// Creates empty runtime list state.
pub fn state_new() -> State {
    State {
        li_id: Vec::new(),
        li_param: Vec::new(),
        li_schema: Vec::new(),
        li_owner: Vec::new(),
        li_owner_index: Vec::new(),
        li_owner_field: Vec::new(),
        li_len: Vec::new(),
        li_next: 0,
        li_slot: HashMap::default(),
        li_child: HashMap::default(),
        lk_param: Vec::new(),
        lk_index: Vec::new(),
        lk_key: Vec::new(),
        lk_slot: HashMap::default(),
        lk_key_index: HashMap::default(),
        lv_param: Vec::new(),
        lv_index: Vec::new(),
        lv_field: Vec::new(),
        lv_kind: Vec::new(),
        lv_num: Vec::new(),
        lv_str: Vec::new(),
        lv_h: Vec::new(),
        lv_sym: Vec::new(),
        lv_slot: HashMap::default(),
        sy_id: Vec::new(),
        sy_each: Vec::new(),
        sy_tpl: Vec::new(),
        sy_key: Vec::new(),
        sy_item: Vec::new(),
        sy_list: Vec::new(),
        sy_next: 0,
        sy_slot: HashMap::default(),
        sy_identity: HashMap::default(),
        sy_materialized: Vec::new(),
        sy_materialized_set: HashSet::default(),
        sy_deleted: Vec::new(),
        prune_pending: false,
        win_each: Vec::new(),
        win_start: Vec::new(),
        win_end: Vec::new(),
        win_viewport: Vec::new(),
        win_origin: Vec::new(),
    }
}

fn slot_by_id(s: &State, list: u32) -> i32 {
    note_lookup();
    s.li_slot.get(&list).map_or(-1, |&slot| {
        i32::try_from(slot).expect("list slot exceeds i32")
    })
}

/// Returns the runtime slot for a root list parameter, or `-1` when invalid.
pub fn list_slot(d: &slir::Doc, s: &State, param: u32) -> i32 {
    let param_ix = signed(param);
    if param_ix < 0 || param_ix >= len_i32(&d.parm_type) || d.parm_type[index(param_ix)] != 6 {
        return -1;
    }
    slot_by_id(s, param)
}

/// Returns the runtime handle for a root list parameter, or `NONE` when invalid.
pub fn root_id(d: &slir::Doc, s: &State, param: u32) -> u32 {
    let slot = list_slot(d, s, param);
    if slot < 0 { NONE } else { s.li_id[index(slot)] }
}

/// Returns a root or nested list's current length, or `-1` for an invalid handle.
pub fn length(_d: &slir::Doc, s: &State, list: u32) -> i32 {
    let slot = slot_by_id(s, list);
    if slot < 0 { -1 } else { s.li_len[index(slot)] }
}

/// Returns an item's assigned key, its decimal index by default, or empty out of range.
pub fn key_at(d: &slir::Doc, s: &State, list: u32, item_index: i32) -> String {
    let n = length(d, s, list);
    if item_index < 0 || item_index >= n {
        return String::new();
    }
    note_lookup();
    s.lk_slot.get(&(list, item_index)).map_or_else(
        || unsigned(item_index).to_string(),
        |&slot| s.lk_key[slot].clone(),
    )
}

/// Returns the schema row for a root list parameter, or `-1` when absent.
pub fn schema_ix(d: &slir::Doc, param: u32) -> i32 {
    d.list_param
        .iter()
        .position(|candidate| *candidate == param)
        .map_or(-1, |slot| {
            i32::try_from(slot).expect("schema index exceeds i32")
        })
}

fn schema_for_list(s: &State, list: u32) -> i32 {
    let slot = slot_by_id(s, list);
    if slot < 0 {
        -1
    } else {
        signed(s.li_schema[index(slot)])
    }
}
/// Returns a concrete root or child list's schema row.
pub fn list_schema(s: &State, list: u32) -> i32 {
    schema_for_list(s, list)
}

fn field_ix_schema(d: &slir::Doc, schema: u32, name: &str) -> i32 {
    let Some(&off) = d.list_field_off.get(schema as usize) else {
        return -1;
    };
    let Some(&len) = d.list_field_len.get(schema as usize) else {
        return -1;
    };
    for field in off..off.wrapping_add(len) {
        if slir::str_at(d, d.list_field_name[index(field)]) == name {
            return field;
        }
    }
    -1
}

/// Returns the absolute field index for a named root or nested list field.
pub fn field_ix(d: &slir::Doc, s: &State, list: u32, name: &str) -> i32 {
    let schema = schema_for_list(s, list);
    if schema < 0 {
        -1
    } else {
        field_ix_schema(d, unsigned(schema), name)
    }
}

fn child_id(s: &State, owner: u32, item: i32, field: u32) -> u32 {
    note_lookup();
    s.li_child
        .get(&(owner, item, field))
        .copied()
        .unwrap_or(NONE)
}

fn push_list(
    s: &mut State,
    id: u32,
    param: u32,
    schema: u32,
    owner: u32,
    owner_index: i32,
    owner_field: u32,
) {
    let slot = s.li_id.len();
    s.li_slot.insert(id, slot);
    if owner != NONE {
        s.li_child.insert((owner, owner_index, owner_field), id);
    }
    s.li_id.push(id);
    s.li_param.push(param);
    s.li_schema.push(schema);
    s.li_owner.push(owner);
    s.li_owner_index.push(owner_index);
    s.li_owner_field.push(owner_field);
    s.li_len.push(0);
}

fn ensure_child(s: &mut State, owner: u32, item: i32, field: u32, schema: u32) -> u32 {
    let existing = child_id(s, owner, item, field);
    if existing != NONE {
        return existing;
    }
    let id = s.li_next;
    s.li_next = s.li_next.wrapping_add(1);
    push_list(s, id, NONE, schema, owner, item, field);
    id
}

/// Resolves `""` or alternating `<index>.<field>` pairs to a list handle.
pub fn resolve_path(d: &slir::Doc, s: &State, param: u32, path: &str) -> u32 {
    let mut list = root_id(d, s, param);
    if list == NONE || path.is_empty() {
        return list;
    }
    let mut parts = path.split('.');
    while let Some(index_text) = parts.next() {
        let Some(field_name) = parts.next() else {
            return NONE;
        };
        if index_text.is_empty() || field_name.is_empty() {
            return NONE;
        }
        let Ok(item) = index_text.parse::<i32>() else {
            return NONE;
        };
        if item < 0 || item >= length(d, s, list) {
            return NONE;
        }
        let field = field_ix(d, s, list, field_name);
        if field < 0 || d.list_field_type.get(index(field)).copied() != Some(6) {
            return NONE;
        }
        let schema = schema_for_list(s, list);
        if schema < 0 {
            return NONE;
        }
        let relative = unsigned(field.wrapping_sub(d.list_field_off[index(schema)]));
        list = child_id(s, list, item, relative);
        if list == NONE {
            return NONE;
        }
        if parts.clone().next().is_none() {
            break;
        }
    }
    list
}

/// Decodes and normalizes an attribute value for a scalar list field type.
pub fn val_from_aval(d: &slir::Doc, kind: u32, ix: i32) -> Val {
    let decoded = value::decode(d, ix);
    let mut out = empty_val(kind);
    match kind {
        0 if decoded.tag == slir::T_STR => out.s = slir::str_at(d, decoded.h),
        1 | 2 => out.num = decoded.num,
        3 => out.rgba = decoded.h,
        4 => out.num = if decoded.num == 0.0 { 0.0 } else { 1.0 },
        5 if decoded.tag == slir::T_ENUM_SYM => out.sym = slir::str_at(d, decoded.h),
        _ => {}
    }
    out
}

/// Stores one normalized scalar field value and reports whether state changed.
pub fn store(s: &mut State, list: u32, item_index: i32, field: u32, v: &Val) -> bool {
    let key = (list, item_index, field);
    note_lookup();
    if let Some(&slot) = s.lv_slot.get(&key) {
        let changed = s.lv_kind[slot] != v.kind
            || s.lv_num[slot] != v.num
            || s.lv_str[slot] != v.s
            || s.lv_h[slot] != v.rgba
            || s.lv_sym[slot] != v.sym;
        if changed {
            s.lv_kind[slot] = v.kind;
            s.lv_num[slot] = v.num;
            s.lv_str[slot].clone_from(&v.s);
            s.lv_h[slot] = v.rgba;
            s.lv_sym[slot].clone_from(&v.sym);
        }
        return changed;
    }
    let slot = s.lv_param.len();
    s.lv_slot.insert(key, slot);
    s.lv_param.push(list);
    s.lv_index.push(item_index);
    s.lv_field.push(field);
    s.lv_kind.push(v.kind);
    s.lv_num.push(v.num);
    s.lv_str.push(v.s.clone());
    s.lv_h.push(v.rgba);
    s.lv_sym.push(v.sym.clone());
    true
}

fn list_default_len(d: &slir::Doc, encoded: i32) -> i32 {
    if encoded < 0
        || encoded >= len_i32(&d.aval_tag)
        || d.aval_tag[index(encoded)] != slir::T_LIST_DEFAULT
    {
        0
    } else {
        signed(d.aval_hi[index(encoded)]).max(0)
    }
}

fn default_item_from(d: &slir::Doc, encoded: i32, item: i32) -> i32 {
    let n = list_default_len(d, encoded);
    if item < 0 || item >= n {
        -1
    } else {
        signed(d.aval_lo[index(encoded)]).wrapping_add(item)
    }
}

/// Returns the encoded default item for a root parameter index.
pub fn default_item(d: &slir::Doc, param: u32, item_index: i32) -> i32 {
    let Some(&encoded) = d.parm_default.get(param as usize) else {
        return -1;
    };
    default_item_from(d, signed(encoded), item_index)
}

fn item_override(d: &slir::Doc, item: i32, field: i32) -> i32 {
    if item < 0 || item >= len_i32(&d.list_item_field_off) {
        return -1;
    }
    let lo = d.list_item_field_off[index(item)];
    let hi = lo.wrapping_add(d.list_item_field_len[index(item)]);
    for override_ix in lo..hi {
        if signed(d.list_item_value_field[index(override_ix)]) == field {
            return signed(d.list_item_value_val[index(override_ix)]);
        }
    }
    -1
}

fn seed_item_id(d: &slir::Doc, s: &mut State, list: u32, item_index: i32, item: i32) {
    let schema = schema_for_list(s, list);
    if schema < 0 {
        return;
    }
    let lo = d.list_field_off[index(schema)];
    let hi = lo.wrapping_add(d.list_field_len[index(schema)]);
    for field in lo..hi {
        let absolute = index(field);
        let relative = field.wrapping_sub(lo);
        let override_value = item_override(d, item, relative);
        let encoded = if override_value >= 0 {
            override_value
        } else {
            signed(d.list_field_default[absolute])
        };
        let kind = d.list_field_type[absolute];
        if kind == 6 {
            let sub = d.list_field_sub.get(absolute).copied().unwrap_or(0);
            if sub == 0 {
                continue;
            }
            let child = ensure_child(s, list, item_index, unsigned(relative), sub - 1);
            let n = list_default_len(d, encoded);
            let child_slot = slot_by_id(s, child);
            s.li_len[index(child_slot)] = n;
            for child_item in 0..n {
                seed_item_id(
                    d,
                    s,
                    child,
                    child_item,
                    default_item_from(d, encoded, child_item),
                );
            }
        } else {
            let v = val_from_aval(d, kind, encoded);
            store(s, list, item_index, unsigned(relative), &v);
        }
    }
}

/// Seeds one item from schema defaults and optional per-item overrides.
pub fn seed_item(d: &slir::Doc, s: &mut State, list: u32, item_index: i32, item: i32) {
    seed_item_id(d, s, list, item_index, item);
}

fn add_key_lookup(s: &mut State, list: u32, item: i32, key: &str) {
    use std::collections::hash_map::Entry;

    match s
        .lk_key_index
        .entry(list)
        .or_default()
        .entry(key.to_owned())
    {
        Entry::Vacant(entry) => {
            entry.insert(KeyItems::One(item));
        }
        Entry::Occupied(mut entry) => {
            let items = entry.get_mut();
            match items {
                KeyItems::One(current) if *current != item => {
                    let first = *current;
                    *items = KeyItems::Many(vec![first.min(item), first.max(item)]);
                }
                KeyItems::Many(duplicates) if !duplicates.contains(&item) => {
                    if item < duplicates[0] {
                        let previous_first = duplicates[0];
                        duplicates[0] = item;
                        duplicates.push(previous_first);
                    } else {
                        duplicates.push(item);
                    }
                }
                KeyItems::One(_) | KeyItems::Many(_) => {}
            }
        }
    }
}

fn remove_key_lookup(s: &mut State, list: u32, item: i32, key: &str) {
    let mut remove_indices = false;
    if let Some(indices) = s.lk_key_index.get_mut(&list) {
        let mut remove_key = false;
        if let Some(items) = indices.get_mut(key) {
            let mut collapse = None;
            match items {
                KeyItems::One(current) => remove_key = *current == item,
                KeyItems::Many(duplicates) => {
                    if let Some(position) =
                        duplicates.iter().position(|candidate| *candidate == item)
                    {
                        duplicates.swap_remove(position);
                        if position == 0 && duplicates.len() > 1 {
                            let first = duplicates
                                .iter()
                                .enumerate()
                                .min_by_key(|(_, candidate)| *candidate)
                                .map(|(index, _)| index)
                                .expect("duplicate key has items");
                            duplicates.swap(0, first);
                        }
                    }
                    if duplicates.len() == 1 {
                        collapse = Some(duplicates[0]);
                    }
                }
            }
            if let Some(only) = collapse {
                *items = KeyItems::One(only);
            }
        }
        if remove_key {
            indices.remove(key);
        }
        remove_indices = indices.is_empty();
    }
    if remove_indices {
        s.lk_key_index.remove(&list);
    }
}

fn item_index_for_key(d: &slir::Doc, s: &State, list: u32, key: &str) -> i32 {
    note_lookup();
    let explicit = s
        .lk_key_index
        .get(&list)
        .and_then(|indices| indices.get(key))
        .map(KeyItems::first);
    let implicit = key.parse::<i32>().ok().filter(|&item| {
        item >= 0 && item < length(d, s, list) && !s.lk_slot.contains_key(&(list, item))
    });
    match (explicit, implicit) {
        (Some(a), Some(b)) => a.min(b),
        (Some(item), None) | (None, Some(item)) => item,
        (None, None) => -1,
    }
}

/// Removes a stored scalar value slot without preserving storage order.
pub fn remove_value(s: &mut State, k: i32) {
    let slot = index(k);
    s.lv_slot
        .remove(&(s.lv_param[slot], s.lv_index[slot], s.lv_field[slot]));
    s.lv_param.swap_remove(slot);
    s.lv_index.swap_remove(slot);
    s.lv_field.swap_remove(slot);
    s.lv_kind.swap_remove(slot);
    s.lv_num.swap_remove(slot);
    s.lv_str.swap_remove(slot);
    s.lv_h.swap_remove(slot);
    s.lv_sym.swap_remove(slot);
    if slot < s.lv_param.len() {
        s.lv_slot
            .insert((s.lv_param[slot], s.lv_index[slot], s.lv_field[slot]), slot);
    }
}

/// Removes an assigned key slot without preserving storage order.
pub fn remove_key(s: &mut State, k: i32) {
    let slot = index(k);
    let list = s.lk_param[slot];
    let item = s.lk_index[slot];
    let key = s.lk_key[slot].clone();
    remove_key_lookup(s, list, item, &key);
    s.lk_slot.remove(&(list, item));
    s.lk_param.swap_remove(slot);
    s.lk_index.swap_remove(slot);
    s.lk_key.swap_remove(slot);
    if slot < s.lk_param.len() {
        s.lk_slot.insert((s.lk_param[slot], s.lk_index[slot]), slot);
    }
}

fn remove_list(s: &mut State, list: u32) {
    let children: Vec<u32> = s
        .li_id
        .iter()
        .zip(&s.li_owner)
        .filter_map(|(&id, &owner)| (owner == list).then_some(id))
        .collect();
    for child in children {
        remove_list(s, child);
    }
    let mut value_slot = len_i32(&s.lv_param) - 1;
    while value_slot >= 0 {
        if s.lv_param[index(value_slot)] == list {
            remove_value(s, value_slot);
        }
        value_slot -= 1;
    }
    let mut key_slot = len_i32(&s.lk_param) - 1;
    while key_slot >= 0 {
        if s.lk_param[index(key_slot)] == list {
            remove_key(s, key_slot);
        }
        key_slot -= 1;
    }
    let slot = slot_by_id(s, list);
    if slot >= 0 {
        let slot = index(slot);
        s.li_slot.remove(&list);
        if s.li_owner[slot] != NONE {
            s.li_child.remove(&(
                s.li_owner[slot],
                s.li_owner_index[slot],
                s.li_owner_field[slot],
            ));
        }
        s.li_id.swap_remove(slot);
        s.li_param.swap_remove(slot);
        s.li_schema.swap_remove(slot);
        s.li_owner.swap_remove(slot);
        s.li_owner_index.swap_remove(slot);
        s.li_owner_field.swap_remove(slot);
        s.li_len.swap_remove(slot);
        if slot < s.li_id.len() {
            s.li_slot.insert(s.li_id[slot], slot);
        }
    }
}

fn schema_eq(d: &slir::Doc, left: i32, right: i32) -> bool {
    if left < 0 || right < 0 {
        return false;
    }
    let (left, right) = (index(left), index(right));
    if d.list_field_len[left] != d.list_field_len[right] {
        return false;
    }
    (0..d.list_field_len[left]).all(|offset| {
        let a = index(d.list_field_off[left].wrapping_add(offset));
        let b = index(d.list_field_off[right].wrapping_add(offset));
        d.list_field_name[a] == d.list_field_name[b]
            && d.list_field_type[a] == d.list_field_type[b]
            && d.list_field_sub.get(a) == d.list_field_sub.get(b)
    })
}

fn authored_each_schema(d: &slir::Doc, each: u32) -> i32 {
    let decoded = value::decode(d, slir::base_attr(d, each, slir::A_EACH));
    if decoded.tag == slir::T_NUM {
        let param = truncate_i32(decoded.num);
        return if param < 0 {
            -1
        } else {
            schema_ix(d, unsigned(param))
        };
    }
    if decoded.tag != slir::T_PROP_REF {
        return -1;
    }
    let mut parent = d.node_parent[each as usize];
    while parent != slir::NONE && d.node_kind[parent as usize] != slir::K_EACH {
        parent = d.node_parent[parent as usize];
    }
    if parent == slir::NONE {
        return -1;
    }
    let outer = authored_each_schema(d, parent);
    if outer < 0 {
        return -1;
    }
    let absolute = d.list_field_off[index(outer)]
        .wrapping_add(i32::try_from(decoded.h).expect("list field index exceeds i32"));
    d.list_field_sub
        .get(absolute as usize)
        .copied()
        .filter(|sub| *sub != 0)
        .map_or(-1, |sub| signed(sub - 1))
}

/// Returns the detached template root for a concrete each.
///
/// Recursive syntax emits an empty back-edge instead of unrolling the schema
/// at compile time. Resolve that edge through the concrete each ownership
/// chain so two structurally identical schemas with different template bodies
/// can never borrow one another's template.
pub fn template_first(d: &slir::Doc, s: &State, each: u32) -> u32 {
    let base_node = base(s, d, each);
    if base_node == slir::NONE {
        return slir::NONE;
    }
    let first = d.node_first[base_node as usize];
    if first != slir::NONE {
        return first;
    }
    let list = each_list(d, s, each);
    if list < 0 {
        return slir::NONE;
    }
    let wanted = schema_for_list(s, unsigned(list));
    let mut owner = each_of(s, d, each);
    while owner != slir::NONE {
        let candidate = base(s, d, owner);
        if candidate == slir::NONE {
            break;
        }
        let candidate_first = d.node_first[candidate as usize];
        if candidate_first != slir::NONE && schema_eq(d, wanted, authored_each_schema(d, candidate))
        {
            return candidate_first;
        }
        owner = each_of(s, d, owner);
    }
    slir::NONE
}

/// Returns the root parameter encoded on an authored each, or `-1` for a property each.
pub fn each_param(d: &slir::Doc, each: u32) -> i32 {
    let decoded = value::decode(d, slir::base_attr(d, each, slir::A_EACH));
    if decoded.tag == slir::T_NUM {
        truncate_i32(decoded.num)
    } else {
        -1
    }
}

/// Resolves the concrete root or child list used by one each instance.
pub fn each_list(d: &slir::Doc, s: &State, each: u32) -> i32 {
    let base = base(s, d, each);
    if base == slir::NONE {
        return -1;
    }
    let decoded = value::decode(d, slir::base_attr(d, base, slir::A_EACH));
    if decoded.tag == slir::T_NUM {
        let param = truncate_i32(decoded.num);
        if param < 0 {
            return -1;
        }
        let list = root_id(d, s, unsigned(param));
        return if list == NONE { -1 } else { signed(list) };
    }
    if decoded.tag != slir::T_PROP_REF {
        return -1;
    }
    let outer = param_of(s, d, each);
    let item = item_ix(s, d, each);
    if outer < 0 || item < 0 {
        return -1;
    }
    let child = child_id(s, unsigned(outer), item, decoded.h);
    if child == NONE { -1 } else { signed(child) }
}

/// Reports whether a concrete list contains a key.
pub fn key_exists(d: &slir::Doc, s: &State, list: u32, key: &str) -> bool {
    item_index_for_key(d, s, list, key) >= 0
}

/// Removes a synthetic node slot without preserving storage order.
pub fn remove_sy(s: &mut State, k: i32) {
    let slot = index(k);
    let id = s.sy_id[slot];
    let identity = (s.sy_each[slot], s.sy_tpl[slot]);
    let key = s.sy_key[slot].clone();
    let mut remove_identity = false;
    if let Some(keys) = s.sy_identity.get_mut(&identity) {
        keys.remove(&key);
        remove_identity = keys.is_empty();
    }
    if remove_identity {
        s.sy_identity.remove(&identity);
    }
    s.sy_slot.remove(&id);
    s.sy_deleted.push(id);
    s.sy_id.swap_remove(slot);
    s.sy_each.swap_remove(slot);
    s.sy_tpl.swap_remove(slot);
    s.sy_key.swap_remove(slot);
    s.sy_item.swap_remove(slot);
    s.sy_list.swap_remove(slot);
    if slot < s.sy_id.len() {
        s.sy_slot.insert(s.sy_id[slot], slot);
    }
}

/// Drops synthetic identities only when their owning data item was truncated.
pub fn prune(d: &slir::Doc, s: &mut State) {
    if !s.prune_pending {
        return;
    }
    s.prune_pending = false;
    loop {
        let mut changed = false;
        let mut slot = len_i32(&s.sy_id) - 1;
        while slot >= 0 {
            let each = s.sy_each[index(slot)];
            let list = each_list(d, s, each);
            if list < 0 || !key_exists(d, s, unsigned(list), &s.sy_key[index(slot)]) {
                remove_sy(s, slot);
                changed = true;
            }
            slot -= 1;
        }
        if !changed {
            break;
        }
    }
}

fn set_len_id(d: &slir::Doc, s: &mut State, list: u32, n: i32) -> i32 {
    let slot = slot_by_id(s, list);
    if slot < 0 || n < 0 {
        return -1;
    }
    let old = s.li_len[index(slot)];
    if old == n {
        return 0;
    }
    if n > old {
        s.li_len[index(slot)] = n;
        for item in old..n {
            seed_item_id(d, s, list, item, -1);
        }
    } else {
        // Recursive host setters commonly rewrite keys and child lengths in
        // several calls. Defer identity pruning until begin_solve observes the
        // complete batch, while removing the truncated data immediately.
        s.prune_pending = true;
        let children: Vec<u32> = s
            .li_id
            .iter()
            .enumerate()
            .filter_map(|(child_slot, &id)| {
                (s.li_owner[child_slot] == list && s.li_owner_index[child_slot] >= n).then_some(id)
            })
            .collect();
        for child in children {
            remove_list(s, child);
        }
        let mut value_slot = len_i32(&s.lv_param) - 1;
        while value_slot >= 0 {
            let ix = index(value_slot);
            if s.lv_param[ix] == list && s.lv_index[ix] >= n {
                remove_value(s, value_slot);
            }
            value_slot -= 1;
        }
        let mut key_slot = len_i32(&s.lk_param) - 1;
        while key_slot >= 0 {
            let ix = index(key_slot);
            if s.lk_param[ix] == list && s.lk_index[ix] >= n {
                remove_key(s, key_slot);
            }
            key_slot -= 1;
        }
        let slot = slot_by_id(s, list);
        s.li_len[index(slot)] = n;
    }
    1
}

/// Changes a root list length.
pub fn set_len(d: &slir::Doc, s: &mut State, param: u32, n: i32) -> i32 {
    let list = root_id(d, s, param);
    if list == NONE {
        -1
    } else {
        set_len_id(d, s, list, n)
    }
}

/// Changes a root or nested list length selected by a validated path.
pub fn set_len_path(d: &slir::Doc, s: &mut State, param: u32, path: &str, n: i32) -> i32 {
    let list = resolve_path(d, s, param, path);
    if list == NONE {
        -1
    } else {
        set_len_id(d, s, list, n)
    }
}

fn set_field_id(
    d: &slir::Doc,
    s: &mut State,
    list: u32,
    item_index: i32,
    field: &str,
    v: &Val,
) -> i32 {
    if item_index < 0 || item_index >= length(d, s, list) {
        return -1;
    }
    let field_ix = field_ix(d, s, list, field);
    if field_ix < 0 || d.list_field_type[index(field_ix)] != v.kind || v.kind == 6 {
        return -1;
    }
    if v.kind == 5 {
        let lo = d.list_field_enum_off[index(field_ix)];
        let hi = lo.wrapping_add(d.list_field_enum_len[index(field_ix)]);
        if !(lo..hi).any(|symbol| slir::str_at(d, d.list_enum_syms[index(symbol)]) == v.sym) {
            return -1;
        }
    }
    let mut normalized = empty_val(v.kind);
    match v.kind {
        0 => normalized.s.clone_from(&v.s),
        1 | 2 => normalized.num = v.num,
        3 => normalized.rgba = v.rgba,
        4 => normalized.num = if v.num == 0.0 { 0.0 } else { 1.0 },
        5 => normalized.sym.clone_from(&v.sym),
        _ => return -1,
    }
    let schema = schema_for_list(s, list);
    let relative = field_ix.wrapping_sub(d.list_field_off[index(schema)]);
    if store(s, list, item_index, unsigned(relative), &normalized) {
        1
    } else {
        0
    }
}

/// Changes one typed scalar field on a root-list item.
pub fn set_field(
    d: &slir::Doc,
    s: &mut State,
    param: u32,
    item_index: i32,
    field: &str,
    v: &Val,
) -> i32 {
    let list = root_id(d, s, param);
    if list == NONE {
        -1
    } else {
        set_field_id(d, s, list, item_index, field, v)
    }
}

/// Changes one typed scalar field on an item in a path-selected list.
pub fn set_field_path(
    d: &slir::Doc,
    s: &mut State,
    param: u32,
    path: &str,
    item_index: i32,
    field: &str,
    v: &Val,
) -> i32 {
    let list = resolve_path(d, s, param, path);
    if list == NONE {
        -1
    } else {
        set_field_id(d, s, list, item_index, field, v)
    }
}

fn set_key_id(d: &slir::Doc, s: &mut State, list: u32, item_index: i32, key: &str) -> i32 {
    if item_index < 0 || item_index >= length(d, s, list) || key.is_empty() {
        return -1;
    }
    if key_at(d, s, list, item_index) == key {
        return 0;
    }
    s.prune_pending = true;
    note_lookup();
    if let Some(&slot) = s.lk_slot.get(&(list, item_index)) {
        let old_key = s.lk_key[slot].clone();
        remove_key_lookup(s, list, item_index, &old_key);
        s.lk_key[slot] = key.to_owned();
        add_key_lookup(s, list, item_index, key);
        return 1;
    }
    let slot = s.lk_param.len();
    s.lk_slot.insert((list, item_index), slot);
    add_key_lookup(s, list, item_index, key);
    s.lk_param.push(list);
    s.lk_index.push(item_index);
    s.lk_key.push(key.to_owned());
    1
}

/// Assigns a stable key to a root-list item.
pub fn set_key(d: &slir::Doc, s: &mut State, param: u32, item_index: i32, key: &str) -> i32 {
    let list = root_id(d, s, param);
    if list == NONE {
        -1
    } else {
        set_key_id(d, s, list, item_index, key)
    }
}

/// Assigns a stable key to an item in a path-selected list.
pub fn set_key_path(
    d: &slir::Doc,
    s: &mut State,
    param: u32,
    path: &str,
    item_index: i32,
    key: &str,
) -> i32 {
    let list = resolve_path(d, s, param, path);
    if list == NONE {
        -1
    } else {
        set_key_id(d, s, list, item_index, key)
    }
}

/// Returns a stored scalar field value or the empty sentinel when absent.
pub fn get(_d: &slir::Doc, s: &State, list: u32, item_index: i32, field: u32) -> Val {
    note_lookup();
    let Some(&slot) = s.lv_slot.get(&(list, item_index, field)) else {
        return empty_val(0);
    };
    Val {
        kind: s.lv_kind[slot],
        num: s.lv_num[slot],
        s: s.lv_str[slot].clone(),
        rgba: s.lv_h[slot],
        sym: s.lv_sym[slot].clone(),
    }
}

/// Resets list state from document schemas and recursive default items.
pub fn init(d: &slir::Doc, s: &mut State) {
    *s = state_new();
    s.li_next = u32::try_from(d.parm_type.len()).expect("parameter count exceeds u32");
    s.sy_next = u32::try_from(d.node_kind.len()).expect("node count exceeds u32");
    for param_ix in 0..d.parm_type.len() {
        if d.parm_type[param_ix] != 6 {
            continue;
        }
        let param = u32::try_from(param_ix).expect("parameter index exceeds u32");
        let schema = schema_ix(d, param);
        if schema < 0 {
            continue;
        }
        push_list(s, param, param, unsigned(schema), NONE, -1, NONE);
        let encoded = signed(d.parm_default[param_ix]);
        let n = list_default_len(d, encoded);
        let slot = slot_by_id(s, param);
        s.li_len[index(slot)] = n;
        for item in 0..n {
            seed_item_id(d, s, param, item, default_item_from(d, encoded, item));
        }
    }
    sync(d, s);
}

fn synthetic_slot(s: &State, node: u32) -> Option<usize> {
    note_lookup();
    let &slot = s.sy_slot.get(&node)?;
    (s.sy_id.get(slot) == Some(&node)).then_some(slot)
}

/// Resolves a synthetic node to its document template; document nodes are unchanged.
pub fn base(s: &State, d: &slir::Doc, node: u32) -> u32 {
    if signed(node) >= 0 && signed(node) < len_i32(&d.node_kind) {
        return node;
    }
    synthetic_slot(s, node).map_or(slir::NONE, |slot| s.sy_tpl[slot])
}

/// Returns the concrete each instance owning a synthetic node.
pub fn each_of(s: &State, d: &slir::Doc, node: u32) -> u32 {
    if signed(node) >= 0 && signed(node) < len_i32(&d.node_kind) {
        return slir::NONE;
    }
    synthetic_slot(s, node).map_or(slir::NONE, |slot| s.sy_each[slot])
}

/// Returns the innermost stable item key represented by a synthetic node.
pub fn item_key(s: &State, d: &slir::Doc, node: u32) -> String {
    if signed(node) >= 0 && signed(node) < len_i32(&d.node_kind) {
        return String::new();
    }
    synthetic_slot(s, node).map_or_else(String::new, |slot| s.sy_key[slot].clone())
}

/// Returns the current innermost item index represented by a synthetic node.
pub fn item_ix(s: &State, _d: &slir::Doc, node: u32) -> i32 {
    synthetic_slot(s, node).map_or(-1, |slot| s.sy_item[slot])
}

/// Returns the concrete list handle for a synthetic node, or `-1` otherwise.
pub fn param_of(s: &State, _d: &slir::Doc, node: u32) -> i32 {
    synthetic_slot(s, node).map_or(-1, |slot| signed(s.sy_list[slot]))
}

fn synthetic_identity(s: &mut State, each: u32, tpl: u32, key: &str) -> (u32, usize) {
    let identity = (each, tpl);
    note_lookup();
    let existing = s
        .sy_identity
        .get(&identity)
        .and_then(|keys| keys.get(key))
        .copied();
    if let Some(id) = existing {
        if let Some(slot) = synthetic_slot(s, id) {
            return (id, slot);
        }
        let mut remove_identity = false;
        if let Some(keys) = s.sy_identity.get_mut(&identity) {
            keys.remove(key);
            remove_identity = keys.is_empty();
        }
        if remove_identity {
            s.sy_identity.remove(&identity);
        }
        s.sy_slot.remove(&id);
    }
    let id = s.sy_next;
    s.sy_next = s.sy_next.wrapping_add(1);
    let slot = s.sy_id.len();
    s.sy_slot.insert(id, slot);
    s.sy_identity
        .entry(identity)
        .or_default()
        .insert(key.to_owned(), id);
    s.sy_id.push(id);
    s.sy_each.push(each);
    s.sy_tpl.push(tpl);
    s.sy_key.push(key.to_owned());
    s.sy_item.push(-1);
    s.sy_list.push(NONE);
    (id, slot)
}

/// Returns or creates the stable identity for one each instance, template, and item key.
pub fn synthetic(_d: &slir::Doc, s: &mut State, each: u32, tpl: u32, key: &str) -> u32 {
    synthetic_identity(s, each, tpl, key).0
}

fn synthetic_at(s: &mut State, each: u32, tpl: u32, key: &str, list: u32, item: i32) -> u32 {
    let (node, slot) = synthetic_identity(s, each, tpl, key);
    s.sy_item[slot] = item;
    s.sy_list[slot] = list;
    node
}

fn mark_materialized(s: &mut State, node: u32) {
    if s.sy_materialized_set.insert(node) {
        s.sy_materialized.push(node);
    }
}

fn sync_template(
    d: &slir::Doc,
    s: &mut State,
    each: u32,
    tpl: u32,
    key: &str,
    list: u32,
    item: i32,
) {
    let node = synthetic_at(s, each, tpl, key, list, item);
    mark_materialized(s, node);
    if d.node_kind[tpl as usize] == slir::K_EACH {
        sync_each(d, s, node);
        return;
    }
    let mut child = d.node_first[tpl as usize];
    while child != slir::NONE {
        sync_template(d, s, each, child, key, list, item);
        child = d.node_next[child as usize];
    }
    for (patch, &owner) in d.patch_node.iter().enumerate() {
        if owner != tpl {
            continue;
        }
        let start = d.patch_child_off[patch];
        let end = start.wrapping_add(d.patch_child_len[patch]);
        for patch_child in start..end {
            sync_template(
                d,
                s,
                each,
                d.patch_children[patch_child as usize],
                key,
                list,
                item,
            );
        }
    }
}

fn sync_each(d: &slir::Doc, s: &mut State, each: u32) {
    let base = base(s, d, each);
    let base_ix = match usize::try_from(base) {
        Ok(base_ix) if d.node_kind.get(base_ix) == Some(&slir::K_EACH) => base_ix,
        _ => return,
    };
    let list = each_list(d, s, each);
    if list < 0 {
        return;
    }
    let list = unsigned(list);
    let list_len = length(d, s, list);
    let range = if d.node_flags[base_ix] & slir::F_VIRTUAL != 0 {
        let current = current_window(s, each);
        if current == (0, 0) && list_len > 0 {
            materialized_window(d, s, each, 0.0).unwrap_or((0, 0))
        } else {
            current
        }
    } else {
        (0, list_len)
    };
    for item in range.0..range.1 {
        let key = key_at(d, s, list, item);
        let mut template = template_first(d, s, each);
        while template != slir::NONE {
            sync_template(d, s, each, template, &key, list, item);
            template = d.node_next[template as usize];
        }
    }
}

/// Materializes recursive identities needed before motion sampling without
/// pruning de-windowed items.
pub fn sync(d: &slir::Doc, s: &mut State) {
    s.sy_materialized.clear();
    s.sy_materialized_set.clear();
    for each_ix in 0..d.node_kind.len() {
        if d.node_kind[each_ix] == slir::K_EACH
            && d.node_parent[each_ix] != slir::NONE
            && d.node_flags[each_ix] & slir::F_DETACHED != 0
        {
            continue;
        }
        if d.node_kind[each_ix] == slir::K_EACH {
            sync_each(
                d,
                s,
                u32::try_from(each_ix).expect("node index exceeds u32"),
            );
        }
    }
}

/// Returns synthetic identities belonging to the current materialized windows.
pub fn materialized(s: &State) -> &[u32] {
    &s.sy_materialized
}

/// Takes identities removed by the latest data deletion or truncation.
pub fn take_deleted_synthetic(s: &mut State) -> Vec<u32> {
    std::mem::take(&mut s.sy_deleted)
}

fn base_num_attr(d: &slir::Doc, node: u32, attr: u32, default: f64) -> f64 {
    let decoded = value::decode(d, slir::base_attr(d, node, attr));
    if decoded.tag == slir::T_NUM {
        decoded.num
    } else {
        default
    }
}

/// Returns virtual-list extent, overscan, and authored scroll parent.
pub fn virtual_config(d: &slir::Doc, s: &State, each: u32) -> Option<(f64, i32, u32)> {
    let base = base(s, d, each);
    let base_ix = usize::try_from(base).ok()?;
    if d.node_kind.get(base_ix) != Some(&slir::K_EACH)
        || d.node_flags.get(base_ix).copied().unwrap_or(0) & slir::F_VIRTUAL == 0
    {
        return None;
    }
    let extent = base_num_attr(d, base, slir::A_ITEM_EXTENT, 0.0);
    if !extent.is_finite() || extent <= 0.0 {
        return None;
    }
    let overscan = truncate_i32(base_num_attr(d, base, slir::A_OVERSCAN, 4.0)).max(0);
    Some((extent, overscan, d.node_parent[base_ix]))
}

fn window_slot(s: &State, each: u32) -> i32 {
    s.win_each
        .iter()
        .position(|candidate| *candidate == each)
        .map_or(-1, |slot| {
            i32::try_from(slot).expect("window slot exceeds i32")
        })
}

/// Returns the last materialized half-open range for a virtual each.
pub fn current_window(s: &State, each: u32) -> (i32, i32) {
    let slot = window_slot(s, each);
    if slot < 0 {
        (0, 0)
    } else {
        (s.win_start[index(slot)], s.win_end[index(slot)])
    }
}
/// Returns a virtual each's exact logical extent inputs and current window.
pub fn virtual_metrics(d: &slir::Doc, s: &State, each: u32) -> Option<(f64, i32, i32, i32)> {
    let (extent, _, _) = virtual_config(d, s, each)?;
    let list = each_list(d, s, each);
    if list < 0 {
        return None;
    }
    let (start, end) = current_window(s, each);
    Some((extent, length(d, s, unsigned(list)), start, end))
}

fn viewport_for(s: &State, each: u32) -> f64 {
    let slot = window_slot(s, each);
    if slot < 0 {
        0.0
    } else {
        s.win_viewport[index(slot)]
    }
}

/// Returns the retained main-axis content origin for a virtual each.
pub fn virtual_origin(s: &State, each: u32) -> f64 {
    let slot = window_slot(s, each);
    if slot < 0 {
        0.0
    } else {
        s.win_origin[index(slot)]
    }
}

fn compute_window(len: i32, extent: f64, overscan: i32, off: f64, viewport: f64) -> (i32, i32) {
    if len <= 0 {
        return (0, 0);
    }
    if viewport <= 0.0 {
        return (0, len.min(overscan.saturating_mul(2)));
    }
    let visible_start = off.max(0.0);
    let visible_end = (off + viewport).max(0.0);
    let first = truncate_i32((visible_start / extent).floor());
    let last = truncate_i32((visible_end / extent).ceil());
    (
        first.saturating_sub(overscan).clamp(0, len),
        last.saturating_add(overscan).clamp(0, len),
    )
}

/// Computes and records the current range from retained viewport and scroll offset.
pub fn materialized_window(
    d: &slir::Doc,
    s: &mut State,
    each: u32,
    off: f64,
) -> Option<(i32, i32)> {
    let (extent, overscan, _) = virtual_config(d, s, each)?;
    let list = each_list(d, s, each);
    if list < 0 {
        return None;
    }
    let range = compute_window(
        length(d, s, unsigned(list)),
        extent,
        overscan,
        off - virtual_origin(s, each),
        viewport_for(s, each),
    );
    let slot = window_slot(s, each);
    if slot < 0 {
        s.win_each.push(each);
        s.win_start.push(range.0);
        s.win_end.push(range.1);
        s.win_viewport.push(0.0);
        s.win_origin.push(0.0);
    } else {
        s.win_start[index(slot)] = range.0;
        s.win_end[index(slot)] = range.1;
    }
    Some(range)
}

/// Updates retained viewport geometry and reports whether the range changes.
pub fn set_virtual_viewport(
    d: &slir::Doc,
    s: &mut State,
    each: u32,
    viewport: f64,
    off: f64,
    origin: f64,
) -> bool {
    let Some((extent, overscan, _)) = virtual_config(d, s, each) else {
        return false;
    };
    let list = each_list(d, s, each);
    if list < 0 {
        return false;
    }
    let viewport = viewport.max(0.0);
    let origin = if origin.is_finite() { origin } else { 0.0 };
    let old = current_window(s, each);
    let new = compute_window(
        length(d, s, unsigned(list)),
        extent,
        overscan,
        off - origin,
        viewport,
    );
    let slot = window_slot(s, each);
    if slot < 0 {
        s.win_each.push(each);
        s.win_start.push(new.0);
        s.win_end.push(new.1);
        s.win_viewport.push(viewport);
        s.win_origin.push(origin);
    } else {
        s.win_start[index(slot)] = new.0;
        s.win_end[index(slot)] = new.1;
        s.win_viewport[index(slot)] = viewport;
        s.win_origin[index(slot)] = origin;
    }
    old != new
}

#[cfg(test)]
mod index_tests {
    use super::*;

    #[test]
    fn later_virtual_root_lookup_ignores_prior_recursive_state() {
        let mut d = slir::doc_new();
        d.parm_type.extend([6, 6]);
        let mut state = state_new();
        push_list(&mut state, 0, 0, 0, NONE, -1, NONE);
        for item in 0..10_000 {
            push_list(
                &mut state,
                u32::try_from(item + 2).expect("fixture list id fits u32"),
                NONE,
                0,
                0,
                item,
                0,
            );
        }
        // Root IDs are parameter IDs even when recursive descendants placed
        // the later virtual-list root at the end of the flat storage vectors.
        push_list(&mut state, 1, 1, 1, NONE, -1, NONE);

        reset_lookup_work();
        assert_eq!(root_id(&d, &state, 1), 1);
        assert_eq!(
            lookup_work(),
            1,
            "root lookup performs one indexed probe regardless of prior descendants"
        );
    }
}
