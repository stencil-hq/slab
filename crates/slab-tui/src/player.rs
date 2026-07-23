//! `--app player`: a real music-player app on top of the 00-player card.
//! The kernel owns rendering/focus/hover/press; this layer owns the
//! playlist and the clock. Transport signals (toggle/next/prev/shuffle/
//! loop) mutate app state, which is pushed back through the kernel's
//! type-checked `inst_set_param` (title/artist/elapsed/remain/progress/
//! playing) so the ordinary dirty-frame loop repaints. The queue hole
//! stays blank on tui (holes are host-filled; cells has no host).

use slab_kernel::frame as kframe;

pub struct Track {
    pub title: &'static str,
    pub len_ms: f64,
}

/// The Sunset Tree, in card order. Track 1 matches the document's param
/// defaults (elapsed 2:37 of 4:12 is the declared 62%).
pub const TRACKS: [Track; 4] = [
    Track {
        title: "Pale Green Things",
        len_ms: 252_000.0, // 4:12
    },
    Track {
        title: "This Year",
        len_ms: 245_000.0, // 4:05
    },
    Track {
        title: "Love Love Love",
        len_ms: 194_000.0, // 3:14
    },
    Track {
        title: "Dance Music",
        len_ms: 119_000.0, // 1:59
    },
];

const ARTIST: &str = "The Mountain Goats";

/// Starting position inside track 1: the doc's declared elapsed=2:37.
const START_ELAPSED_MS: f64 = 157_000.0;

pub struct PlayerApp {
    cur: usize,
    playing: bool,
    pub shuffle: bool,
    pub looping: bool,
    elapsed_ms: f64,
    rng: u32,
}

/// `mm:ss` from a millisecond count (floored to whole seconds).
fn mmss(ms: f64) -> String {
    let s = (ms / 1000.0).floor().max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn pv_text(s: &str) -> kframe::ParamValue {
    kframe::ParamValue {
        kind: 0,
        num: 0.0,
        s: s.to_string(),
        rgba: 0,
        sym: String::new(),
    }
}

fn pv_num(kind: u32, num: f64) -> kframe::ParamValue {
    kframe::ParamValue {
        kind,
        num,
        s: String::new(),
        rgba: 0,
        sym: String::new(),
    }
}

/// Set a declared param by name through the kernel's type check.
fn set(inst: &mut kframe::Instance, name: &str, v: &kframe::ParamValue) -> Result<(), String> {
    let doc = &inst.doc;
    let p = (0..doc.parm_name.len())
        .position(|p| doc.strs[doc.parm_name[p] as usize] == name)
        .ok_or_else(|| format!("--app player: document has no param '{name}'"))?;
    if !kframe::inst_set_param(inst, p as u32, v) {
        return Err(format!("--app player: param '{name}' has the wrong type"));
    }
    Ok(())
}

impl PlayerApp {
    /// Start paused at the doc's declared position in track 1; push the
    /// initial state through the kernel (fails on a doc without the
    /// player's params — --app player needs the player card).
    pub fn new(inst: &mut kframe::Instance) -> Result<Self, String> {
        let app = PlayerApp {
            cur: 0,
            playing: false,
            shuffle: false,
            looping: false,
            elapsed_ms: START_ELAPSED_MS,
            rng: 0x9E3779B9,
        };
        app.sync(inst)?;
        Ok(app)
    }

    /// Push title/artist/times/progress/playing into the kernel; the
    /// set_param dirty bit makes the next frame re-solve and repaint.
    fn sync(&self, inst: &mut kframe::Instance) -> Result<(), String> {
        let tr = &TRACKS[self.cur];
        let remain = (tr.len_ms - self.elapsed_ms).max(0.0);
        set(inst, "title", &pv_text(tr.title))?;
        set(inst, "artist", &pv_text(ARTIST))?;
        set(inst, "elapsed", &pv_text(&mmss(self.elapsed_ms)))?;
        set(inst, "remain", &pv_text(&format!("-{}", mmss(remain))))?;
        set(
            inst,
            "progress",
            &pv_num(2, (self.elapsed_ms / tr.len_ms * 100.0).clamp(0.0, 100.0)),
        )?;
        set(inst, "playing", &pv_num(4, f64::from(self.playing)))?;
        Ok(())
    }

    /// Next track index: sequential, or any OTHER track when shuffling
    /// (deterministic LCG so scripted runs replay identically).
    fn next_ix(&mut self) -> usize {
        if self.shuffle {
            self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let skip = (self.rng >> 16) as usize % (TRACKS.len() - 1);
            (self.cur + 1 + skip) % TRACKS.len()
        } else {
            (self.cur + 1) % TRACKS.len()
        }
    }

    fn goto(&mut self, inst: &mut kframe::Instance, ix: usize) -> Result<(), String> {
        self.cur = ix % TRACKS.len();
        self.elapsed_ms = 0.0;
        self.sync(inst)
    }

    /// React to a transport signal emitted by the kernel.
    pub fn on_signal(&mut self, inst: &mut kframe::Instance, name: &str) -> Result<(), String> {
        match name {
            "toggle" => {
                self.playing = !self.playing;
                self.sync(inst)
            }
            "next" => {
                let ix = self.next_ix();
                self.goto(inst, ix)
            }
            "prev" => {
                let ix = (self.cur + TRACKS.len() - 1) % TRACKS.len();
                self.goto(inst, ix)
            }
            "shuffle" => {
                self.shuffle = !self.shuffle;
                Ok(())
            }
            "loop" => {
                self.looping = !self.looping;
                Ok(())
            }
            _ => Ok(()), // pick etc: queue rows live in the (blank) hole
        }
    }

    /// Advance the play clock by dt milliseconds. At track end: restart
    /// when looping, else auto-advance to the next track.
    pub fn advance(&mut self, inst: &mut kframe::Instance, dt_ms: f64) -> Result<(), String> {
        if !self.playing || dt_ms <= 0.0 {
            return Ok(());
        }
        self.elapsed_ms += dt_ms;
        while self.elapsed_ms >= TRACKS[self.cur].len_ms {
            self.elapsed_ms -= TRACKS[self.cur].len_ms;
            if !self.looping {
                let ix = self.next_ix();
                self.cur = ix;
            }
        }
        self.sync(inst)
    }

    /// Debug-footer badge text: active modes, e.g. " [SHUF] [LOOP]".
    pub fn badges(&self) -> String {
        let mut out = String::new();
        if self.shuffle {
            out.push_str(" [SHUF]");
        }
        if self.looping {
            out.push_str(" [LOOP]");
        }
        out
    }
}
