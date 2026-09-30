use std::collections::HashMap;
use std::time::{Duration, Instant};

use log::{debug, trace};

use crate::config::{Config, DevCfg, RLS_FNG_THR_MS};
use crate::mouse::{BtEv, Point};

// 单个触摸板触点，含标识与坐标
#[derive(Debug, Clone, Copy)]
pub struct TpCtc {
    pub id: i32,
    pub x: i32,
    pub y: i32,
}

impl TpCtc {
    // 到另一触点的欧氏距离
    pub fn getdst2d(&self, other: &Self) -> f32 {
        let dx = (other.x - self.x) as f32;
        let dy = (other.y - self.y) as f32;
        (dx * dx + dy * dy).sqrt()
    }
}

// 识别三指拖拽，并输出光标位移与按键边沿
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
    // 构造尚未收到任何触点的引擎
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

    // 用一帧触点推进状态机，拖拽中返回待施加的光标位移
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

    // 取走待发出的按键边沿
    pub fn btnevnt(&mut self) -> Option<BtEv> {
        self.pendbtn.take()
    }

    // 当前是否处于拖拽保持状态
    pub fn isdrgging(&self) -> bool {
        self.isdrgging
    }

    // 释放定时器触发时结束拖拽
    pub fn onrltmr(&mut self) {
        if self.isdrgging {
            debug!("STOP DRAG from timer");
            self.stopdrg();
        }
    }

    // 清空拖拽状态并排入按键抬起
    fn stopdrg(&mut self) {
        self.isdrgging = false;
        self.pendbtn = Some(BtEv::Up);
        self.dstmgr.reset();
        self.fngcnt.reset();
        self.avgx = 0.0;
        self.avgy = 0.0;
        self.avgcnt = 0;
    }

    // 两组触点是否描述同一批触点标识
    fn areidscmn(a: &[TpCtc], b: &[TpCtc]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        b.iter().all(|bc| a.iter().any(|ac| ac.id == bc.id))
    }
}

// 记录哪些触点已稳定到可参与测距，以及新旧两帧间的最大位移
struct DstMgr {
    quarantine: HashMap<i32, Instant>,
    trusted: Vec<i32>,
}

impl DstMgr {
    // 构造空的跟踪器
    fn new() -> Self {
        Self {
            quarantine: HashMap::new(),
            trusted: Vec::new(),
        }
    }

    // 清空全部触点记录，例如手指离开后
    fn reset(&mut self) {
        self.quarantine.clear();
        self.trusted.clear();
    }

    // 返回位移最大触点的标识、偏移量与二维距离，稳定期内的触点不参与
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

// 按短、长两个位移窗口统计移动手指数，用于区分误触与有意拖拽
struct FngCnt {
    origcnt: i32,
    shortmv: f32,
    longmv: f32,
    shortcnt: i32,
    longcnt: i32,
}

impl FngCnt {
    // 构造各窗口为空的计数器
    fn new() -> Self {
        Self {
            origcnt: 0,
            shortmv: 0.0,
            longmv: 0.0,
            shortcnt: 0,
            longcnt: 0,
        }
    }

    // 清空全部计数
    fn reset(&mut self) {
        self.origcnt = 0;
        self.shortmv = 0.0;
        self.longmv = 0.0;
        self.shortcnt = 0;
        self.longcnt = 0;
    }

    // 累计位移，返回（板上手指数，短延迟移动数，长延迟移动数，初始手指数）
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

// 按设备光标速度缩放位移，供阈值累计使用
fn aplspd(distance: f32, config: &Config, devid: &str) -> f32 {
    distance * (config.devcfg(devid).cursor_speed / 60.0)
}

// 按设备速度与加速度曲线缩放拖拽偏移
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
