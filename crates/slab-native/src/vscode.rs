//! `slab-native --demo vscode` — native Rust host for the VS Code demo.
//!
//! The scripted twin of `demos/vscode/web/main.js` over the same document and
//! contracts:
//! - multiEditorTabsControl.ts strip semantics: mousedown switch, midpoint
//!   reorder indicator, middle-click close, dblclick pin, X close, context menu
//!   (Close / Close Others / Close Saved / Pin / Close All);
//! - editorDropTarget.ts `DropOverlay` zones (10% edge gates, thirds/halves,
//!   half-rect preview via `zone-*` node states) and grid.ts addGroup splits
//!   (sibling insert / orientation wrap, Sizing.Split via pane splits);
//! - explorer tree, cross-file search, SCM changes from dirty editors,
//!   problems, terminal, find widget, chat sessions, navigation history, and VS
//!   Code pixel-pinned chrome sashes.
//!
//! Scene-dependent writes (editor seeding, split sizing, reveal) are queued
//! and drained at the top of [`ShellHost::effects`], which the shell invokes
//! right after each kernel solve: the scene is fresh there, the writes mark
//! the instance dirty, and the shell schedules the follow-up frame — the
//! native equivalent of the web host's staged `whenSettled` callbacks.
//!
//! Host-window keyboard chords (Cmd+S/F/J) and outside-click menu dismissal
//! are not ported: [`ShellHost`] deliberately exposes no raw input hooks, so
//! the menu closes on the next handled signal instead.

use std::{
	collections::{BTreeMap, BTreeSet},
	time::Instant,
};

use slab_kernel::{dispatch, edit::FieldStyle, frame as kframe, scene, slir};
use winit::event_loop::{ControlFlow, EventLoop};

use crate::{
	NativeDocument, demo,
	gen_vscode::{
		self as gv, ChatSessionsItem, MenuItemsItem, RootCrumbsItem, RootItem, RootTabsItem,
		ScmChangesItem, SearchResultsItem, Signal, TreeItem,
	},
	view::{NativeShell, ShellEvent, ShellHost, ShellOptions},
	vscode_fs,
};

const RED: &str = "#A63D5B";
const GRAY: &str = "#909094";
const LINE_H: f64 = 18.0;
/// Activity bar plus both vertical sash widths.
const CHROME_W: f64 = 55.0 + 4.0 + 4.0;
/// Titlebar, panel sash, statusbar, and bottom border heights.
const CHROME_H: f64 = 28.0 + 4.0 + 22.0 + 3.0;
const PANEL_FLOOR: f64 = 77.0;
const MDB_PATH: &str = "include/agentfs/mdb.hpp";

/// Parses `#RRGGBB` / `#RRGGBBAA` into the packed SLIR color word.
fn tint(hex: &str) -> gv::Rgba {
	let hex = hex.strip_prefix('#').unwrap_or(hex);
	let byte = |at: usize| u8::from_str_radix(hex.get(at..at + 2).unwrap_or("00"), 16).unwrap_or(0);
	let alpha = if hex.len() >= 8 { byte(6) } else { 0xff };
	gv::rgba(byte(0), byte(2), byte(4), alpha)
}

// ---------------------------------------------------------------- model ----

#[derive(Clone)]
struct Tab {
	key:     String,
	name:    String,
	note:    String,
	tint:    gv::Rgba,
	badge:   String,
	active:  bool,
	hot:     bool,
	preview: bool,
	dirty:   bool,
}

impl Tab {
	fn new(name: &str, tint_hex: &str, badge: &str, active: bool) -> Self {
		Self {
			key: name.to_owned(),
			name: name.to_owned(),
			note: "agentfs".to_owned(),
			tint: tint(tint_hex),
			badge: badge.to_owned(),
			active,
			hot: false,
			preview: false,
			dirty: false,
		}
	}
}

/// One VS Code gridview node: a leaf editor group or an oriented branch.
#[derive(Clone)]
struct Group {
	key:         String,
	leaf:        bool,
	horizontal:  bool,
	show_find:   bool,
	find_status: String,
	curline:     f64,
	curline_on:  bool,
	tabs:        Vec<Tab>,
	children:    Vec<Self>,
}

impl Group {
	fn leaf_node(key: &str, tabs: Vec<Tab>) -> Self {
		Self {
			key: key.to_owned(),
			leaf: true,
			horizontal: false,
			show_find: false,
			find_status: "No results".to_owned(),
			curline: 0.0,
			curline_on: false,
			tabs,
			children: Vec::new(),
		}
	}

	fn branch(key: &str, horizontal: bool, children: Vec<Self>) -> Self {
		Self {
			key: key.to_owned(),
			leaf: false,
			horizontal,
			show_find: false,
			find_status: "No results".to_owned(),
			curline: 0.0,
			curline_on: false,
			tabs: Vec::new(),
			children,
		}
	}
}

fn for_each_leaf_mut(node: &mut Group, visit: &mut impl FnMut(&mut Group)) {
	if node.leaf {
		visit(node);
		return;
	}
	for child in &mut node.children {
		for_each_leaf_mut(child, visit);
	}
}

fn find_leaf_mut<'tree>(node: &'tree mut Group, key: &str) -> Option<&'tree mut Group> {
	if node.leaf {
		return (node.key == key).then_some(node);
	}
	node
		.children
		.iter_mut()
		.find_map(|child| find_leaf_mut(child, key))
}

fn find_leaf<'tree>(node: &'tree Group, key: &str) -> Option<&'tree Group> {
	if node.leaf {
		return (node.key == key).then_some(node);
	}
	node.children.iter().find_map(|child| find_leaf(child, key))
}

/// Leaf key containing `item`, in VS Code's group order.
fn leaf_of(node: &Group, item: &str) -> Option<String> {
	if node.leaf {
		return node
			.tabs
			.iter()
			.any(|tab| tab.key == item)
			.then(|| node.key.clone());
	}
	node.children.iter().find_map(|child| leaf_of(child, item))
}

fn take_tab(node: &mut Group, item: &str) -> Option<Tab> {
	if node.leaf {
		let index = node.tabs.iter().position(|tab| tab.key == item)?;
		return Some(node.tabs.remove(index));
	}
	node
		.children
		.iter_mut()
		.find_map(|child| take_tab(child, item))
}

/// VS Code closeEmptyGroups: drop empty leaves, collapse single-child
/// branches.
fn prune_walk(mut node: Group) -> Option<Group> {
	if node.leaf {
		return (!node.tabs.is_empty()).then_some(node);
	}
	node.children = node.children.into_iter().filter_map(prune_walk).collect();
	if node.children.len() == 1 {
		return node.children.pop();
	}
	(!node.children.is_empty()).then_some(node)
}

// ----------------------------------------------------------- scene index ---

#[derive(Clone, Copy, Default)]
struct Rect {
	x: f64,
	y: f64,
	w: f64,
	h: f64,
}

/// Canonical scene keys and rects the signal handlers need, rebuilt after
/// every model push (the web host's `reindex`).
#[derive(Default)]
struct SceneIndex {
	/// Leaf → editor body rect (`…/#ed`).
	ed:       BTreeMap<String, Rect>,
	/// Leaf → drop-preview rect key (`…/#zone`).
	zone:     BTreeMap<String, String>,
	/// Leaf → pane rect and key (`…#kids~leaf/stack@0`).
	pane:     BTreeMap<String, (String, Rect)>,
	/// Leaf → editor scroll viewport key (`…/#edscroll`).
	edscroll: BTreeMap<String, String>,
	/// Leaf → editable code field key.
	field:    BTreeMap<String, String>,
	/// Tab item → tab body rect (`…/#body`).
	body:     BTreeMap<String, Rect>,
	/// Tab item → insertion indicator rect keys.
	indl:     BTreeMap<String, String>,
	indr:     BTreeMap<String, String>,
}

/// Unescapes one canonical key segment (`%2F`/`%7E`/`%25`).
fn unescape_segment(escaped: &str) -> String {
	escaped
		.replace("%2F", "/")
		.replace("%7E", "~")
		.replace("%25", "%")
}

/// Decodes the innermost `~item/` segment of a canonical key prefix.
fn last_item_seg(key: &str) -> Option<String> {
	let mut item: Option<&str> = None;
	let mut rest = key;
	while let Some(at) = rest.find('~') {
		let tail = &rest[at + 1..];
		let Some(end) = tail.find('/') else { break };
		item = Some(&tail[..end]);
		rest = &tail[end + 1..];
	}
	item.map(unescape_segment)
}

/// Leaf key owning an `…/#ed…` scene key.
fn leaf_key_from_editor_key(key: &str) -> Option<String> {
	let at = key.find("/#ed")?;
	last_item_seg(&key[..=at])
}

// ------------------------------------------------------------- highlight ---

/// One lexed token: codepoint length plus enough class to color it.
enum Token {
	Comment(usize),
	Str(usize),
	Ident(usize),
	Number(usize),
	Other(char),
}

impl Token {
	const fn len(&self) -> usize {
		match *self {
			Self::Comment(len) | Self::Str(len) | Self::Ident(len) | Self::Number(len) => len,
			Self::Other(_) => 1,
		}
	}
}

/// Tokenizes one line like fsmodel.js's `TOKENS` regex:
/// `//.*$ | "[^"]*" | <[^>\s]+> | ident | digits | any`.
fn lex_line(line: &str) -> Vec<Token> {
	let chars: Vec<char> = line.chars().collect();
	let mut out = Vec::new();
	let mut at = 0;
	while at < chars.len() {
		let ch = chars[at];
		if ch == '/' && chars.get(at + 1) == Some(&'/') {
			out.push(Token::Comment(chars.len() - at));
			break;
		}
		if ch == '"' {
			let mut end = at + 1;
			while end < chars.len() && chars[end] != '"' {
				end += 1;
			}
			if end < chars.len() {
				out.push(Token::Str(end + 1 - at));
				at = end + 1;
				continue;
			}
		}
		if ch == '<' {
			let mut end = at + 1;
			while end < chars.len() && chars[end] != '>' && !chars[end].is_whitespace() {
				end += 1;
			}
			if end > at + 1 && chars.get(end) == Some(&'>') {
				out.push(Token::Str(end + 1 - at));
				at = end + 1;
				continue;
			}
		}
		if ch.is_ascii_alphabetic() || ch == '_' {
			let mut end = at + 1;
			while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
				end += 1;
			}
			out.push(Token::Ident(end - at));
			at = end;
			continue;
		}
		if ch.is_ascii_digit() {
			let mut end = at + 1;
			while end < chars.len() && chars[end].is_ascii_digit() {
				end += 1;
			}
			out.push(Token::Number(end - at));
			at = end;
			continue;
		}
		out.push(Token::Other(ch));
		at += 1;
	}
	out
}

const KEYWORDS: &[&str] = &[
	"constexpr",
	"explicit",
	"operator",
	"const",
	"return",
	"if",
	"void",
	"struct",
	"delete",
	"default",
	"noexcept",
	"namespace",
	"using",
	"bool",
	"nullptr",
	"this",
	"class",
	"public",
];
const TYPES: &[&str] =
	&["uint64_t", "MDB_env", "MDB_txn", "MDB_dbi", "node_kind", "dir_entry", "size_t"];

/// Paint-only syntax ranges, a verbatim port of `fsmodel.js highlight()`.
fn highlight(text: &str) -> Vec<FieldStyle> {
	let kw = tint("#9D74BE");
	let string_tone = tint("#DCA9A8");
	let comment_tone = tint("#46454A");
	let type_tone = tint("#DCB99E");
	let fn_tone = tint("#5E9AA0");
	let mut ranges: Vec<FieldStyle> = Vec::new();
	let mut add = |start: i32, end: i32, rgba: gv::Rgba| {
		if let Some(previous) = ranges.last_mut()
			&& previous.end == start
			&& previous.rgba == rgba
		{
			previous.end = end;
			return;
		}
		ranges.push(FieldStyle { start, end, rgba, flags: 0 });
	};
	let ilen = |len: usize| i32::try_from(len).unwrap_or(0);

	let mut line_start: i32 = 0;
	let lines: Vec<&str> = text.split('\n').collect();
	for (line_index, line) in lines.iter().enumerate() {
		let line_len = ilen(line.chars().count());
		let trimmed = line.trim_start();
		if trimmed.starts_with('#') {
			// `^(\s*)(#\w+)(.*)$`: directive keyword, then only strings and
			// `<…>` includes take color.
			let indent = line.chars().count() - trimmed.chars().count();
			let word_len = trimmed
				.chars()
				.skip(1)
				.take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
				.count();
			let directive_len = ilen(indent + 1 + word_len);
			add(line_start, line_start + directive_len, kw);
			let rest: String = line.chars().skip(indent + 1 + word_len).collect();
			let mut offset = line_start + directive_len;
			for token in lex_line(&rest) {
				if matches!(token, Token::Str(_)) {
					add(offset, offset + ilen(token.len()), string_tone);
				}
				offset += ilen(token.len());
			}
		} else {
			let tokens = lex_line(line);
			let chars: Vec<char> = line.chars().collect();
			let mut offset = line_start;
			let mut char_at = 0usize;
			for (index, token) in tokens.iter().enumerate() {
				let len = ilen(token.len());
				match *token {
					Token::Comment(_) => add(offset, offset + len, comment_tone),
					Token::Str(_) => add(offset, offset + len, string_tone),
					Token::Number(_) => add(offset, offset + len, type_tone),
					Token::Ident(word_len) => {
						let word: String = chars[char_at..char_at + word_len].iter().collect();
						if KEYWORDS.contains(&word.as_str()) {
							add(offset, offset + len, kw);
						} else if TYPES.contains(&word.as_str()) {
							add(offset, offset + len, type_tone);
						} else {
							let mut next = index + 1;
							while next < tokens.len() && matches!(tokens[next], Token::Other(' ')) {
								next += 1;
							}
							if matches!(tokens.get(next), Some(Token::Other('('))) {
								add(offset, offset + len, fn_tone);
							}
						}
					},
					Token::Other(_) => {},
				}
				offset += len;
				char_at += token.len();
			}
		}
		line_start += line_len;
		if line_index + 1 < lines.len() {
			line_start += 1;
		}
	}
	ranges
}

// ------------------------------------------------------------------ host ---

/// Application state mirroring `web/main.js` module scope.
pub struct VscodeHost {
	doc:               gv::Doc,
	start:             Instant,
	root:              Group,
	focused_leaf:      String,
	uid:               u32,
	untitled:          u32,
	contents:          BTreeMap<String, String>,
	open_dirs:         BTreeSet<String>,
	last_seeded:       BTreeMap<String, String>,
	selected_path:     String,
	indicator:         Option<(String, bool)>,
	zone:              Option<(String, &'static str)>,
	index:             SceneIndex,
	/// Scene-dependent work staged for the next post-solve drain.
	needs_drain:       bool,
	pending_splits:    Vec<(String, f64)>,
	pending_scrolls:   Vec<(String, f64)>,
	pending_reveal:    Option<(String, String, u32)>,
	scm_rows:          Vec<ScmChangesItem>,
	chat_sessions:     Vec<ChatSessionsItem>,
	term_log:          String,
	selected_tree_row: Option<String>,
	selected_activity: Option<String>,
	selected_panel:    Option<String>,
	last_tree_pick:    (String, Instant),
	menu_target:       Option<(String, String)>,
	nav_history:       Vec<String>,
	nav_index:         usize,
	navigating:        bool,
	pane_sidebar:      f64,
	pane_chat:         f64,
	pane_panel:        f64,
	panel_restore:     f64,
	panel_pre_max:     f64,
	panel_maximized:   bool,
	last_caret_status: String,
}

impl VscodeHost {
	pub fn new() -> Self {
		let selected_path = "include/agentfs/store.hpp".to_owned();
		Self {
			doc:               gv::Doc::new(),
			start:             Instant::now(),
			root:              Group::branch("g0", true, vec![
				Group::leaf_node("gA", vec![Tab::new("mdb.hpp", RED, "2", true)]),
				Group::leaf_node("gB", vec![
					Tab::new("overlay.hpp", GRAY, "", false),
					Tab::new("store.hpp", RED, "2", true),
				]),
			]),
			focused_leaf:      "gB".to_owned(),
			uid:               0,
			untitled:          0,
			contents:          vscode_fs::CONTENTS
				.iter()
				.map(|&(path, content)| (path.to_owned(), content.to_owned()))
				.collect(),
			open_dirs:         vscode_fs::DEFAULT_OPEN
				.iter()
				.map(|&path| path.to_owned())
				.collect(),
			last_seeded:       BTreeMap::new(),
			selected_path:     selected_path.clone(),
			indicator:         None,
			zone:              None,
			index:             SceneIndex::default(),
			needs_drain:       false,
			pending_splits:    Vec::new(),
			pending_scrolls:   Vec::new(),
			pending_reveal:    None,
			scm_rows:          Vec::new(),
			chat_sessions:     Vec::new(),
			term_log:          "can@mac slab-lang %".to_owned(),
			selected_tree_row: None,
			selected_activity: None,
			selected_panel:    None,
			last_tree_pick:    (String::new(), Instant::now()),
			menu_target:       None,
			nav_history:       vec![selected_path],
			nav_index:         0,
			navigating:        false,
			pane_sidebar:      195.0,
			pane_chat:         198.0,
			pane_panel:        178.0,
			panel_restore:     178.0,
			panel_pre_max:     178.0,
			panel_maximized:   false,
			last_caret_status: String::new(),
		}
	}

	/// Whether the embedded document decoded.
	pub const fn ok(&self) -> bool {
		self.doc.ok()
	}

	// ------------------------------------------------------ model helpers --

	fn path_of_tab_key(key: &str) -> String {
		match key {
			"mdb.hpp" => MDB_PATH.to_owned(),
			"store.hpp" => "include/agentfs/store.hpp".to_owned(),
			other => other.to_owned(),
		}
	}

	fn active_path_of(&self, leaf: &str) -> String {
		find_leaf(&self.root, leaf)
			.and_then(|group| group.tabs.iter().find(|tab| tab.active))
			.map(|tab| Self::path_of_tab_key(&tab.key))
			.unwrap_or_default()
	}

	fn content_of(&self, path: &str) -> &str {
		self.contents.get(path).map_or("", String::as_str)
	}

	fn activate(&mut self, leaf_key: &str, item: &str) {
		if let Some(group) = find_leaf_mut(&mut self.root, leaf_key) {
			for tab in &mut group.tabs {
				tab.active = tab.key == item;
			}
			leaf_key.clone_into(&mut self.focused_leaf);
		}
	}

	fn prune_empty(&mut self) {
		let placeholder = Group::branch("g0", true, Vec::new());
		let taken = std::mem::replace(&mut self.root, placeholder);
		self.root = match prune_walk(taken) {
			None => {
				self.uid += 1;
				Group::branch("g0", true, vec![Group::leaf_node(
					&format!("gN{}", self.uid),
					Vec::new(),
				)])
			},
			Some(next) if next.leaf => Group::branch("g0", true, vec![next]),
			Some(next) => next,
		};
	}

	fn close_tab(&mut self, item: &str) {
		let Some(leaf_key) = leaf_of(&self.root, item) else {
			return;
		};
		let mut next_active: Option<String> = None;
		if let Some(group) = find_leaf_mut(&mut self.root, &leaf_key)
			&& let Some(index) = group.tabs.iter().position(|tab| tab.key == item)
		{
			let was_active = group.tabs[index].active;
			group.tabs.remove(index);
			if was_active && !group.tabs.is_empty() {
				next_active = Some(group.tabs[index.min(group.tabs.len() - 1)].key.clone());
			}
		}
		if let Some(next) = next_active {
			self.activate(&leaf_key, &next);
		}
		self.prune_empty();
	}

	fn move_tab_to_leaf(&mut self, item: &str, leaf_key: &str, slot: usize) {
		let from = leaf_of(&self.root, item);
		let Some(tab) = take_tab(&mut self.root, item) else {
			return;
		};
		let Some(target) = find_leaf_mut(&mut self.root, leaf_key) else {
			return;
		};
		let mut slot = slot;
		if from.as_deref() == Some(leaf_key) && slot > target.tabs.len() {
			slot = target.tabs.len();
		}
		target.tabs.insert(slot.min(target.tabs.len()), tab);
		self.activate(leaf_key, item);
		self.prune_empty();
	}

	/// grid.ts addGroup: LEFT/UP insert before, RIGHT/DOWN after;
	/// same-orientation parents take a sibling, different orientations wrap.
	fn split_leaf(&mut self, target_key: &str, dir: &str, item: &str) -> Option<String> {
		let tab = take_tab(&mut self.root, item)?;
		let horizontal = dir == "left" || dir == "right";
		let before = dir == "left" || dir == "up";
		self.uid += 1;
		let fresh_key = format!("gN{}", self.uid);
		let fresh = Group::leaf_node(&fresh_key, vec![tab]);

		fn insert(
			node: &mut Group,
			target_key: &str,
			horizontal: bool,
			before: bool,
			fresh: &mut Option<Group>,
			wrap_uid: &mut u32,
		) -> bool {
			let hit = node
				.children
				.iter()
				.position(|child| child.leaf && child.key == target_key);
			if let Some(index) = hit {
				let fresh = fresh.take().expect("fresh group consumed once");
				if node.horizontal == horizontal {
					node
						.children
						.insert(if before { index } else { index + 1 }, fresh);
				} else {
					*wrap_uid += 1;
					let target = node.children.remove(index);
					let children = if before {
						vec![fresh, target]
					} else {
						vec![target, fresh]
					};
					let wrap = Group::branch(&format!("gW{wrap_uid}"), horizontal, children);
					node.children.insert(index, wrap);
				}
				return true;
			}
			node.children.iter_mut().any(|child| {
				!child.leaf && insert(child, target_key, horizontal, before, fresh, wrap_uid)
			})
		}

		let mut carrier = Some(fresh);
		let mut wrap_uid = self.uid;
		if !insert(&mut self.root, target_key, horizontal, before, &mut carrier, &mut wrap_uid) {
			return None;
		}
		self.uid = wrap_uid;
		self.activate(&fresh_key, item);
		self.prune_empty();
		Some(fresh_key)
	}

	// ------------------------------------------------------------ pushes --

	fn push_tree(&mut self) {
		let mut rows = Vec::new();
		let mut skip_deeper_than: Option<u32> = None;
		for entry in vscode_fs::ENTRIES {
			if let Some(depth) = skip_deeper_than {
				if entry.depth > depth {
					continue;
				}
				skip_deeper_than = None;
			}
			let open = self.open_dirs.contains(entry.path);
			if entry.dir && !open {
				skip_deeper_than = Some(entry.depth);
			}
			rows.push(
				TreeItem {
					key: None,
					name: entry.name.to_owned(),
					letter: entry.letter.to_owned(),
					icon: if entry.dir {
						if open { "folder-open" } else { "folder" }.to_owned()
					} else if entry.icon.is_empty() {
						"blank".to_owned()
					} else {
						entry.icon.to_owned()
					},
					tint: tint(entry.tint),
					badge: entry.badge.to_owned(),
					indent: f64::from(14 + entry.depth * 10),
					dir: entry.dir,
					open,
				}
				.with_key(entry.path),
			);
		}
		let _ = self.doc.set_tree(&rows);
	}

	fn push_scm_changes(&mut self) {
		let mut dirty: Vec<String> = Vec::new();
		for_each_leaf_mut(&mut self.root, &mut |group| {
			for tab in &group.tabs {
				if tab.dirty {
					let path = Self::path_of_tab_key(&tab.key);
					if !dirty.contains(&path) {
						dirty.push(path);
					}
				}
			}
		});
		let rows: Vec<ScmChangesItem> = dirty
			.iter()
			.map(|path| {
				ScmChangesItem {
					key:   None,
					file:  path.rsplit('/').next().unwrap_or(path).to_owned(),
					badge: "M".to_owned(),
				}
				.with_key(path)
			})
			.collect();
		if rows == self.scm_rows {
			return;
		}
		self.scm_rows = rows;
		let _ = self.doc.set_scm_changes(&self.scm_rows);
	}

	fn strip(&self, node: &Group) -> RootItem {
		let path = if node.leaf {
			node
				.tabs
				.iter()
				.find(|tab| tab.active)
				.map(|tab| Self::path_of_tab_key(&tab.key))
				.unwrap_or_default()
		} else {
			String::new()
		};
		let segments: Vec<&str> = if path.is_empty() {
			Vec::new()
		} else {
			path.split('/').collect()
		};
		let crumbs = segments
			.iter()
			.enumerate()
			.map(|(index, seg)| {
				let last = index == segments.len() - 1;
				RootCrumbsItem {
					key: None,
					seg: (*seg).to_owned(),
					// Verbatim port: the web host matches extensions
					// case-sensitively too.
					#[allow(
						clippy::case_sensitive_file_extension_comparisons,
						reason = "verbatim port of the web host's matching"
					)]
					letter: if last {
						if seg.ends_with(".hpp") {
							"h".to_owned()
						} else if seg.ends_with(".cpp") || seg.ends_with(".c") {
							"C".to_owned()
						} else {
							String::new()
						}
					} else {
						String::new()
					},
					last,
				}
				.with_key(index.to_string())
			})
			.collect();
		let gutter = if path.is_empty() {
			String::new()
		} else {
			let lines = self.content_of(&path).split('\n').count();
			let mut gutter = String::new();
			for line in 1..=lines {
				if line > 1 {
					gutter.push('\n');
				}
				gutter.push_str(&line.to_string());
			}
			gutter
		};
		RootItem {
			key: None,
			leaf: node.leaf,
			horizontal: node.horizontal,
			show_mdb: node.leaf && path.ends_with(MDB_PATH),
			show_store: node.leaf && path.ends_with("include/agentfs/store.hpp"),
			show_edit: node.leaf && !path.is_empty(),
			show_find: node.leaf && path == MDB_PATH,
			find_status: node.find_status.clone(),
			curline: node.curline,
			curline_on: node.curline_on,
			gutter,
			crumbs,
			tabs: node
				.tabs
				.iter()
				.map(|tab| {
					RootTabsItem {
						key:     None,
						name:    tab.name.clone(),
						note:    tab.note.clone(),
						tint:    tab.tint,
						badge:   tab.badge.clone(),
						active:  tab.active,
						hot:     tab.hot,
						preview: tab.preview,
						dirty:   tab.dirty,
					}
					.with_key(&tab.key)
				})
				.collect(),
			children: node
				.children
				.iter()
				.map(|child| self.strip(child))
				.collect(),
		}
		.with_key(&node.key)
	}

	/// The web host's `pushModel`: derive per-leaf display state and publish
	/// the grid; scene-dependent follow-ups run in the next drain.
	fn push_model(&mut self) {
		let focused = self.focused_leaf.clone();
		for_each_leaf_mut(&mut self.root, &mut |group| {
			if !group.tabs.iter().any(|tab| tab.active) && !group.tabs.is_empty() {
				group.tabs[0].active = true;
			}
			let key = group.key.clone();
			for tab in &mut group.tabs {
				tab.hot = tab.active && key == focused;
			}
		});
		let lang = {
			#[allow(
				clippy::case_sensitive_file_extension_comparisons,
				reason = "verbatim port of the web host's matching"
			)]
			let of_name = |name: &str| {
				if name == "CMakeLists.txt" {
					"CMake"
				} else if name.ends_with(".hpp") || name.ends_with(".cpp") {
					"C++"
				} else if name.ends_with(".md") {
					"Markdown"
				} else if name.ends_with(".sh") {
					"Shell Script"
				} else {
					"Plain Text"
				}
			};
			let path = self.active_path_of(&focused);
			of_name(path.rsplit('/').next().unwrap_or(""))
		};
		let _ = self.doc.set_status_lang(lang);
		let root_item = self.strip(&self.root);
		let _ = self.doc.set_root(&[root_item]);
		self.push_scm_changes();
		self.needs_drain = true;
	}

	/// Rebuilds the scene index from the last solved scene.
	fn reindex(&mut self) {
		let mut index = SceneIndex::default();
		let inst = &self.doc.inst;
		for entry in &inst.sc.entries {
			let key = scene::key_of(inst.doc(), &inst.st.lists, entry.node);
			let rect = Rect { x: entry.x, y: entry.y, w: entry.w, h: entry.h };
			let Some(item) = last_item_seg(&format!("{key}/")) else {
				continue;
			};
			if key.ends_with("/#ed") {
				index.ed.insert(item, rect);
			} else if key.ends_with("/#zone") {
				index.zone.insert(item, key);
			} else if key.ends_with("/#edscroll") {
				index.edscroll.insert(item, key);
			} else if key.contains("/#edscroll/") && key.ends_with("/text@1") {
				index.field.insert(item, key);
			} else if key.ends_with("/#body") {
				index.body.insert(item, rect);
			} else if key.ends_with("/#indl") {
				index.indl.insert(item, key);
			} else if key.ends_with("/#indr") {
				index.indr.insert(item, key);
			} else if let Some(prefix) = key.strip_suffix("/stack@0")
				&& prefix.ends_with(&format!("#kids~{item}"))
			{
				index.pane.insert(item, (key, rect));
			}
		}
		self.index = index;
	}

	/// One post-solve pass: refresh the index, seed newly revealed editors,
	/// then run writes staged for a measured scene (scrolls, splits, reveal).
	fn drain(&mut self) {
		self.reindex();

		// Scrolls staged by the PREVIOUS pass — their content has now been
		// measured, so offsets no longer clamp against stale extents.
		for (key, off) in std::mem::take(&mut self.pending_scrolls) {
			let _ = self.doc.set_scroll(&key, 0, off);
		}

		// Seed newly activated editors with content and syntax styles.
		let mut leaves: Vec<String> = Vec::new();
		for_each_leaf_mut(&mut self.root, &mut |group| leaves.push(group.key.clone()));
		let mut seeded_now: BTreeSet<String> = BTreeSet::new();
		for leaf_key in &leaves {
			let path = self.active_path_of(leaf_key);
			if path.is_empty() || self.last_seeded.get(leaf_key.as_str()) == Some(&path) {
				continue;
			}
			let Some(field_key) = self.index.field.get(leaf_key.as_str()).cloned() else {
				continue;
			};
			let content = self.content_of(&path).to_owned();
			let _ = self.doc.set_field_text(&field_key, &content);
			let _ =
				kframe::inst_set_field_styles(&mut self.doc.inst, &field_key, &highlight(&content));
			self.last_seeded.insert(leaf_key.clone(), path.clone());
			seeded_now.insert(leaf_key.clone());
			if let Some(scroll_key) = self.index.edscroll.get(leaf_key.as_str()).cloned() {
				let off = if path == MDB_PATH { 38.0 * LINE_H } else { 0.0 };
				self.pending_scrolls.push((scroll_key, off));
			}
		}

		let splits = std::mem::take(&mut self.pending_splits);
		for (leaf_key, half) in splits {
			if let Some((pane_key, _)) = self.index.pane.get(&leaf_key) {
				let key = pane_key.clone();
				let _ = self.doc.set_split(&key, half);
			}
		}

		// Reveal after the target leaf's seed has measured.
		if let Some((leaf_key, path, line)) = self.pending_reveal.clone()
			&& !seeded_now.contains(&leaf_key)
		{
			self.pending_reveal = None;
			self.apply_reveal(&leaf_key, &path, line);
		}

		self.needs_drain = !self.pending_scrolls.is_empty() || self.pending_reveal.is_some();
	}

	fn apply_reveal(&mut self, leaf_key: &str, path: &str, line: u32) {
		let target = line.max(1);
		if self.active_path_of(leaf_key) != path {
			return;
		}
		let text = self.content_of(path).to_owned();
		let mut offset = 0i32;
		let mut current = 1u32;
		for ch in text.chars() {
			if current >= target {
				break;
			}
			offset += 1;
			if ch == '\n' {
				current += 1;
			}
		}
		if let Some(field_key) = self.index.field.get(leaf_key).cloned() {
			let _ = kframe::inst_set_caret(&mut self.doc.inst, &field_key, offset, offset);
		}
		if let Some(scroll_key) = self.index.edscroll.get(leaf_key).cloned() {
			let _ = self
				.doc
				.set_scroll(&scroll_key, 0, f64::from(target - 1) * LINE_H);
		}
		let _ = self.doc.set_status_caret(&format!("Ln {target}, Col 1"));
	}

	// -------------------------------------------------- indicator + zone --

	fn set_indicator(&mut self, next: Option<(String, bool)>) {
		if self.indicator == next {
			return;
		}
		if let Some((item, before)) = self.indicator.take() {
			let map = if before {
				&self.index.indl
			} else {
				&self.index.indr
			};
			if let Some(key) = map.get(&item).cloned() {
				let state = if before {
					"insert-before"
				} else {
					"insert-after"
				};
				let _ = kframe::inst_set_node_state(&mut self.doc.inst, &key, state, false);
			}
		}
		if let Some((item, before)) = next {
			let map = if before {
				&self.index.indl
			} else {
				&self.index.indr
			};
			if let Some(key) = map.get(&item).cloned() {
				let state = if before {
					"insert-before"
				} else {
					"insert-after"
				};
				let _ = kframe::inst_set_node_state(&mut self.doc.inst, &key, state, true);
				self.indicator = Some((item, before));
			}
		}
	}

	fn set_zone(&mut self, next: Option<(String, &'static str)>) {
		if self.zone == next {
			return;
		}
		if let Some((leaf, dir)) = self.zone.take()
			&& let Some(key) = self.index.zone.get(&leaf).cloned()
		{
			let _ =
				kframe::inst_set_node_state(&mut self.doc.inst, &key, &format!("zone-{dir}"), false);
		}
		if let Some((leaf, dir)) = next
			&& let Some(key) = self.index.zone.get(&leaf).cloned()
		{
			let _ =
				kframe::inst_set_node_state(&mut self.doc.inst, &key, &format!("zone-{dir}"), true);
			self.zone = Some((leaf, dir));
		}
	}

	/// editorDropTarget.ts positionOverlay, verbatim thresholds.
	fn zone_for(rect: Rect, x: f64, y: f64) -> &'static str {
		let rel_x = x - rect.x;
		let rel_y = y - rect.y;
		let edge_w = rect.w * 0.1;
		let edge_h = rect.h * 0.1;
		if rel_x > edge_w && rel_x < rect.w - edge_w && rel_y > edge_h && rel_y < rect.h - edge_h {
			return "merge";
		}
		if rel_x < rect.w / 3.0 {
			return "left";
		}
		if rel_x > rect.w / 3.0 * 2.0 {
			return "right";
		}
		if rel_y < rect.h / 2.0 { "up" } else { "down" }
	}

	// -------------------------------------------------------------- files --

	fn record_nav(&mut self, path: &str) {
		if self.navigating || path.is_empty() {
			return;
		}
		if self.nav_history.get(self.nav_index).map(String::as_str) == Some(path) {
			return;
		}
		self.nav_history.truncate(self.nav_index + 1);
		self.nav_history.push(path.to_owned());
		self.nav_index = self.nav_history.len() - 1;
		self.sync_nav();
	}

	fn sync_nav(&mut self) {
		let _ = self.doc.set_nav_canback(self.nav_index > 0);
		let _ = self
			.doc
			.set_nav_canfwd(self.nav_index + 1 < self.nav_history.len());
	}

	fn open_file(&mut self, path: &str, preview: bool, line: Option<u32>) {
		let key = if path.ends_with(MDB_PATH) {
			"mdb.hpp".to_owned()
		} else if path.ends_with("include/agentfs/store.hpp") {
			"store.hpp".to_owned()
		} else {
			path.to_owned()
		};
		if let Some(leaf_key) = leaf_of(&self.root, &key) {
			if !preview
				&& let Some(group) = find_leaf_mut(&mut self.root, &leaf_key)
				&& let Some(tab) = group.tabs.iter_mut().find(|tab| tab.key == key)
			{
				tab.preview = false;
			}
			path.clone_into(&mut self.selected_path);
			self.activate(&leaf_key, &key);
			if let Some(line) = line {
				self.pending_reveal = Some((leaf_key.clone(), path.to_owned(), line));
			}
			self.push_model();
			return;
		}

		let leaf_key = if find_leaf(&self.root, &self.focused_leaf).is_some() {
			self.focused_leaf.clone()
		} else {
			let mut first = None;
			for_each_leaf_mut(&mut self.root, &mut |group| {
				if first.is_none() {
					first = Some(group.key.clone());
				}
			});
			let Some(first) = first else { return };
			first
		};
		let name = path.rsplit('/').next().unwrap_or(path).to_owned();
		let parts: Vec<&str> = path.split('/').collect();
		let note = if parts.len() > 1 {
			parts[parts.len() - 2].to_owned()
		} else {
			String::new()
		};
		if let Some(group) = find_leaf_mut(&mut self.root, &leaf_key) {
			let slot = if preview {
				group.tabs.iter().position(|tab| tab.preview)
			} else {
				None
			};
			if let Some(slot) = slot {
				let existing = &mut group.tabs[slot];
				existing.key.clone_from(&key);
				existing.name = name;
				existing.note = note;
				existing.tint = tint(GRAY);
				existing.badge = String::new();
				existing.preview = true;
				existing.dirty = false;
			} else {
				group.tabs.push(Tab {
					key: key.clone(),
					name,
					note,
					tint: tint(GRAY),
					badge: String::new(),
					active: true,
					hot: false,
					preview,
					dirty: false,
				});
			}
		}
		path.clone_into(&mut self.selected_path);
		self.activate(&leaf_key, &key);
		if let Some(line) = line {
			self.pending_reveal = Some((leaf_key.clone(), path.to_owned(), line));
		}
		self.push_model();
	}

	// -------------------------------------------------------- caret status --

	/// Mirrors the web host's `updateCaretStatus`: status text plus the
	/// focused group's current-line highlight.
	fn update_caret_status(&mut self) {
		let focused = kframe::inst_focus(&self.doc.inst);
		if focused == slir::NONE {
			return;
		}
		let key = scene::key_of(self.doc.inst.doc(), &self.doc.inst.st.lists, focused);
		if !key.contains("/#edscroll/") {
			return;
		}
		let edit_index = dispatch::ed_ix(&self.doc.inst.ds, focused);
		if edit_index < 0 {
			return;
		}
		let state = &self.doc.inst.ds.ed[usize::try_from(edit_index).expect("edit index")];
		let caret = usize::try_from(state.caret.max(0)).unwrap_or(0);
		let cps = state.text.cps();
		let before = &cps[..caret.min(cps.len())];
		let line = before.iter().filter(|&&cp| cp == 10).count() + 1;
		let last_newline = before.iter().rposition(|&cp| cp == 10);
		let col = before.len() - last_newline.map_or(0, |at| at + 1) + 1;
		let status = format!("Ln {line}, Col {col}");
		if status != self.last_caret_status {
			let _ = self.doc.set_status_caret(&status);
			self.last_caret_status = status;
		}

		let Some(leaf_key) = leaf_key_from_editor_key(&key) else {
			return;
		};
		#[allow(clippy::cast_precision_loss, reason = "editor line counts stay far below 2^52")]
		let curline = (line as f64 - 1.0) * LINE_H;
		let mut changed = false;
		for_each_leaf_mut(&mut self.root, &mut |group| {
			let on = group.key == leaf_key;
			if on && group.curline != curline {
				group.curline = curline;
				changed = true;
			}
			if group.curline_on != on {
				group.curline_on = on;
				changed = true;
			}
		});
		if changed {
			self.push_model();
		}
	}

	// ------------------------------------------------------------- layout --

	fn apply_layout(&mut self) {
		let width = self.doc.inst.st.env.vw;
		let height = self.doc.inst.st.env.vh;
		if self.panel_maximized {
			self.pane_panel = (height - CHROME_H - 200.0).max(PANEL_FLOOR);
		}
		let _ = self.doc.set_divider(gv::keys::SASHL, self.pane_sidebar);
		let _ = self
			.doc
			.set_divider(gv::keys::SASHR, width - CHROME_W - self.pane_sidebar - self.pane_chat);
		let _ = self
			.doc
			.set_divider(gv::keys::SASHP, height - CHROME_H - self.pane_panel);
	}

	fn close_panel(&mut self) {
		if self.pane_panel > PANEL_FLOOR {
			self.panel_restore = self.pane_panel;
		}
		self.pane_panel = 0.0;
		self.panel_maximized = false;
		self.apply_layout();
	}

	fn toggle_max_panel(&mut self) {
		if self.panel_maximized {
			self.pane_panel = self.panel_pre_max;
			if self.panel_pre_max > PANEL_FLOOR {
				self.panel_restore = self.panel_pre_max;
			}
			self.panel_maximized = false;
		} else {
			self.panel_pre_max = self.pane_panel;
			self.panel_maximized = true;
		}
		self.apply_layout();
	}

	fn close_menu(&mut self) {
		if self.menu_target.take().is_some() {
			let _ = self.doc.set_menu_open(false);
		}
	}

	/// Populates the initial UI. Runs before the window opens, while the host
	/// still owns the instance, so explicit settle frames are safe here.
	pub fn initialize(&mut self, width: f64, height: f64, dark: bool) {
		self.doc.set_env(width, height, dark, false);
		self.push_tree();
		self.sync_nav();
		let log = self.term_log.clone();
		let _ = self.doc.set_panel_termlog(&log);
		self.push_model();
		self.apply_layout();
		let mut settles = 0;
		while self.needs_drain && settles < 4 {
			let t = self.start.elapsed().as_secs_f64() * 1000.0;
			let _ = self.doc.frame(t);
			self.drain();
			settles += 1;
		}
		// Initial chrome selection: Explorer store.hpp row + OUTPUT panel tab.
		let tree_row = gv::keys::item_key(gv::keys::TREEROWS, &self.selected_path, "row@0");
		if kframe::inst_set_node_state(&mut self.doc.inst, &tree_row, "selected", true) {
			self.selected_tree_row = Some(tree_row);
		}
		let output_tab = format!("{}/row@0/output", gv::keys::PANEL);
		if kframe::inst_set_node_state(&mut self.doc.inst, &output_tab, "selected", true) {
			self.selected_panel = Some(output_tab);
		}
	}

	// ------------------------------------------------------------ signals --

	fn on_signal(&mut self, signal: Signal) {
		if !matches!(signal, Signal::TabMenu { .. } | Signal::MenuPick { .. }) {
			// No raw pointer hook exists host-side; any other interaction
			// dismisses an open context menu, like VS Code's outside click.
			self.close_menu();
		}
		match signal {
			Signal::TabPress { item, meta } => {
				if meta.hit_key.contains("/#body/icon") || item.is_empty() {
					return;
				}
				if let Some(leaf_key) = leaf_of(&self.root, &item) {
					self.activate(&leaf_key, &item);
				}
				self.push_model();
			},
			Signal::TabUp { item, meta } => {
				if meta.button == 1 && !item.is_empty() && meta.hit_key.contains('~') {
					self.close_tab(&item);
					self.push_model();
				}
			},
			Signal::TabClose { item, .. } => {
				self.close_tab(&item);
				self.push_model();
			},
			Signal::TabDbl { item, .. } => {
				if let Some(leaf_key) = leaf_of(&self.root, &item)
					&& let Some(group) = find_leaf_mut(&mut self.root, &leaf_key)
					&& let Some(tab) = group.tabs.iter_mut().find(|tab| tab.key == item)
					&& tab.preview
				{
					tab.preview = false;
					self.push_model();
				}
			},
			Signal::TabMove { item, meta } => self.on_tab_move(&item, &meta),
			Signal::TabDrop { item, meta } => {
				let src = meta.src_item;
				if src.is_empty() || src == item {
					return;
				}
				let Some(leaf_key) = leaf_of(&self.root, &item) else {
					return;
				};
				let before = self
					.index
					.body
					.get(&item)
					.is_some_and(|rect| meta.x < rect.x + rect.w / 2.0);
				let Some(group) = find_leaf(&self.root, &leaf_key) else {
					return;
				};
				let Some(index) = group.tabs.iter().position(|tab| tab.key == item) else {
					return;
				};
				self.move_tab_to_leaf(&src, &leaf_key, if before { index } else { index + 1 });
				self.push_model();
			},
			Signal::StripDrop { meta, .. } => {
				if meta.src_item.is_empty() {
					return;
				}
				let Some(strip_at) = meta.key.find("/#strip") else {
					return;
				};
				if let Some(leaf_key) = last_item_seg(&meta.key[..=strip_at]) {
					self.move_tab_to_leaf(&meta.src_item, &leaf_key, usize::MAX);
				}
				self.push_model();
			},
			Signal::EditorDrop { meta, .. } => self.on_editor_drop(&meta),
			Signal::TabEnd { .. } => {
				self.set_indicator(None);
				self.set_zone(None);
			},
			Signal::StripDbl { meta, .. } => {
				let Some(strip_at) = meta.key.find("/#strip") else {
					return;
				};
				let Some(leaf_key) = last_item_seg(&meta.key[..=strip_at]) else {
					return;
				};
				self.untitled += 1;
				let key = format!("untitled:{}", self.untitled);
				self.contents.insert(key.clone(), String::new());
				if let Some(group) = find_leaf_mut(&mut self.root, &leaf_key) {
					let mut tab = Tab::new(&format!("Untitled-{}", self.untitled), GRAY, "", false);
					tab.key.clone_from(&key);
					tab.note = String::new();
					group.tabs.push(tab);
				}
				self.activate(&leaf_key, &key);
				self.push_model();
			},
			Signal::TabMenu { item, meta } => self.on_tab_menu(&item, &meta),
			Signal::MenuPick { item, .. } => self.on_menu_pick(&item),
			Signal::TreePick { item, meta } => self.on_tree_pick(&item, &meta),
			Signal::SidebarSearchChange { text, .. } => {
				if text.chars().count() < 2 {
					let _ = self.doc.set_search_results(&[]);
					return;
				}
				let needle = text.to_lowercase();
				let mut rows = Vec::new();
				'files: for &(path, _) in vscode_fs::CONTENTS {
					let content = self.content_of(path).to_owned();
					for (index, line) in content.split('\n').enumerate() {
						if !line.to_lowercase().contains(&needle) {
							continue;
						}
						let line_no = index + 1;
						rows.push(
							SearchResultsItem {
								key:     None,
								file:    path.rsplit('/').next().unwrap_or(path).to_owned(),
								line:    line_no.to_string(),
								preview: line.trim().chars().take(60).collect(),
							}
							.with_key(format!("{path}|{line_no}")),
						);
						if rows.len() == 40 {
							break 'files;
						}
					}
				}
				let _ = self.doc.set_search_results(&rows);
			},
			Signal::SearchPick { item, .. } => {
				let Some(split) = item.rfind('|') else { return };
				let path = item[..split].to_owned();
				let line = item[split + 1..].parse::<u32>().ok();
				self.open_file(&path, false, line);
				self.record_nav(&path);
			},
			Signal::ScmPick { item, .. } => {
				let path = item;
				self.open_file(&path, false, None);
				self.record_nav(&path);
			},
			Signal::ActivityPick { meta, .. } => {
				let key = meta.key.rsplit('/').next().unwrap_or("").to_owned();
				if let Some(previous) = self.selected_activity.take() {
					let _ =
						kframe::inst_set_node_state(&mut self.doc.inst, &previous, "selected", false);
				}
				let _ = kframe::inst_set_node_state(&mut self.doc.inst, &meta.key, "selected", true);
				self.selected_activity = Some(meta.key.clone());
				if !["explorer", "search", "scm", "debug", "ext"].contains(&key.as_str()) {
					return;
				}
				let _ = self.doc.set_sidebar_explorer(key == "explorer");
				let _ = self.doc.set_sidebar_search(key == "search");
				let _ = self.doc.set_sidebar_scm(key == "scm");
				let _ = self.doc.set_sidebar_debug(key == "debug");
				let _ = self.doc.set_sidebar_ext(key == "ext");
			},
			Signal::PanelPick { meta, .. } => {
				let key = meta.key.rsplit('/').next().unwrap_or("").to_owned();
				if !["problems", "output", "debugc", "terminal", "ports"].contains(&key.as_str()) {
					return;
				}
				if let Some(previous) = self.selected_panel.take() {
					let _ =
						kframe::inst_set_node_state(&mut self.doc.inst, &previous, "selected", false);
				}
				let _ = kframe::inst_set_node_state(&mut self.doc.inst, &meta.key, "selected", true);
				self.selected_panel = Some(meta.key.clone());
				let _ = self.doc.set_panel_problems(key == "problems");
				let _ = self.doc.set_panel_output(key == "output");
				let _ = self.doc.set_panel_debugc(key == "debugc");
				let _ = self.doc.set_panel_terminal(key == "terminal");
				let _ = self.doc.set_panel_ports(key == "ports");
			},
			Signal::ProblemPick { item, meta } => {
				// Problem rows are keyed nodes, not list items; the decoded
				// identity rides `item` when present, else the escaped
				// trailing key segment.
				let target = if item.is_empty() {
					unescape_segment(meta.key.rsplit('/').next().unwrap_or(""))
				} else {
					item
				};
				let Some(split) = target.rfind('|') else {
					return;
				};
				let path = target[..split].to_owned();
				let line = target[split + 1..].parse::<u32>().ok();
				self.open_file(&path, false, line);
				self.record_nav(&path);
			},
			Signal::NavBack { .. } => {
				if self.nav_index == 0 {
					return;
				}
				self.nav_index -= 1;
				let path = self.nav_history[self.nav_index].clone();
				self.navigating = true;
				self.open_file(&path, false, None);
				self.navigating = false;
				self.sync_nav();
			},
			Signal::NavFwd { .. } => {
				if self.nav_index + 1 >= self.nav_history.len() {
					return;
				}
				self.nav_index += 1;
				let path = self.nav_history[self.nav_index].clone();
				self.navigating = true;
				self.open_file(&path, false, None);
				self.navigating = false;
				self.sync_nav();
			},
			Signal::CodeChange { text, meta, .. } => self.on_code_change(&text, &meta.key),
			Signal::FindChange { text, meta, .. } => {
				let owner = meta
					.key
					.find("/#ed")
					.and_then(|at| last_item_seg(&meta.key[..=at]))
					.unwrap_or_else(|| self.focused_leaf.clone());
				let count = if text.is_empty() {
					0
				} else {
					let content = self.content_of(&self.active_path_of(&owner)).to_owned();
					let haystack = content.to_lowercase();
					let needle = text.to_lowercase();
					let mut count = 0usize;
					let mut at = 0usize;
					while let Some(hit) = haystack[at..].find(&needle) {
						count += 1;
						at += hit + needle.len();
					}
					count
				};
				let status = if count > 0 {
					format!("1 of {count}")
				} else {
					"No results".to_owned()
				};
				if let Some(group) = find_leaf_mut(&mut self.root, &owner) {
					group.find_status = status;
					self.push_model();
				}
			},
			Signal::FindClose { meta, .. } => {
				let owner = meta
					.key
					.find("/#ed")
					.and_then(|at| last_item_seg(&meta.key[..=at]))
					.unwrap_or_else(|| self.focused_leaf.clone());
				if let Some(group) = find_leaf_mut(&mut self.root, &owner) {
					group.show_find = false;
					self.push_model();
				}
			},
			Signal::TermSend { text, meta, .. } => {
				let command = text.trim();
				let first = command.split_whitespace().next().unwrap_or("");
				let response = if command == "ls" {
					vscode_fs::ENTRIES
						.iter()
						.filter(|entry| entry.depth == 0)
						.map(|entry| entry.name)
						.collect::<Vec<_>>()
						.join("\n")
				} else if command == "pwd" {
					"/work/agentfs-cxx".to_owned()
				} else {
					format!("zsh: command not found: {first}")
				};
				self.term_log = format!("{}\ncan@mac slab-lang % {text}\n{response}", self.term_log);
				let log = self.term_log.clone();
				let _ = self.doc.set_panel_termlog(&log);
				let _ = self.doc.set_field_text(&meta.key, "");
			},
			Signal::ChatSend { text, meta, .. } => {
				if text.trim().is_empty() {
					return;
				}
				let key = (self.chat_sessions.len() + 1).to_string();
				self
					.chat_sessions
					.push(ChatSessionsItem { key: None, title: text }.with_key(key));
				let sessions = self.chat_sessions.clone();
				let _ = self.doc.set_chat_sessions(&sessions);
				let _ = self.doc.set_field_text(&meta.key, "");
			},
			Signal::SashSidebar { text, .. } => {
				if let Ok(extent) = text.parse::<f64>() {
					self.pane_sidebar = extent;
				}
			},
			Signal::SashCenter { text, .. } => {
				if let Ok(extent) = text.parse::<f64>() {
					self.pane_chat = self.doc.inst.st.env.vw - CHROME_W - self.pane_sidebar - extent;
				}
			},
			Signal::SashPanel { text, .. } => {
				if let Ok(extent) = text.parse::<f64>() {
					self.pane_panel = self.doc.inst.st.env.vh - CHROME_H - extent;
					self.panel_maximized = false;
					if self.pane_panel > PANEL_FLOOR {
						self.panel_restore = self.pane_panel;
					}
				}
			},
			Signal::PanelMax { .. } => self.toggle_max_panel(),
			Signal::PanelClose { .. } => self.close_panel(),
			_ => {},
		}
	}

	fn on_tab_move(&mut self, item: &str, meta: &gv::SignalMeta) {
		let hit = meta.hit_key.as_str();
		// Over a tab body or its indicators: strip insertion indicator.
		let over_tab_item = ["/#body", "/#indl", "/#indr"]
			.iter()
			.find_map(|suffix| hit.find(suffix))
			.and_then(|cut| last_item_seg(&format!("{}/", &hit[..cut])));
		if let Some(over) = over_tab_item
			&& over != item
		{
			self.set_zone(None);
			let Some(rect) = self.index.body.get(&over).copied() else {
				self.set_indicator(None);
				return;
			};
			let before = meta.x < rect.x + rect.w / 2.0;
			self.set_indicator(Some((over, before)));
			return;
		}
		self.set_indicator(None);
		// Over an editor body: DropOverlay zones.
		if let Some(ed_at) = hit.find("/#ed")
			&& let Some(leaf_key) = last_item_seg(&hit[..=ed_at])
			&& let Some(rect) = self.index.ed.get(&leaf_key).copied()
			&& let Some(src_leaf) = leaf_of(&self.root, item)
		{
			let dir = Self::zone_for(rect, meta.x, meta.y);
			let src_tabs = find_leaf(&self.root, &src_leaf).map_or(0, |group| group.tabs.len());
			// VS Code: no drop of an editor onto itself if the source group
			// would empty, and no self-merge.
			if src_leaf == leaf_key && (src_tabs < 2 || dir == "merge") {
				self.set_zone(None);
				return;
			}
			self.set_zone(Some((leaf_key, dir)));
			return;
		}
		self.set_zone(None);
	}

	fn on_editor_drop(&mut self, meta: &gv::SignalMeta) {
		let src = meta.src_item.clone();
		if src.is_empty() {
			return;
		}
		let leaf_key = meta
			.key
			.find("/#ed")
			.and_then(|at| last_item_seg(&meta.key[..=at]));
		let operation = self.zone.clone();
		self.set_zone(None);
		let Some(leaf_key) = leaf_key else { return };
		let Some((zone_leaf, dir)) = operation else {
			return;
		};
		if zone_leaf != leaf_key {
			return;
		}
		if dir == "merge" {
			if leaf_of(&self.root, &src).as_deref() == Some(leaf_key.as_str()) {
				return;
			}
			self.move_tab_to_leaf(&src, &leaf_key, usize::MAX);
		} else {
			let reference = self
				.index
				.pane
				.get(&leaf_key)
				.map(|(_, rect)| *rect)
				.or_else(|| self.index.ed.get(&leaf_key).copied());
			let horizontal = dir == "left" || dir == "right";
			let half = reference.map_or(0.0, |rect| if horizontal { rect.w } else { rect.h } / 2.0);
			if let Some(fresh) = self.split_leaf(&leaf_key, dir, &src)
				&& half > 0.0
			{
				// Sizing.Split: both panes take half the reference extent.
				self.pending_splits.push((fresh, half));
				self.pending_splits.push((leaf_key, half));
			}
		}
		self.push_model();
	}

	fn on_tab_menu(&mut self, item: &str, meta: &gv::SignalMeta) {
		let Some(strip_at) = meta.key.find("/#strip") else {
			return;
		};
		let Some(leaf_key) = last_item_seg(&meta.key[..=strip_at]) else {
			return;
		};
		let Some(target) = find_leaf(&self.root, &leaf_key)
			.and_then(|group| group.tabs.iter().find(|tab| tab.key == item))
		else {
			return;
		};
		let pin_label = if target.preview { "Keep Open" } else { "Pin" };
		self.menu_target = Some((leaf_key, item.to_owned()));
		let anchor = meta.key.clone();
		let _ = self.doc.set_menu_anchor(&anchor);
		let item_of = |key: &str, label: &str| {
			MenuItemsItem { key: None, label: label.to_owned(), enabled: true }.with_key(key)
		};
		let _ = self.doc.set_menu_items(&[
			item_of("close", "Close"),
			item_of("closeOthers", "Close Others"),
			item_of("closeSaved", "Close Saved"),
			item_of("pin", pin_label),
			item_of("closeAll", "Close All"),
		]);
		let _ = self.doc.set_menu_open(true);
	}

	fn on_menu_pick(&mut self, item: &str) {
		if let Some((leaf_key, tab_key)) = self.menu_target.clone() {
			match item {
				"close" => self.close_tab(&tab_key),
				"closeOthers" => {
					if let Some(group) = find_leaf_mut(&mut self.root, &leaf_key) {
						group.tabs.retain(|tab| tab.key == tab_key);
					}
					self.activate(&leaf_key, &tab_key);
				},
				"closeSaved" => {
					if let Some(group) = find_leaf_mut(&mut self.root, &leaf_key) {
						group.tabs.retain(|tab| tab.dirty);
					}
					let first = find_leaf(&self.root, &leaf_key).and_then(|group| {
						(!group.tabs.is_empty() && !group.tabs.iter().any(|tab| tab.active))
							.then(|| group.tabs[0].key.clone())
					});
					if let Some(first) = first {
						self.activate(&leaf_key, &first);
					}
					self.prune_empty();
				},
				"pin" => {
					if let Some(group) = find_leaf_mut(&mut self.root, &leaf_key)
						&& let Some(tab) = group.tabs.iter_mut().find(|tab| tab.key == tab_key)
					{
						tab.preview = false;
					}
				},
				"closeAll" => {
					if let Some(group) = find_leaf_mut(&mut self.root, &leaf_key) {
						group.tabs.clear();
					}
					self.prune_empty();
				},
				_ => {},
			}
			self.push_model();
		}
		self.close_menu();
	}

	fn on_tree_pick(&mut self, item: &str, meta: &gv::SignalMeta) {
		let Some(entry) = vscode_fs::ENTRIES.iter().find(|entry| entry.path == item) else {
			return;
		};
		if entry.dir {
			if !self.open_dirs.remove(item) {
				self.open_dirs.insert(item.to_owned());
			}
			self.push_tree();
			return;
		}
		item.clone_into(&mut self.selected_path);
		if let Some(previous) = self.selected_tree_row.take() {
			let _ = kframe::inst_set_node_state(&mut self.doc.inst, &previous, "selected", false);
		}
		let _ = kframe::inst_set_node_state(&mut self.doc.inst, &meta.key, "selected", true);
		self.selected_tree_row = Some(meta.key.clone());
		let now = Instant::now();
		let preview = self.last_tree_pick.0 != item
			|| now.duration_since(self.last_tree_pick.1).as_millis() > 450;
		self.last_tree_pick = (item.to_owned(), now);
		let path = item.to_owned();
		self.open_file(&path, preview, None);
		self.record_nav(&path);
	}

	fn on_code_change(&mut self, text: &str, key: &str) {
		let Some(leaf_key) = leaf_key_from_editor_key(key) else {
			return;
		};
		let path = self.active_path_of(&leaf_key);
		if path.is_empty() {
			return;
		}
		// Seeding echoes a kernel Change with identical text; only real user
		// edits may pin the preview tab or mark it dirty.
		if text == self.content_of(&path) {
			return;
		}
		let previous_lines = self.content_of(&path).split('\n').count();
		self.contents.insert(path, text.to_owned());
		let _ = kframe::inst_set_field_styles(&mut self.doc.inst, key, &highlight(text));
		let mut tab_changed = false;
		if let Some(group) = find_leaf_mut(&mut self.root, &leaf_key)
			&& let Some(tab) = group.tabs.iter_mut().find(|tab| tab.active)
		{
			tab_changed = tab.preview || !tab.dirty;
			tab.preview = false;
			tab.dirty = true;
		}
		if tab_changed || text.split('\n').count() != previous_lines {
			self.push_model();
		}
	}
}

impl Default for VscodeHost {
	fn default() -> Self {
		Self::new()
	}
}

impl ShellHost<()> for VscodeHost {
	fn effects(&mut self, document: &mut NativeDocument, effects: &dispatch::Effects) {
		// The shell owns the live instance; borrow it into the typed document
		// wrapper for the duration of the callback.
		std::mem::swap(&mut self.doc.inst, &mut document.inst);
		let signals = self.doc.decode_signals(effects);
		for signal in signals {
			self.on_signal(signal);
		}
		self.update_caret_status();
		std::mem::swap(&mut self.doc.inst, &mut document.inst);
	}

	fn after_solve(&mut self, document: &mut NativeDocument) {
		// Runs only right after a kernel solve, so the staged writes see the
		// fresh scene; each mutation marks the instance dirty and the shell
		// schedules the follow-up frame that paints it.
		if !self.needs_drain {
			return;
		}
		std::mem::swap(&mut self.doc.inst, &mut document.inst);
		self.drain();
		std::mem::swap(&mut self.doc.inst, &mut document.inst);
	}
}

/// Opens the VS Code demo in a winit window.
pub fn run(opts: demo::Opts) -> Result<(), String> {
	if opts.headless_out.is_some() {
		return Err("--demo vscode is windowed; drop --headless-frame".into());
	}
	let mut host = VscodeHost::new();
	if !host.ok() {
		return Err("embedded vscode SLIR failed to decode".into());
	}
	host.initialize(opts.width, opts.height, opts.dark);
	let inst = std::mem::replace(&mut host.doc.inst, kframe::inst_shell());
	let imgs = std::mem::take(&mut host.doc.imgs);
	let document = NativeDocument::from_parts(inst, imgs);

	let options = ShellOptions {
		title:         "slab — vscode".to_owned(),
		width:         opts.width,
		height:        opts.height,
		dark:          opts.dark,
		undecorated:   opts.undecorated,
		max_frames:    opts.max_frames,
		exit_after_ms: opts.exit_after_ms,
		stats:         opts.stats,
		stats_csv:     opts.stats_csv,
	};
	let event_loop = EventLoop::<ShellEvent<()>>::with_user_event()
		.build()
		.map_err(|error| error.to_string())?;
	event_loop.set_control_flow(ControlFlow::Wait);
	let mut app = NativeShell::new(document, options, event_loop.create_proxy(), host);
	event_loop
		.run_app(&mut app)
		.map_err(|error| error.to_string())?;
	eprintln!("slab-native: presented {} frames", app.frames);
	app.finish_stats()?;
	Ok(())
}

#[cfg(test)]
mod tests {
	use slab_kernel::dispatch::{E_POINTER_DOWN, E_POINTER_MOVE, E_POINTER_UP, Event};

	use super::*;

	const fn pointer(etype: u32, x: f64, y: f64) -> Event {
		Event {
			etype,
			x,
			y,
			dx: 0.0,
			dy: 0.0,
			button: 0,
			clicks: 1,
			key: String::new(),
			text: String::new(),
			clauses: Vec::new(),
			mods: 0,
		}
	}

	/// Simulates the shell loop: dispatch, decode, handle, then solve and
	/// drain staged scene work like [`ShellHost::after_solve`].
	fn click(host: &mut VscodeHost, t: &mut f64, x: f64, y: f64) {
		for etype in [E_POINTER_MOVE, E_POINTER_DOWN, E_POINTER_UP] {
			let (_, signals) = host.doc.dispatch(&pointer(etype, x, y));
			for signal in signals {
				host.on_signal(signal);
			}
		}
		settle(host, t);
	}

	fn settle(host: &mut VscodeHost, t: &mut f64) {
		for _ in 0..4 {
			*t += 16.0;
			let _ = host.doc.frame(*t);
			if host.needs_drain {
				host.drain();
			}
		}
	}

	fn rect_center(host: &VscodeHost, key: &str) -> (f64, f64) {
		let inst = &host.doc.inst;
		let node = scene::node_by_key(inst.doc(), &inst.st.lists, key);
		assert_ne!(node, slir::NONE, "scene key resolves: {key}");
		let index = scene::index_of(&inst.sc, node);
		assert!(index >= 0, "scene entry exists: {key}");
		let entry = &inst.sc.entries[usize::try_from(index).expect("scene index")];
		(entry.x + entry.w / 2.0, entry.y + entry.h / 2.0)
	}

	fn host() -> (VscodeHost, f64) {
		let mut host = VscodeHost::new();
		assert!(host.ok(), "embedded document decodes");
		host.initialize(1568.0, 844.0, true);
		let mut t = 1000.0;
		settle(&mut host, &mut t);
		(host, t)
	}

	#[test]
	fn initialize_seeds_active_editors() {
		let (host, _) = host();
		let field = host.index.field.get("gB").expect("gB editor field indexed");
		let seeded = host.doc.field_text(field).expect("field text");
		assert_eq!(
			seeded,
			host.content_of("include/agentfs/store.hpp"),
			"store.hpp seeds the focused editor"
		);
		assert_eq!(
			host.last_seeded.get("gA").map(String::as_str),
			Some(MDB_PATH),
			"mdb editor seeded"
		);
	}

	#[test]
	fn tree_pick_opens_preview_tab_and_seeds_it() {
		let (mut host, mut t) = host();
		let row = gv::keys::item_key(gv::keys::TREEROWS, "README.md", "row@0");
		// README.md is the last tree row and can sit below the viewport clip;
		// scroll the tree to the bottom so the click lands on a visible rect.
		let scroll = gv::keys::TREEROWS
			.strip_suffix("/#treerows")
			.expect("tree rows live under the tree scroll container");
		assert!(
			kframe::inst_set_scroll(&mut host.doc.inst, scroll, 0, f64::from(u16::MAX)),
			"tree scroll container accepts an offset"
		);
		settle(&mut host, &mut t);
		let (x, y) = rect_center(&host, &row);
		click(&mut host, &mut t, x, y);
		let group = find_leaf(&host.root, "gB").expect("focused leaf");
		let tab = group.tabs.iter().find(|tab| tab.key == "README.md");
		let tab = tab.expect("tree pick opened a README.md tab in the focused group");
		assert!(tab.preview, "single click opens a preview tab");
		assert!(tab.active, "opened tab activates");
		let field = host.index.field.get("gB").expect("gB editor field indexed");
		assert_eq!(
			host.doc.field_text(field).expect("field text"),
			host.content_of("README.md"),
			"editor reseeded with the picked file"
		);
	}

	#[test]
	fn close_tab_collapses_empty_group() {
		let (mut host, mut t) = host();
		host.on_signal(Signal::TabClose {
			item: "mdb.hpp".to_owned(),
			meta: gv::SignalMeta {
				x:           -1.0,
				y:           -1.0,
				dx:          0.0,
				dy:          0.0,
				drag_dx:     0.0,
				drag_dy:     0.0,
				mods:        0,
				button:      0,
				clicks:      0,
				key:         String::new(),
				src_key:     String::new(),
				src_item:    String::new(),
				cancelled:   false,
				dropped:     false,
				hit_key:     String::new(),
				pressed_key: String::new(),
			},
		});
		settle(&mut host, &mut t);
		assert!(find_leaf(&host.root, "gA").is_none(), "closing the only tab drops the empty group");
		assert!(find_leaf(&host.root, "gB").is_some(), "sibling group survives");
	}
}
