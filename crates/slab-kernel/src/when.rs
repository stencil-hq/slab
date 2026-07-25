//! Evaluation of `when` conditions.
//!
//! Boolean parameters take precedence over document-wide states with the same
//! symbol. Per-node states, such as hover, pressed, and focus, are considered
//! only by the node-state-aware entry points. Width and height comparisons use
//! the incoming constraints, never a node's resolved size.

use crate::{list, slir};
use std::cmp::Ordering;

/// Values from the host environment that may be referenced by conditions.
#[derive(Clone, Debug)]
pub struct Env {
    /// Viewport width.
    pub vw: f64,
    /// Viewport height.
    pub vh: f64,
    /// Client kind: 0 web, 1 GPU, 2 TUI, 3 SVG, or 4 PNG.
    pub client: u32,
    /// Whether the host uses a dark color scheme.
    pub dark: bool,
    /// Whether the primary pointing device has coarse accuracy.
    pub coarse: bool,
    /// Active theme name.
    pub theme: String,
}

/// Returns an environment with web selected and all other values empty.
pub fn env_default() -> Env {
    Env {
        vw: 0.0,
        vh: 0.0,
        client: 0,
        dark: false,
        coarse: false,
        theme: String::new(),
    }
}

/// Returns the numeric code for a supported client name, or `-1` if unknown.
pub fn client_code(name: &str) -> i32 {
    match name {
        "web" => 0,
        "gpu" => 1,
        "tui" => 2,
        "svg" => 3,
        "png" => 4,
        _ => -1,
    }
}

/// Reports whether a symbol is enabled by a boolean parameter or global state.
///
/// A matching boolean parameter overrides the global state set. Unknown
/// symbols are inactive.
pub fn state_active(d: &slir::Doc, sym: u32, states: &[u32], pv_num: &[f64]) -> bool {
    for (p, &name) in d.parm_name.iter().enumerate() {
        if name == sym && d.parm_type[p] == 4 {
            return pv_num[p] != 0.0;
        }
    }
    states.contains(&sym)
}

/// Reports whether a symbol is enabled by parameters, global state, or a
/// per-node state overlay, in that precedence order.
pub fn state_active_ns(
    d: &slir::Doc,
    sym: u32,
    node: u32,
    states: &[u32],
    ns_node: &[u32],
    ns_sym: &[u32],
    pv_num: &[f64],
) -> bool {
    if state_active(d, sym, states, pv_num) {
        return true;
    }
    ns_node
        .iter()
        .enumerate()
        .any(|(i, &state_node)| state_node == node && ns_sym[i] == sym)
}

/// Applies an encoded condition comparison operator to two numbers.
///
/// Operator codes 0–3 mean `<`, `<=`, `>`, and `>=`; every other code means
/// equality.
pub fn cmp(base: f64, op: u32, num: f64) -> bool {
    match op {
        0 => base < num,
        1 => base <= num,
        2 => base > num,
        3 => base >= num,
        _ => base == num,
    }
}

/// Evaluates condition `ci` in the document-wide environment.
///
/// `cw` and `ch` are the node's incoming maximum constraints. Only width and
/// height comparisons read them; callers may pass viewport dimensions during
/// the global pass over state, environment, client, and theme conditions.
// These are the independent inputs to condition evaluation; bundling borrowed
// runtime state would obscure precedence at this public kernel boundary.
#[allow(clippy::too_many_arguments)]
pub fn eval_cond(
    d: &slir::Doc,
    ci: i32,
    _node: u32,
    env: &Env,
    states: &[u32],
    pv_num: &[f64],
    cw: f64,
    ch: f64,
) -> bool {
    let ci = usize::try_from(ci).expect("condition index must be nonnegative");
    let kind = d.cond_kind[ci];
    let sym = d.cond_sym[ci];
    let active = match kind {
        slir::C_STATE => state_active(d, sym, states, pv_num),
        slir::C_ENV => match slir::str_at(d, sym).as_str() {
            "portrait" => env.vw < env.vh,
            // This is deliberately the inverse of portrait. Expressing the
            // partial order explicitly keeps equal and unordered (NaN)
            // dimensions in the established landscape branch.
            "landscape" => env.vw.partial_cmp(&env.vh) != Some(Ordering::Less),
            "dark" => env.dark,
            "coarse" => env.coarse,
            _ => false,
        },
        slir::C_CLIENT => {
            let client = i32::from_ne_bytes(env.client.to_ne_bytes());
            client_code(&slir::str_at(d, sym)) == client
        }
        slir::C_THEME => env.theme == slir::str_at(d, sym),
        slir::C_WCMP => cmp(cw, d.cond_op[ci], d.cond_num[ci]),
        slir::C_HCMP => cmp(ch, d.cond_op[ci], d.cond_num[ci]),
        _ => false,
    };

    if d.cond_neg[ci] == 1 { !active } else { active }
}

/// Evaluates a condition with a per-node state overlay.
///
/// State conditions consult the node's own states after boolean parameters and
/// global states. Every other condition has the same behavior as [`eval_cond`].
// Node-state evaluation adds the two parallel overlay slices to the base
// condition inputs; keeping them explicit preserves their indexing contract.
#[allow(clippy::too_many_arguments)]
pub fn eval_cond_ns(
    d: &slir::Doc,
    ci: i32,
    node: u32,
    env: &Env,
    states: &[u32],
    ns_node: &[u32],
    ns_sym: &[u32],
    pv_num: &[f64],
    cw: f64,
    ch: f64,
) -> bool {
    let condition = usize::try_from(ci).expect("condition index must be nonnegative");
    if d.cond_kind[condition] != slir::C_STATE {
        return eval_cond(d, ci, node, env, states, pv_num, cw, ch);
    }

    let active = state_active_ns(
        d,
        d.cond_sym[condition],
        node,
        states,
        ns_node,
        ns_sym,
        pv_num,
    );
    if d.cond_neg[condition] == 1 {
        !active
    } else {
        active
    }
}

/// Evaluates a condition for a synthetic list item.
///
/// For property conditions, the condition symbol is the zero-based schema
/// field index. All other conditions retain normal per-node behavior.
// Item evaluation composes condition, node-state, list-state, and constraint
// inputs. The explicit boundary mirrors that domain operation without copies.
#[allow(clippy::too_many_arguments)]
pub fn eval_cond_item(
    d: &slir::Doc,
    ci: i32,
    node: u32,
    env: &Env,
    states: &[u32],
    ns_node: &[u32],
    ns_sym: &[u32],
    pv_num: &[f64],
    lists: &list::State,
    cw: f64,
    ch: f64,
) -> bool {
    let condition = usize::try_from(ci).expect("condition index must be nonnegative");
    if d.cond_kind[condition] != slir::C_PROP {
        return eval_cond_ns(d, ci, node, env, states, ns_node, ns_sym, pv_num, cw, ch);
    }

    let param = list::param_of(lists, d, node);
    let item = list::item_ix(lists, d, node);
    // Preserve the encoded two's-complement value passed by the generated
    // implementation; `get` is called even when no list parameter is found.
    let param_bits = u32::from_ne_bytes(param.to_ne_bytes());
    let value = list::get_ref(lists, param_bits, item, d.cond_sym[condition]);
    let active = if param < 0 || item < 0 {
        false
    } else {
        match value.kind {
            0 => !value.s.is_empty(),
            3 => value.rgba != 0,
            5 => !value.sym.is_empty(),
            _ => value.num != 0.0,
        }
    };

    if d.cond_neg[condition] == 1 {
        !active
    } else {
        active
    }
}
