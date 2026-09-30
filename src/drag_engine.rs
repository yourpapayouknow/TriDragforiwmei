use std::collections::HashMap;
use std::time::{Duration, Instant};

use log::{debug, trace};

use crate::config::{Config, DevCfg, RLS_FNG_THR_MS};
use crate::mouse::{BtEv, Point};

/// A single touchpad contact with its identifier and coordinates.
#[derive(Debug, Clone, Copy)]
pub struct TpCtc {
    pub id: i32,
    pub x: i32,
    pub y: i32,
}

impl TpCtc {
    /// Euclidean distance to another contact.
    pub fn getdst2d(&self, other: &Self) -> f32 {
        let dx = (other.x - self.x) as f32;
        let dy = (other.y - self.y) as f32;
        (dx * dx + dy * dy).sqrt()
    }
}

/// Recognises three-finger drags and reports the resulting cursor motion and
/// button edges.
pub struct DrgEng {
    isdrgging: bool,
    dstmgr: DstMgr,
    fngcnt: FngCnt,
    lastctc: Vec<TpCtc>,
    lasttime: Instant,
    avgx: f32,
    avgy: f32,
    avgcnt: u32,
    pendbtn: Option<BtEv>,
}

impl DrgEng {
    /// Creates an engine with no contacts seen yet.
    pub fn new() -> Self {
        Self {
            isdrgging: false,
            dstmgr: DstMgr::new(),
            fngcnt: FngCnt::new(),
            lastctc: Vec::new(),
            lasttime: Instant::now(),
            avgx: 0.0,
            avgy: 0.0,
            avgcnt: 0,
            pendbtn: None,
        }
    }

    /// Advances the state machine with one contact report, returning the cursor
    /// offset to apply when a drag is in progress.
    pub fn onctc(&mut self, ctc: &[TpCtc], config: &Config, devid: &str) -> Option<Point> {
        let now = Instant::now();
        let elapsed = now.duration_since(self.lasttime).as_millis() as u32;
        self.lasttime = now;
        let released = elapsed > RLS_FNG_THR_MS;

        let idscommon = Self::areidscmn(&self.lastctc, ctc);
        let (longid, longdelta, longdist) = self.dstmgr.longest(&self.lastctc, ctc, released);

        let (fngcnt, shortmv, longmv, origcnt) = self
            .fngcnt
            .count(ctc, idscommon, longdist, released, config, devid);

        trace!(
            "fingers={fngcnt} original={origcnt} moving={shortmv}/{longmv} dist={longdist} id={longid}"
        );

        if fngcnt >= 3 && idscommon && longmv == 3 && origcnt == 3 && !self.isdrgging {
            self.isdrgging = true;
            self.pendbtn = Some(BtEv::Down);
            debug!("START DRAG");
        } else if self.isdrgging && (shortmv < 2 || (origcnt != 3 && origcnt >= 2)) {
            debug!("STOP DRAG");
            self.stopdrg();
        } else if fngcnt >= 2 && origcnt == 3 && idscommon && self.isdrgging && longdist > 0.0 {
            let devcfg = config.devcfg(devid);
            if devcfg.cursor_move
                && (config.max_finger_move_distance == 0.0
                    || longdist <= config.max_finger_move_distance)
            {
                let delta = aplspdacc(longdelta, elapsed, &devcfg);
                if config.cursor_averaging > 1 {
                    self.avgx += delta.x;
                    self.avgy += delta.y;
                    self.avgcnt += 1;
                    if self.avgcnt >= config.cursor_averaging {
                        let out = Point::new(self.avgx, self.avgy);
                        self.avgx = 0.0;
                        self.avgy = 0.0;
                        self.avgcnt = 0;
                        self.lastctc = ctc.to_vec();
                        return Some(out);
                    }
                } else {
                    self.lastctc = ctc.to_vec();
                    return Some(delta);
                }
            }
        }

        self.lastctc = ctc.to_vec();
        None
    }

    /// Takes the pending button edge, if any, for the caller to emit.
    pub fn btnevnt(&mut self) -> Option<BtEv> {
        self.pendbtn.take()
    }

    /// Whether a drag is currently held.
    pub fn isdrgging(&self) -> bool {
        self.isdrgging
    }

    /// Ends the drag when the release timer fires.
    pub fn onrltmr(&mut self) {
        if self.isdrgging {
            debug!("STOP DRAG from timer");
            self.stopdrg();
        }
    }

    /// Clears drag state and queues the button release.
    fn stopdrg(&mut self) {
        self.isdrgging = false;
        self.pendbtn = Some(BtEv::Up);
        self.dstmgr.reset();
        self.fngcnt.reset();
        self.avgx = 0.0;
        self.avgy = 0.0;
        self.avgcnt = 0;
    }

    /// Whether two contact lists describe the same set of contact identifiers.
    fn areidscmn(a: &[TpCtc], b: &[TpCtc]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        b.iter().all(|bc| a.iter().any(|ac| ac.id == bc.id))
    }
}

/// Tracks which contacts have settled long enough to be measured, and the
/// longest movement seen between the old and new reports.
struct DstMgr {
    quarantine: HashMap<i32, Instant>,
    trusted: Vec<i32>,
}

impl DstMgr {
    /// Creates an empty tracker.
    fn new() -> Self {
        Self {
            quarantine: HashMap::new(),
            trusted: Vec::new(),
        }
    }

    /// Forgets all contacts, e.g. after the fingers were lifted.
    fn reset(&mut self) {
        self.quarantine.clear();
        self.trusted.clear();
    }

    /// Returns the identifier, offset and 2D distance of the contact that moved
    /// furthest, ignoring contacts still inside the settle window.
    fn longest(&mut self, old: &[TpCtc], new: &[TpCtc], released: bool) -> (i32, Point, f32) {
        if released {
            self.reset();
            return (0, Point::default(), 0.0);
        }

        self.trusted.retain(|id| new.iter().any(|c| c.id == *id));
        self.quarantine
            .retain(|id, _| new.iter().any(|c| c.id == *id));

        for c in new {
            if let Some(t) = self.quarantine.get(&c.id) {
                if t.elapsed() > Duration::from_millis(RLS_FNG_THR_MS as u64) {
                    self.trusted.push(c.id);
                    self.quarantine.remove(&c.id);
                }
            } else {
                self.quarantine.insert(c.id, Instant::now());
            }
        }

        let mut longid = 0;
        let mut longdelta = Point::default();
        let mut longdist = 0.0f32;

        for nc in new {
            if !self.trusted.contains(&nc.id) {
                continue;
            }
            for oc in old {
                if nc.id != oc.id {
                    continue;
                }
                let d = nc.getdst2d(oc);
                if d > longdist {
                    longdist = d;
                    longid = nc.id;
                    longdelta = Point::new((nc.x - oc.x) as f32, (nc.y - oc.y) as f32);
                }
                break;
            }
        }
        (longid, longdelta, longdist)
    }
}

/// Counts moving fingers over a short and a long movement window, which
/// separates an accidental brush from an intended three-finger drag.
struct FngCnt {
    origcnt: i32,
    shortmv: f32,
    longmv: f32,
    shortcnt: i32,
    longcnt: i32,
}

impl FngCnt {
    /// Creates a counter with all windows empty.
    fn new() -> Self {
        Self {
            origcnt: 0,
            shortmv: 0.0,
            longmv: 0.0,
            shortcnt: 0,
            longcnt: 0,
        }
    }

    /// Clears all counters.
    fn reset(&mut self) {
        self.origcnt = 0;
        self.shortmv = 0.0;
        self.longmv = 0.0;
        self.shortcnt = 0;
        self.longcnt = 0;
    }

    /// Accumulates movement and returns (fingers on pad, short-delay moving
    /// count, long-delay moving count, original finger count).
    fn count(
        &mut self,
        ctc: &[TpCtc],
        idscommon: bool,
        longdist: f32,
        released: bool,
        config: &Config,
        devid: &str,
    ) -> (i32, i32, i32, i32) {
        if !idscommon && (ctc.len() <= 1 || released) {
            self.origcnt = 0;
        }
        if !idscommon || released {
            self.shortmv = 0.0;
            self.longmv = 0.0;
            return (0, self.shortcnt, self.longcnt, self.origcnt);
        }

        let dist = aplspd(longdist, config, devid);
        if dist >= 1.0 {
            self.shortmv += dist;
            self.longmv += dist;
        }

        if self.shortmv >= config.stop_threshold {
            self.shortcnt = ctc.len() as i32;
            self.shortmv = 0.0;
        }
        if self.longmv > config.start_threshold {
            self.longcnt = ctc.len() as i32;
            self.longmv = 0.0;
            if self.origcnt <= 1 {
                self.origcnt = ctc.len() as i32;
            }
        }

        (ctc.len() as i32, self.shortcnt, self.longcnt, self.origcnt)
    }
}

/// Scales a distance by the device cursor speed for threshold accumulation.
fn aplspd(distance: f32, config: &Config, devid: &str) -> f32 {
    distance * (config.devcfg(devid).cursor_speed / 60.0)
}

/// Scales a drag offset by the device speed and the acceleration curve.
fn aplspdacc(delta: Point, elapsed_ms: u32, devcfg: &DevCfg) -> Point {
    let mut d = delta;
    d.multiply(devcfg.cursor_speed / 120.0);

    let mousevel = (d.length() / elapsed_ms.max(1) as f32).min(4.0);
    let a = devcfg.cursor_acceleration / 10.0;
    let ptrvel = if a != 0.0 {
        let exp =
            2.6 * a * (mousevel - 1.0 + (3.0 - ((0.8_f32 / 0.3_f32) - 1.0_f32).log2()) / (2.6 * a))
                - 3.0;
        let k = exp.exp();
        0.7 + 0.8 * (k / (1.0 + k))
    } else {
        1.0
    };

    d.multiply(ptrvel);
    d
}
