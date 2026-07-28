package slabtui

import (
	"strconv"
	"unicode/utf8"

	"github.com/stencil-hq/slab/clients/go/slab"
)

// EventKind classifies a decoded terminal input event.
type EventKind int

const (
	// EventKey is a key press. Key holds the SDP key name.
	EventKey EventKind = iota
	// EventText is a text insertion. Text holds the inserted characters.
	EventText
	// EventPaste is a bracketed paste. Text holds the pasted content.
	EventPaste
	// EventPointerDown is a mouse button press.
	EventPointerDown
	// EventPointerUp is a mouse button release.
	EventPointerUp
	// EventPointerMove is a mouse motion, with or without a button held.
	EventPointerMove
	// EventWheel is a wheel notch.
	EventWheel
	// EventQuit is Ctrl+C, which the driver never forwards to the document.
	EventQuit
)

// String names the event kind for diagnostics.
func (k EventKind) String() string {
	names := [...]string{"key", "text", "paste", "pointer.down", "pointer.up", "pointer.move", "wheel", "quit"}
	if int(k) < 0 || int(k) >= len(names) {
		return "unknown"
	}
	return names[k]
}

// Event is one decoded terminal input event.
//
// Only the fields relevant to Kind carry meaning; the rest stay at zero.
type Event struct {
	// Kind selects which fields are meaningful.
	Kind EventKind
	// Key is the SDP key name for [EventKey].
	Key string
	// Text is the inserted or pasted text for [EventText] and [EventPaste].
	Text string
	// Mods lists the held modifiers in `shift`, `alt`, `ctrl`, `meta` order.
	Mods []string
	// Col is the zero-based terminal column of a pointer or wheel event.
	Col int
	// Row is the zero-based terminal row of a pointer or wheel event.
	Row int
	// Button is the pointer button code for press, release, and drag events.
	Button int
	// WheelUp reports a wheel notch scrolling up rather than down.
	WheelUp bool
}

// PointerXY converts a pointer or wheel event cell to layout units at the
// center of the cell.
func (e Event) PointerXY() (float64, float64) {
	return float64(e.Col)*slab.CellWidth + slab.CellWidth/2,
		float64(e.Row)*slab.CellHeight + slab.CellHeight/2
}

// WheelDY returns the wheel delta in layout units: three rows per notch,
// negative when scrolling up.
func (e Event) WheelDY() float64 {
	if e.WheelUp {
		return -3 * slab.CellHeight
	}
	return 3 * slab.CellHeight
}

// Modifier bits shared by the xterm parameter encoding and SGR mouse reports.
const (
	bitShift = 1
	bitAlt   = 2
	bitCtrl  = 4
	bitMeta  = 8
)

// modsOf expands a modifier bitmask into SDP modifier names.
func modsOf(bits int) []string {
	if bits == 0 {
		return nil
	}
	mods := make([]string, 0, 4)
	if bits&bitShift != 0 {
		mods = append(mods, slab.ModShift)
	}
	if bits&bitAlt != 0 {
		mods = append(mods, slab.ModAlt)
	}
	if bits&bitCtrl != 0 {
		mods = append(mods, slab.ModCtrl)
	}
	if bits&bitMeta != 0 {
		mods = append(mods, slab.ModMeta)
	}
	return mods
}

// acceptsText reports whether a printable key also produces text input.
//
// A ctrl, alt, or meta chord is a command, not typing.
func acceptsText(bits int) bool {
	return bits&(bitCtrl|bitAlt|bitMeta) == 0
}

// Decoder turns a raw terminal byte stream into [Event] values.
//
// Feed it every chunk read from the terminal; it buffers partial escape
// sequences across calls. The zero value is ready to use, and a decoder is not
// safe for concurrent use.
type Decoder struct {
	buf     []byte
	paste   []byte
	inPaste bool
}

// pasteEnd is the bracketed-paste terminator.
var pasteEnd = []byte("\x1b[201~")

// Feed consumes chunk and returns every event it completes, in order.
//
// Bytes that end mid-sequence stay buffered for the next call. A trailing lone
// escape byte resolves to the `Escape` key, because a terminal emits a whole
// escape sequence in a single write.
func (d *Decoder) Feed(chunk []byte) []Event {
	d.buf = append(d.buf, chunk...)
	var events []Event
	offset := 0
	for offset < len(d.buf) {
		rest := d.buf[offset:]
		if d.inPaste {
			consumed, done := d.consumePaste(rest)
			offset += consumed
			if !done {
				break
			}
			events = append(events, Event{Kind: EventPaste, Text: string(d.paste)})
			d.paste = d.paste[:0]
			continue
		}
		decoded, consumed := decodeOne(rest, d)
		if consumed == 0 {
			break
		}
		offset += consumed
		events = append(events, decoded...)
	}
	d.buf = append(d.buf[:0], d.buf[offset:]...)
	return events
}

// consumePaste accumulates paste body bytes from buf.
//
// It returns the bytes consumed and whether the terminator has been seen. A
// partial terminator at the end of buf stays unconsumed for the next chunk.
func (d *Decoder) consumePaste(buf []byte) (int, bool) {
	for index := range buf {
		if buf[index] != 0x1b {
			continue
		}
		tail := buf[index:]
		if len(tail) < len(pasteEnd) {
			if string(tail) == string(pasteEnd[:len(tail)]) {
				d.paste = append(d.paste, buf[:index]...)
				return index, false
			}
			continue
		}
		if string(tail[:len(pasteEnd)]) == string(pasteEnd) {
			d.paste = append(d.paste, buf[:index]...)
			d.inPaste = false
			return index + len(pasteEnd), true
		}
	}
	d.paste = append(d.paste, buf...)
	return len(buf), false
}

// decodeOne decodes the events at the front of buf.
//
// A zero byte count means buf holds an incomplete sequence. The decoder is
// passed so a bracketed-paste introducer can switch the stream into paste mode.
func decodeOne(buf []byte, d *Decoder) ([]Event, int) {
	head := buf[0]
	switch {
	case head == 0x1b:
		return decodeEscape(buf, d)
	case head == 0x03:
		return []Event{{Kind: EventQuit}}, 1
	case head == 0x09:
		return []Event{{Kind: EventKey, Key: "Tab"}}, 1
	case head == 0x0d, head == 0x0a:
		return []Event{{Kind: EventKey, Key: "Enter"}}, 1
	case head == 0x08, head == 0x7f:
		return []Event{{Kind: EventKey, Key: "Backspace"}}, 1
	case head < 0x20:
		return controlChord(head, 0), 1
	default:
		return decodeText(buf, 0)
	}
}

// controlChord maps a C0 byte to its Ctrl+letter key event.
func controlChord(head byte, extra int) []Event {
	switch {
	case head == 0:
		return []Event{{Kind: EventKey, Key: " ", Mods: modsOf(bitCtrl | extra)}}
	case head >= 0x1c && head <= 0x1f:
		names := [...]string{"\\", "]", "^", "_"}
		return []Event{{Kind: EventKey, Key: names[head-0x1c], Mods: modsOf(bitCtrl | extra)}}
	default:
		return []Event{{Kind: EventKey, Key: string(rune('a' + head - 1)), Mods: modsOf(bitCtrl | extra)}}
	}
}

// decodeText decodes one UTF-8 rune as a key plus, when unmodified, text.
func decodeText(buf []byte, extra int) ([]Event, int) {
	character, size := utf8.DecodeRune(buf)
	if character == utf8.RuneError && size <= 1 {
		if !utf8.FullRune(buf) && len(buf) < utf8.UTFMax {
			return nil, 0
		}
		return nil, 1
	}
	text := string(character)
	events := []Event{{Kind: EventKey, Key: text, Mods: modsOf(extra)}}
	if acceptsText(extra) {
		events = append(events, Event{Kind: EventText, Text: text})
	}
	return events, size
}

// decodeEscape decodes a sequence that starts with the escape byte.
func decodeEscape(buf []byte, d *Decoder) ([]Event, int) {
	if len(buf) == 1 {
		return []Event{{Kind: EventKey, Key: "Escape"}}, 1
	}
	switch buf[1] {
	case '[':
		return decodeCSI(buf, d)
	case 'O':
		return decodeSS3(buf)
	case 0x1b:
		return []Event{{Kind: EventKey, Key: "Escape"}}, 1
	default:
		if buf[1] < 0x20 {
			return controlChord(buf[1], bitAlt), 2
		}
		events, consumed := decodeText(buf[1:], bitAlt)
		if consumed == 0 {
			return nil, 0
		}
		return events, consumed + 1
	}
}

// ss3Keys maps single-shift-three finals to key names.
var ss3Keys = map[byte]string{
	'A': "ArrowUp", 'B': "ArrowDown", 'C': "ArrowRight", 'D': "ArrowLeft",
	'H': "Home", 'F': "End",
	'P': "F1", 'Q': "F2", 'R': "F3", 'S': "F4",
}

// decodeSS3 decodes an `ESC O x` application-mode sequence.
func decodeSS3(buf []byte) ([]Event, int) {
	if len(buf) < 3 {
		return nil, 0
	}
	name, ok := ss3Keys[buf[2]]
	if !ok {
		return nil, 3
	}
	return []Event{{Kind: EventKey, Key: name}}, 3
}

// csiFinals maps a CSI final byte to a key name.
var csiFinals = map[byte]string{
	'A': "ArrowUp", 'B': "ArrowDown", 'C': "ArrowRight", 'D': "ArrowLeft",
	'H': "Home", 'F': "End",
	'P': "F1", 'Q': "F2", 'S': "F4",
}

// tildeKeys maps the `CSI n ~` parameter to a key name.
var tildeKeys = map[int]string{
	1: "Home", 2: "Insert", 3: "Delete", 4: "End", 5: "PageUp", 6: "PageDown",
	7: "Home", 8: "End",
	11: "F1", 12: "F2", 13: "F3", 14: "F4", 15: "F5",
	17: "F6", 18: "F7", 19: "F8", 20: "F9", 21: "F10",
	23: "F11", 24: "F12", 25: "F13", 26: "F14", 28: "F15", 29: "F16",
	31: "F17", 32: "F18", 33: "F19", 34: "F20",
}

// decodeCSI decodes a control-sequence-introducer sequence.
func decodeCSI(buf []byte, d *Decoder) ([]Event, int) {
	index := 2
	for index < len(buf) && (buf[index] < 0x40 || buf[index] > 0x7e) {
		index++
	}
	if index >= len(buf) {
		return nil, 0
	}
	final := buf[index]
	body := string(buf[2:index])
	consumed := index + 1
	if len(body) > 0 && body[0] == '<' {
		return decodeMouse(body[1:], final), consumed
	}
	params := parseParams(body)
	if final == '~' && len(params) > 0 && params[0] == 200 {
		d.inPaste = true
		return nil, consumed
	}
	mods := 0
	if len(params) > 1 && params[1] > 0 {
		mods = params[1] - 1
	}
	switch {
	case final == 'Z':
		return []Event{{Kind: EventKey, Key: "Tab", Mods: modsOf(mods | bitShift)}}, consumed
	case final == '~':
		if len(params) == 0 {
			return nil, consumed
		}
		name, ok := tildeKeys[params[0]]
		if !ok {
			return nil, consumed
		}
		return []Event{{Kind: EventKey, Key: name, Mods: modsOf(mods)}}, consumed
	case final == 'R' && len(params) >= 2:
		// A cursor position report, not F3; drivers never ask for one.
		return nil, consumed
	}
	name, ok := csiFinals[final]
	if !ok {
		return nil, consumed
	}
	return []Event{{Kind: EventKey, Key: name, Mods: modsOf(mods)}}, consumed
}

// parseParams splits a semicolon-separated CSI parameter list.
func parseParams(body string) []int {
	if body == "" {
		return nil
	}
	params := make([]int, 0, 3)
	start := 0
	for index := 0; index <= len(body); index++ {
		if index < len(body) && body[index] != ';' {
			continue
		}
		value, err := strconv.Atoi(body[start:index])
		if err != nil {
			value = 0
		}
		params = append(params, value)
		start = index + 1
	}
	return params
}

// decodeMouse decodes one SGR (mode 1006) mouse report.
func decodeMouse(body string, final byte) []Event {
	params := parseParams(body)
	if len(params) < 3 {
		return nil
	}
	code := params[0]
	col := max(params[1]-1, 0)
	row := max(params[2]-1, 0)
	mods := 0
	if code&4 != 0 {
		mods |= bitShift
	}
	if code&8 != 0 {
		mods |= bitAlt
	}
	if code&16 != 0 {
		mods |= bitCtrl
	}
	event := Event{Col: col, Row: row, Mods: modsOf(mods), Button: code & 3}
	switch {
	case code&64 != 0:
		event.Kind = EventWheel
		event.WheelUp = code&1 == 0
		event.Button = 0
	case code&32 != 0:
		event.Kind = EventPointerMove
		if event.Button == 3 {
			event.Button = 0
		}
	case final == 'm':
		event.Kind = EventPointerUp
	default:
		event.Kind = EventPointerDown
	}
	return []Event{event}
}
