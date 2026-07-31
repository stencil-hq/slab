//! Compact retained storage for scalar parameter values.

use crate::slir;

const NO_SLOT: usize = usize::MAX;

/// Retained scalar parameter values grouped into dense type-compatible lanes.
#[derive(Clone, Debug, Default)]
pub struct ParamStore {
	slots:    Vec<usize>,
	numbers:  Vec<f64>,
	texts:    Vec<String>,
	colors:   Vec<u32>,
	booleans: Vec<u64>,
	symbols:  Vec<String>,
}

impl ParamStore {
	/// Rebuilds retained values from the document's decoded parameter defaults.
	pub(super) fn init(&mut self, d: &slir::Doc) {
		self.slots.clear();
		self.numbers.clear();
		self.texts.clear();
		self.colors.clear();
		self.symbols.clear();
		self.booleans.clear();
		self.booleans.resize(d.parm_type.len().div_ceil(64), 0);
		self.slots.reserve(d.parm_type.len());

		for (param, &kind) in d.parm_type.iter().enumerate() {
			let encoded = d.parm_default[param];
			let decoded = crate::value::decode(d, i32::from_ne_bytes(encoded.to_ne_bytes()));
			let slot = match kind {
				slir::PARAM_TEXT => {
					let slot = self.texts.len();
					self.texts.push(if decoded.tag == slir::T_STR {
						slir::str_at(d, decoded.h).to_owned()
					} else {
						String::new()
					});
					slot
				},
				slir::PARAM_NUM | slir::PARAM_PCT => {
					let slot = self.numbers.len();
					self.numbers.push(decoded.num);
					slot
				},
				slir::PARAM_COLOR => {
					let slot = self.colors.len();
					self.colors.push(decoded.h);
					slot
				},
				slir::PARAM_BOOL => {
					if decoded.num != 0.0 {
						self.booleans[param / 64] |= 1 << (param % 64);
					}
					NO_SLOT
				},
				slir::PARAM_ENUM => {
					let slot = self.symbols.len();
					self.symbols.push(if decoded.tag == slir::T_ENUM_SYM {
						slir::str_at(d, decoded.h).to_owned()
					} else {
						String::new()
					});
					slot
				},
				slir::PARAM_LIST => NO_SLOT,
				_ => NO_SLOT,
			};
			self.slots.push(slot);
		}
	}

	/// Returns a numeric or percentage parameter value.
	pub(super) fn number(&self, param: usize) -> f64 {
		self.numbers[self.slots[param]]
	}

	/// Returns a text parameter value.
	pub(super) fn text(&self, param: usize) -> &str {
		&self.texts[self.slots[param]]
	}

	/// Returns a packed RGBA8 color parameter value.
	pub(super) fn color(&self, param: usize) -> u32 {
		self.colors[self.slots[param]]
	}

	/// Returns a boolean parameter value.
	pub(super) fn boolean(&self, param: usize) -> bool {
		(self.booleans[param / 64] & (1 << (param % 64))) != 0
	}

	/// Returns an enum parameter member name.
	pub(super) fn symbol(&self, param: usize) -> &str {
		&self.symbols[self.slots[param]]
	}

	/// Sets a numeric or percentage value and reports whether it changed.
	pub(super) fn set_number(&mut self, param: usize, value: f64) -> bool {
		let retained = &mut self.numbers[self.slots[param]];
		if *retained == value {
			return false;
		}
		*retained = value;
		true
	}

	/// Sets a text value and reports whether it changed.
	pub(super) fn set_text(&mut self, param: usize, value: &str) -> bool {
		let retained = &mut self.texts[self.slots[param]];
		if retained == value {
			return false;
		}
		value.clone_into(retained);
		true
	}

	/// Sets a packed RGBA8 color value and reports whether it changed.
	pub(super) fn set_color(&mut self, param: usize, value: u32) -> bool {
		let retained = &mut self.colors[self.slots[param]];
		if *retained == value {
			return false;
		}
		*retained = value;
		true
	}

	/// Sets a boolean value and reports whether it changed.
	pub(super) fn set_boolean(&mut self, param: usize, value: bool) -> bool {
		let bit = 1 << (param % 64);
		let retained = &mut self.booleans[param / 64];
		let previous = (*retained & bit) != 0;
		if previous == value {
			return false;
		}
		*retained ^= bit;
		true
	}

	/// Sets an enum member name and reports whether it changed.
	pub(super) fn set_symbol(&mut self, param: usize, value: &str) -> bool {
		let retained = &mut self.symbols[self.slots[param]];
		if retained == value {
			return false;
		}
		value.clone_into(retained);
		true
	}
}
