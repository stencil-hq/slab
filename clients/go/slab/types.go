package slab

import (
	"encoding/json"
	"fmt"
)

// Modifier names accepted by SDP input methods.
const (
	// ModShift is the shift modifier.
	ModShift = "shift"
	// ModAlt is the alt (option) modifier.
	ModAlt = "alt"
	// ModCtrl is the control modifier.
	ModCtrl = "ctrl"
	// ModMeta is the meta (command, super) modifier.
	ModMeta = "meta"
)

// Terminal cell geometry used to derive a document-sized environment.
const (
	// CellWidth is the layout width of one terminal column.
	CellWidth = 8.0
	// CellHeight is the layout height of one terminal row.
	CellHeight = 16.0
)

// Pointer button codes used by `input.pointer` and `input.click`.
const (
	// ButtonLeft is the primary pointer button.
	ButtonLeft = 0
	// ButtonMiddle is the middle pointer button.
	ButtonMiddle = 1
	// ButtonRight is the secondary pointer button.
	ButtonRight = 2
)

// EnvSpec is the desired document environment.
//
// Width and Height are layout units, not terminal cells; a terminal host sets
// `width = cols * CellWidth` and `height = rows * CellHeight`. Client is one of
// `web`, `gpu`, `tui`, `svg`, or `png`. An empty Client or Theme is omitted, so
// the session keeps its current value for that field.
type EnvSpec struct {
	// Width is the document width in layout units.
	Width float64 `json:"width"`
	// Height is the document height in layout units.
	Height float64 `json:"height"`
	// Client names the rendering client.
	Client string `json:"client,omitempty"`
	// Dark selects the dark color scheme.
	Dark bool `json:"dark"`
	// Coarse selects coarse (touch) pointer semantics.
	Coarse bool `json:"coarse"`
	// Theme names an authored theme; empty keeps the current one.
	Theme string `json:"theme,omitempty"`
}

// CellsOptions selects how `render.cells` formats the grid.
type CellsOptions struct {
	// Plain drops all styling and emits bare text.
	Plain bool `json:"plain"`
	// Caret paints the text caret of the focused editor.
	Caret bool `json:"caret"`
}

// Cells is one rendered terminal grid.
type Cells struct {
	// Text is the grid, rows joined by newlines; styled unless Plain was set.
	Text string `json:"text"`
	// Cols is the column count.
	Cols int `json:"cols"`
	// Rows is the row count.
	Rows int `json:"rows"`
	// Notes lists renderer diagnostics for this frame.
	Notes []string `json:"notes"`
}

// Rect is a document-space rectangle in layout units.
//
// SDP encodes it as a four-element array, so it decodes from `[x,y,w,h]`.
type Rect struct {
	// X is the left edge.
	X float64
	// Y is the top edge.
	Y float64
	// W is the width.
	W float64
	// H is the height.
	H float64
}

// UnmarshalJSON decodes the SDP `[x,y,w,h]` array form.
func (r *Rect) UnmarshalJSON(data []byte) error {
	var values [4]float64
	if err := json.Unmarshal(data, &values); err != nil {
		return fmt.Errorf("slab: decode rect: %w", err)
	}
	r.X, r.Y, r.W, r.H = values[0], values[1], values[2], values[3]
	return nil
}

// MarshalJSON encodes the SDP `[x,y,w,h]` array form.
func (r Rect) MarshalJSON() ([]byte, error) {
	return json.Marshal([4]float64{r.X, r.Y, r.W, r.H})
}

// SignalMeta is the pointer, keyboard, and drag context of one signal.
type SignalMeta struct {
	// X is the pointer x position in layout units.
	X float64 `json:"x"`
	// Y is the pointer y position in layout units.
	Y float64 `json:"y"`
	// DX is the pointer or wheel delta along x.
	DX float64 `json:"dx"`
	// DY is the pointer or wheel delta along y.
	DY float64 `json:"dy"`
	// DragDX is the accumulated drag delta along x.
	DragDX float64 `json:"drag_dx"`
	// DragDY is the accumulated drag delta along y.
	DragDY float64 `json:"drag_dy"`
	// Mods is the kernel modifier bitmask active for the signal.
	Mods uint32 `json:"mods"`
	// Button is the pointer button code.
	Button uint32 `json:"button"`
	// Clicks is the consecutive click count.
	Clicks uint32 `json:"clicks"`
	// Key is the scene key of the node that emitted the signal.
	Key string `json:"key"`
	// SrcKey is the scene key of a drag source, when there is one.
	SrcKey string `json:"src_key"`
	// SrcItem is the item id of a drag source, when there is one.
	SrcItem string `json:"src_item"`
	// Cancelled reports a cancelled gesture.
	Cancelled bool `json:"cancelled"`
	// Dropped reports a completed drop.
	Dropped bool `json:"dropped"`
}

// Signal is one authored signal emitted by a dispatch, in kernel order.
type Signal struct {
	// Name is the authored signal name.
	Name string `json:"name"`
	// Text is the payload text, for editors and text-bearing signals.
	Text string `json:"text"`
	// Item is the list item id, for signals raised inside an `each`.
	Item string `json:"item"`
	// Meta is the pointer, keyboard, and drag context.
	Meta SignalMeta `json:"meta"`
}

// ScrollChange is one scroll offset the dispatch moved.
type ScrollChange struct {
	// Key is the scene key of the scrolling node.
	Key string `json:"key"`
	// Axis is 0 for horizontal and 1 for vertical.
	Axis uint32 `json:"axis"`
	// Off is the resulting offset in layout units.
	Off float64 `json:"off"`
}

// Effects is everything one input dispatch changed.
type Effects struct {
	// Repaint reports that the frame must be redrawn.
	Repaint bool `json:"repaint"`
	// Signals lists the emitted signals in kernel order.
	Signals []Signal `json:"signals"`
	// Caret is the text caret rectangle, or nil when there is none.
	Caret *Rect `json:"caret"`
	// IME is the input-method candidate rectangle, or nil when there is none.
	IME *Rect `json:"ime"`
	// Cursor is the requested cursor shape id.
	Cursor uint32 `json:"cursor"`
	// Focus is the scene key of the focused node, empty when nothing has focus.
	Focus string `json:"focus"`
	// Scrolls lists the scroll offsets this dispatch changed.
	Scrolls []ScrollChange `json:"scrolls"`
}

// Signal returns the first signal named name, and whether it was found.
func (e Effects) Signal(name string) (Signal, bool) {
	for _, signal := range e.Signals {
		if signal.Name == name {
			return signal, true
		}
	}
	return Signal{}, false
}

// inputResult is the `{"effects":...,"t":...}` envelope of every input method.
type inputResult struct {
	Effects Effects `json:"effects"`
	T       float64 `json:"t"`
}

// ParamInfo is one declared document parameter.
type ParamInfo struct {
	// Name is the parameter name.
	Name string `json:"name"`
	// Type is the declared parameter type.
	Type string `json:"type"`
}

// DocInfo describes the loaded document, as returned by `doc.info`.
type DocInfo struct {
	// File is the source path, empty for a document opened from inline source.
	File string `json:"file"`
	// Params lists the declared parameters.
	Params []ParamInfo `json:"params"`
	// Themes lists the authored theme names.
	Themes []string `json:"themes"`
	// Holes lists the named host content holes.
	Holes []string `json:"holes"`
	// Signals lists every signal name the document can emit.
	Signals []string `json:"signals"`
	// Env is the current environment.
	Env EnvSpec `json:"env"`
	// T is the virtual clock position in milliseconds.
	T float64 `json:"t"`
}
