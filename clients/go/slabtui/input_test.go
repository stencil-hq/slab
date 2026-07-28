package slabtui

import (
	"reflect"
	"testing"
)

// decode feeds one complete byte stream through a fresh decoder.
func decode(input string) []Event {
	var decoder Decoder
	return decoder.Feed([]byte(input))
}

func TestDecodeNamedKeys(t *testing.T) {
	cases := []struct {
		input string
		key   string
		mods  []string
	}{
		{"\x1b[A", "ArrowUp", nil},
		{"\x1b[B", "ArrowDown", nil},
		{"\x1b[C", "ArrowRight", nil},
		{"\x1b[D", "ArrowLeft", nil},
		{"\x1b[H", "Home", nil},
		{"\x1b[F", "End", nil},
		{"\x1b[2~", "Insert", nil},
		{"\x1b[3~", "Delete", nil},
		{"\x1b[5~", "PageUp", nil},
		{"\x1b[6~", "PageDown", nil},
		{"\x1b[15~", "F5", nil},
		{"\x1b[24~", "F12", nil},
		{"\x1bOP", "F1", nil},
		{"\x1bOB", "ArrowDown", nil},
		{"\t", "Tab", nil},
		{"\r", "Enter", nil},
		{"\n", "Enter", nil},
		{"\x7f", "Backspace", nil},
		{"\x08", "Backspace", nil},
		{"\x1b", "Escape", nil},
		{"\x1b[Z", "Tab", []string{"shift"}},
		{"\x1b[1;5C", "ArrowRight", []string{"ctrl"}},
		{"\x1b[1;2A", "ArrowUp", []string{"shift"}},
		{"\x1b[1;3D", "ArrowLeft", []string{"alt"}},
		{"\x1b[1;7B", "ArrowDown", []string{"alt", "ctrl"}},
		{"\x1b[3;5~", "Delete", []string{"ctrl"}},
	}
	for _, testCase := range cases {
		events := decode(testCase.input)
		if len(events) != 1 {
			t.Errorf("%q decoded to %d events, want 1", testCase.input, len(events))
			continue
		}
		got := events[0]
		if got.Kind != EventKey || got.Key != testCase.key {
			t.Errorf("%q decoded to %s %q, want key %q", testCase.input, got.Kind, got.Key, testCase.key)
		}
		if !reflect.DeepEqual(got.Mods, testCase.mods) {
			t.Errorf("%q mods = %v, want %v", testCase.input, got.Mods, testCase.mods)
		}
	}
}

func TestDecodePrintableEmitsKeyThenText(t *testing.T) {
	events := decode("a")
	if len(events) != 2 {
		t.Fatalf("decoded %d events, want key then text: %+v", len(events), events)
	}
	if events[0].Kind != EventKey || events[0].Key != "a" {
		t.Errorf("first = %s %q, want key \"a\"", events[0].Kind, events[0].Key)
	}
	if events[1].Kind != EventText || events[1].Text != "a" {
		t.Errorf("second = %s %q, want text \"a\"", events[1].Kind, events[1].Text)
	}
}

func TestDecodeUtf8Text(t *testing.T) {
	events := decode("é☃")
	want := []Event{
		{Kind: EventKey, Key: "é"},
		{Kind: EventText, Text: "é"},
		{Kind: EventKey, Key: "☃"},
		{Kind: EventText, Text: "☃"},
	}
	if !reflect.DeepEqual(events, want) {
		t.Errorf("events = %+v, want %+v", events, want)
	}
}

func TestDecodeSplitUtf8RuneWaitsForTheRest(t *testing.T) {
	var decoder Decoder
	head := []byte("é")
	if events := decoder.Feed(head[:1]); len(events) != 0 {
		t.Fatalf("a half rune decoded to %+v, want nothing", events)
	}
	events := decoder.Feed(head[1:])
	if len(events) != 2 || events[0].Key != "é" {
		t.Errorf("events = %+v, want the completed rune", events)
	}
}

func TestDecodeSplitEscapeSequenceWaitsForTheRest(t *testing.T) {
	var decoder Decoder
	if events := decoder.Feed([]byte("\x1b[1")); len(events) != 0 {
		t.Fatalf("a partial CSI decoded to %+v, want nothing", events)
	}
	events := decoder.Feed([]byte(";5C"))
	if len(events) != 1 || events[0].Key != "ArrowRight" {
		t.Fatalf("events = %+v, want ArrowRight", events)
	}
	if !reflect.DeepEqual(events[0].Mods, []string{"ctrl"}) {
		t.Errorf("mods = %v, want [ctrl]", events[0].Mods)
	}
}

func TestDecodeControlChords(t *testing.T) {
	events := decode("\x01")
	if len(events) != 1 || events[0].Key != "a" || !reflect.DeepEqual(events[0].Mods, []string{"ctrl"}) {
		t.Fatalf("Ctrl+A decoded to %+v", events)
	}
	if events[0].Kind == EventText {
		t.Error("a ctrl chord produced text input")
	}
}

func TestDecodeCtrlCIsQuit(t *testing.T) {
	events := decode("\x03")
	if len(events) != 1 || events[0].Kind != EventQuit {
		t.Fatalf("Ctrl+C decoded to %+v, want a quit", events)
	}
}

func TestDecodeAltChordSuppressesText(t *testing.T) {
	events := decode("\x1bx")
	if len(events) != 1 {
		t.Fatalf("Alt+x decoded to %+v, want one key event", events)
	}
	if events[0].Key != "x" || !reflect.DeepEqual(events[0].Mods, []string{"alt"}) {
		t.Errorf("event = %+v, want alt+x", events[0])
	}
}

func TestDecodeSgrMousePressAndRelease(t *testing.T) {
	events := decode("\x1b[<0;10;5M\x1b[<0;10;5m")
	if len(events) != 2 {
		t.Fatalf("decoded %d events, want a press and a release: %+v", len(events), events)
	}
	press, release := events[0], events[1]
	if press.Kind != EventPointerDown || press.Col != 9 || press.Row != 4 || press.Button != 0 {
		t.Errorf("press = %+v, want a left press at cell 9,4", press)
	}
	if release.Kind != EventPointerUp || release.Col != 9 || release.Row != 4 {
		t.Errorf("release = %+v, want a release at cell 9,4", release)
	}
	x, y := press.PointerXY()
	if x != 9*8+4 || y != 4*16+8 {
		t.Errorf("pointer = %g,%g, want the cell center 76,72", x, y)
	}
}

func TestDecodeSgrMouseModifiersAndButtons(t *testing.T) {
	events := decode("\x1b[<18;3;3M")
	if len(events) != 1 {
		t.Fatalf("decoded %+v, want one event", events)
	}
	if events[0].Button != 2 {
		t.Errorf("button = %d, want 2 (right)", events[0].Button)
	}
	if !reflect.DeepEqual(events[0].Mods, []string{"ctrl"}) {
		t.Errorf("mods = %v, want [ctrl]", events[0].Mods)
	}
}

func TestDecodeSgrMouseMotion(t *testing.T) {
	events := decode("\x1b[<35;7;2M")
	if len(events) != 1 || events[0].Kind != EventPointerMove {
		t.Fatalf("decoded %+v, want a motion event", events)
	}
	if events[0].Col != 6 || events[0].Row != 1 {
		t.Errorf("cell = %d,%d, want 6,1", events[0].Col, events[0].Row)
	}
}

func TestDecodeWheelNotches(t *testing.T) {
	events := decode("\x1b[<64;1;1M\x1b[<65;1;1M")
	if len(events) != 2 {
		t.Fatalf("decoded %+v, want two wheel events", events)
	}
	if !events[0].WheelUp || events[0].WheelDY() != -48 {
		t.Errorf("scroll up = %+v, dy %g, want dy -48", events[0], events[0].WheelDY())
	}
	if events[1].WheelUp || events[1].WheelDY() != 48 {
		t.Errorf("scroll down = %+v, dy %g, want dy 48", events[1], events[1].WheelDY())
	}
}

func TestDecodeBracketedPaste(t *testing.T) {
	events := decode("\x1b[200~one\ttwo\x1b[201~x")
	if len(events) != 3 {
		t.Fatalf("decoded %+v, want paste plus key and text", events)
	}
	if events[0].Kind != EventPaste || events[0].Text != "one\ttwo" {
		t.Errorf("paste = %+v, want the bracketed body", events[0])
	}
	if events[1].Kind != EventKey || events[1].Key != "x" {
		t.Errorf("trailing key = %+v, want x", events[1])
	}
}

func TestDecodePasteSplitAcrossChunks(t *testing.T) {
	var decoder Decoder
	if events := decoder.Feed([]byte("\x1b[200~ab")); len(events) != 0 {
		t.Fatalf("an unterminated paste decoded to %+v", events)
	}
	if events := decoder.Feed([]byte("cd\x1b[20")); len(events) != 0 {
		t.Fatalf("a partial terminator decoded to %+v", events)
	}
	events := decoder.Feed([]byte("1~"))
	if len(events) != 1 || events[0].Kind != EventPaste || events[0].Text != "abcd" {
		t.Fatalf("events = %+v, want a paste of \"abcd\"", events)
	}
}

func TestDecodeMixedStream(t *testing.T) {
	events := decode("\t\x1b[Bhi\x1b[<0;2;2M\x03")
	kinds := make([]EventKind, len(events))
	for index, event := range events {
		kinds[index] = event.Kind
	}
	want := []EventKind{
		EventKey, EventKey,
		EventKey, EventText, EventKey, EventText,
		EventPointerDown, EventQuit,
	}
	if !reflect.DeepEqual(kinds, want) {
		t.Errorf("kinds = %v, want %v", kinds, want)
	}
}
