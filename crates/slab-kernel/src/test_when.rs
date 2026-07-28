//! Condition evaluation across environment changes, state resolution with
//! boolean parameter overrides, client equality, width and height boundaries,
//! and negation.

use crate::{
	dispatch, motion, scene,
	slir::{self, Doc},
	style,
	when::{self, Env},
};

/// Builds the condition table shared by the evaluator tests.
///
/// String symbols are: 0 `""`, 1 `"dark"`, 2 `"portrait"`, 3 `"hot"`,
/// 4 `"tui"`, 5 `"landscape"`, 6 `"coarse"`, and 7 `"dusk"`.
///
/// Conditions are: 0 `Env(dark)`, 1 `Env(portrait)`, 2 `State(hot)`,
/// 3 `Client(tui)`, 4 `WCmp(< 600)`, 5 `HCmp(>= 400)`, 6 `!State(hot)`,
/// 7 `Env(landscape)`, 8 `Env(coarse)`, 9 `WCmp(<= 600)`,
/// 10 `WCmp(> 600)`, 11 `WCmp(== 600)`, and 12 `Theme(dusk)`.
pub fn fixture() -> Doc {
	let mut doc = slir::doc_new();
	doc.strs.extend(
		["", "dark", "portrait", "hot", "tui", "landscape", "coarse", "dusk"].map(str::to_owned),
	);

	let conditions = [
		(1, 0, 0, 0.0, 1),
		(1, 0, 0, 0.0, 2),
		(0, 0, 0, 0.0, 3),
		(2, 0, 0, 0.0, 4),
		(3, 0, 0, 600.0, 0),
		(4, 0, 3, 400.0, 0),
		(0, 1, 0, 0.0, 3),
		(1, 0, 0, 0.0, 5),
		(1, 0, 0, 0.0, 6),
		(3, 0, 1, 600.0, 0),
		(3, 0, 2, 600.0, 0),
		(3, 0, 4, 600.0, 0),
		(6, 0, 0, 0.0, 7),
	];
	for (kind, negated, op, number, symbol) in conditions {
		doc.cond_kind.push(kind);
		doc.cond_neg.push(negated);
		doc.cond_op.push(op);
		doc.cond_num.push(number);
		doc.cond_sym.push(symbol);
	}
	doc
}

/// Constructs an environment with the base theme.
pub const fn ev(vw: f64, vh: f64, client: u32, dark: bool, coarse: bool) -> Env {
	Env { vw, vh, client, dark, coarse, theme: String::new() }
}

/// Verifies dark, orientation, and coarse-pointer environment conditions.
pub fn test_env_conds() {
	let doc = fixture();
	let states = Vec::new();
	let param_values = Vec::new();

	let dark = ev(800.0, 600.0, 0, true, false);
	assert!(when::eval_cond(&doc, 0, 0, &dark, &states, &param_values, 0.0, 0.0), "dark on");

	let light = ev(800.0, 600.0, 0, false, false);
	assert!(!when::eval_cond(&doc, 0, 0, &light, &states, &param_values, 0.0, 0.0), "dark off");
	assert!(
		!when::eval_cond(&doc, 1, 0, &light, &states, &param_values, 0.0, 0.0),
		"800x600 not portrait"
	);
	assert!(
		when::eval_cond(&doc, 7, 0, &light, &states, &param_values, 0.0, 0.0),
		"800x600 landscape"
	);

	let portrait = ev(400.0, 800.0, 0, false, false);
	assert!(
		when::eval_cond(&doc, 1, 0, &portrait, &states, &param_values, 0.0, 0.0),
		"400x800 portrait"
	);
	assert!(
		!when::eval_cond(&doc, 7, 0, &portrait, &states, &param_values, 0.0, 0.0),
		"400x800 not landscape"
	);
	assert!(!when::eval_cond(&doc, 8, 0, &light, &states, &param_values, 0.0, 0.0), "coarse off");
	assert!(
		when::eval_cond(&doc, 8, 0, &ev(1.0, 1.0, 0, false, true), &states, &param_values, 0.0, 0.0),
		"coarse on"
	);
}

/// Verifies positive and negated state conditions with and without an active
/// state.
pub fn test_state_conds() {
	let doc = fixture();
	let env = ev(800.0, 600.0, 0, false, false);
	let no_states = Vec::new();
	let param_values = Vec::new();
	assert!(!when::eval_cond(&doc, 2, 0, &env, &no_states, &param_values, 0.0, 0.0), "hot inactive");
	assert!(when::eval_cond(&doc, 6, 0, &env, &no_states, &param_values, 0.0, 0.0), "!hot active");

	let hot = vec![3];
	assert!(when::eval_cond(&doc, 2, 0, &env, &hot, &param_values, 0.0, 0.0), "hot via states");
	assert!(!when::eval_cond(&doc, 6, 0, &env, &hot, &param_values, 0.0, 0.0), "!hot inactive");
}

/// Verifies that a boolean parameter takes precedence over the global state
/// set.
pub fn test_bool_param_override() {
	let mut doc = fixture();
	// Parameter "hot" (symbol 3), type Bool, current value 0: it overrides the
	// global state set even when the state is present.
	doc.parm_name.push(3);
	doc.parm_type.push(4);
	doc.parm_default.push(0);
	doc.parm_enum_off.push(0);
	doc.parm_enum_len.push(0);
	doc.parm_site_off.push(0);
	doc.parm_site_len.push(0);

	let env = ev(800.0, 600.0, 0, false, false);
	let hot = vec![3];
	let false_param = vec![0.0];
	assert!(
		!when::eval_cond(&doc, 2, 0, &env, &hot, &false_param, 0.0, 0.0),
		"bool param 0 wins over state"
	);

	let true_param = vec![1.0];
	let no_states = Vec::new();
	assert!(
		when::eval_cond(&doc, 2, 0, &env, &no_states, &true_param, 0.0, 0.0),
		"bool param 1 activates"
	);
}

/// Verifies that client conditions compare against the encoded client value.
pub fn test_client_cond() {
	let doc = fixture();
	let states = Vec::new();
	let param_values = Vec::new();
	assert!(
		when::eval_cond(&doc, 3, 0, &ev(1.0, 1.0, 2, false, false), &states, &param_values, 0.0, 0.0),
		"client tui matches 2"
	);
	assert!(
		!when::eval_cond(
			&doc,
			3,
			0,
			&ev(1.0, 1.0, 1, false, false),
			&states,
			&param_values,
			0.0,
			0.0
		),
		"client gpu is not tui"
	);
}

/// Verifies theme matching for the base and named themes.
pub fn test_theme_cond() {
	let doc = fixture();
	let states = Vec::new();
	let param_values = Vec::new();
	let mut env = ev(1.0, 1.0, 0, false, false);
	assert!(
		!when::eval_cond(&doc, 12, 0, &env, &states, &param_values, 0.0, 0.0),
		"base is not dusk"
	);
	env.theme.clear();
	env.theme.push_str("dusk");
	assert!(when::eval_cond(&doc, 12, 0, &env, &states, &param_values, 0.0, 0.0), "dusk matches");
}

/// Verifies exact and adjacent width and height comparison boundaries.
pub fn test_wcmp_boundaries() {
	let doc = fixture();
	let env = ev(800.0, 600.0, 0, false, false);
	let states = Vec::new();
	let param_values = Vec::new();

	// Condition 4 is `w < 600`, evaluated against the incoming constraint.
	assert!(
		when::eval_cond(&doc, 4, 0, &env, &states, &param_values, 599.999, 0.0),
		"w<600 at 599.999"
	);
	assert!(
		!when::eval_cond(&doc, 4, 0, &env, &states, &param_values, 600.0, 0.0),
		"w<600 at 600 exact"
	);

	// Condition 9 is `w <= 600`.
	assert!(when::eval_cond(&doc, 9, 0, &env, &states, &param_values, 600.0, 0.0), "w<=600 at 600");
	assert!(
		!when::eval_cond(&doc, 9, 0, &env, &states, &param_values, 600.001, 0.0),
		"w<=600 above"
	);

	// Condition 10 is `w > 600`.
	assert!(when::eval_cond(&doc, 10, 0, &env, &states, &param_values, 600.001, 0.0), "w>600 above");
	assert!(!when::eval_cond(&doc, 10, 0, &env, &states, &param_values, 600.0, 0.0), "w>600 at 600");

	// Condition 11 is `w == 600` (reserved operator 4).
	assert!(when::eval_cond(&doc, 11, 0, &env, &states, &param_values, 600.0, 0.0), "w==600 exact");
	assert!(!when::eval_cond(&doc, 11, 0, &env, &states, &param_values, 599.0, 0.0), "w==600 miss");

	// Condition 5 is `h >= 400`.
	assert!(when::eval_cond(&doc, 5, 0, &env, &states, &param_values, 0.0, 400.0), "h>=400 at 400");
	assert!(
		!when::eval_cond(&doc, 5, 0, &env, &states, &param_values, 0.0, 399.999),
		"h>=400 below"
	);
}

/// Verifies every supported client code and rejection of the removed GUI
/// client.
pub fn test_client_code() {
	assert_eq!(when::client_code("web"), 0, "web");
	assert_eq!(when::client_code("gpu"), 1, "gpu");
	assert_eq!(when::client_code("tui"), 2, "tui");
	assert_eq!(when::client_code("svg"), 3, "svg");
	assert_eq!(when::client_code("png"), 4, "png");
	assert_eq!(when::client_code("gui"), -1, "gui is gone (rule 6)");
}

fn gated_interactivity_doc() -> Doc {
	let mut doc = slir::doc_new();
	doc.strs
		.extend(["", "enabled", "fire", "pulse"].map(str::to_owned));
	doc.node_kind.push(slir::K_RECT);
	doc.node_flags.push(0);
	doc.node_parent.push(slir::NONE);
	doc.node_first.push(slir::NONE);
	doc.node_next.push(slir::NONE);
	doc.node_key.push(0);
	doc.node_id.push(0);
	doc.node_line.push(1);
	doc.attr_index.extend([0, 0]);

	doc.cond_kind.push(slir::C_STATE);
	doc.cond_neg.push(0);
	doc.cond_op.push(0);
	doc.cond_num.push(0.0);
	doc.cond_sym.push(1);
	doc.patch_node.push(0);
	doc.patch_cond.push(0);
	doc.patch_attr_off.push(0);
	doc.patch_attr_len.push(3);
	doc.patch_child_off.push(0);
	doc.patch_child_len.push(0);

	let signal = doc.aval_tag.len() as u32;
	doc.aval_tag.push(slir::T_STR);
	doc.aval_lo.push(2);
	doc.aval_hi.push(0);
	doc.aval_num.push(0.0);
	let flags = doc.aval_tag.len() as u32;
	doc.aval_tag.push(slir::T_NUM);
	doc.aval_lo.push(0);
	doc.aval_hi.push(0);
	doc.aval_num.push(f64::from(slir::F_FOCUSABLE));
	let animation = doc.aval_tag.len() as u32;
	doc.aval_tag.push(slir::T_STR);
	doc.aval_lo.push(3);
	doc.aval_hi.push(0);
	doc.aval_num.push(0.0);
	doc.wattr_id
		.extend([slir::A_ACT, slir::A_FLAGS, slir::A_ANIMATE]);
	doc.wattr_val.extend([signal, flags, animation]);

	doc.sign_name.push(2);
	doc.sign_node.push(0);
	doc.sign_trigger.push(dispatch::TR_ACTIVATE);
	doc.anim_name.push(3);
	doc.anim_stop_off.push(0);
	doc.anim_stop_len.push(0);
	doc.bind_node.push(0);
	doc.bind_anim.push(0);
	doc.bind_dur.push(1_000.0);
	doc.bind_mode.push(0);
	doc.bind_easing.push(0);
	doc.bind_delay.push(0.0);
	doc
}

fn gated_style(doc: &Doc, enabled: bool) -> style::St {
	let mut st = style::st_new();
	style::init_params(doc, &mut st);
	if enabled {
		st.states.push(1);
	}
	style::begin_solve(doc, &mut st);
	st
}

/// Verifies that a conditional signal fires only while its patch condition
/// holds.
pub fn test_conditional_signal_firing() {
	let doc = gated_interactivity_doc();
	let mut effects = dispatch::effects_new();
	let inactive = gated_style(&doc, false);
	assert!(
		!dispatch::deliver_trigger(&doc, &inactive, &mut effects, 0, dispatch::TR_ACTIVATE, "",),
		"false condition suppresses dispatch"
	);
	let active = gated_style(&doc, true);
	assert!(dispatch::deliver_trigger(&doc, &active, &mut effects, 0, dispatch::TR_ACTIVATE, "",));
	assert_eq!(effects.sig_name, [2], "true condition emits its signal");
}

/// Verifies that a false conditional binder removes its node from the tab ring.
pub fn test_conditional_binder_tab_exclusion() {
	let doc = gated_interactivity_doc();
	for (enabled, expected) in [(false, Vec::new()), (true, vec![0])] {
		let st = gated_style(&doc, enabled);
		let mut retained = scene::scene_new();

		retained.entries.push(crate::flatten::SceneNode {
			node: 0,
			parent_ix: -1,
			w: 100.0,
			h: 100.0,
			flags: style::eff_flags(&doc, &st, 0),
			src_line: 1,
			..Default::default()
		});
		retained.authored_order.push(0);
		let mut focusable = Vec::new();
		scene::focusables(&retained, &mut focusable);
		assert_eq!(focusable, expected);
	}
}

/// Verifies that deactivating a field binder preserves the authored node's edit
/// buffer.
pub fn test_conditional_binder_retains_edit_state() {
	let doc = gated_interactivity_doc();
	let mut inactive = gated_style(&doc, false);
	let mut dispatch_state = dispatch::dstate_new();
	dispatch_state.ed_node.push(0);
	dispatch_state
		.ed
		.push(crate::edit::es_new(0, "retained draft"));

	assert!(!dispatch::prune_vanished(&doc, &mut inactive, &mut dispatch_state));
	assert_eq!(dispatch_state.ed_node, [0]);
	assert_eq!(dispatch_state.ed[0].text, "retained draft");
}

/// Verifies that animation on a child declared under `when` follows parent
/// materialization.
pub fn test_conditional_child_animation_idle() {
	let mut doc = gated_interactivity_doc();
	doc.node_kind.push(slir::K_RECT);
	doc.node_flags.push(slir::F_DETACHED);
	doc.node_parent.push(0);
	doc.node_first.push(slir::NONE);
	doc.node_next.push(slir::NONE);
	doc.node_key.push(0);
	doc.node_id.push(0);
	doc.node_line.push(2);
	doc.attr_index.push(1);
	doc.attr_id.push(slir::A_ANIMATE);
	doc.attr_val.push(2);
	doc.patch_children.push(1);
	doc.patch_child_len[0] = 1;
	doc.bind_node[0] = 1;

	let mut inactive = gated_style(&doc, false);
	let mut ms = motion::mst_new();
	assert!(!motion::apply(&doc, &mut inactive, &mut ms, 100.0));
	assert!(!ms.active, "detached child permits idle repaint");

	let mut active = gated_style(&doc, true);
	assert!(motion::apply(&doc, &mut active, &mut ms, 100.0));
	assert!(ms.active, "materialized child runs its animation clock");
}

/// Verifies that a false animation condition leaves motion inactive.
pub fn test_conditional_animation_idle() {
	let doc = gated_interactivity_doc();
	let mut inactive = gated_style(&doc, false);
	let mut ms = motion::mst_new();
	assert!(!motion::apply(&doc, &mut inactive, &mut ms, 100.0));
	assert!(!ms.active, "false condition permits idle repaint");

	let mut active = gated_style(&doc, true);
	assert!(motion::apply(&doc, &mut active, &mut ms, 100.0));
	assert!(ms.active, "true condition runs the animation clock");
}
