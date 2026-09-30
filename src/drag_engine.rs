use std::collections::HashMap;
use std::time::{Duration, Instant};

use log::{debug, trace};

use crate::config::{Config, DeviceConfig, RELEASE_FINGERS_THRESHOLD_MS};
use crate::mouse::{ButtonEvent, Point};

#[derive(Debug, Clone, Copy)]
pub struct TouchpadContact {
    pub id: i32,
    pub x: i32,
    pub y: i32,
}

impl TouchpadContact {
    pub fn dist2d(&self, other: &Self) -> f32 {
        let dx = (other.x - self.x) as f32;
        let dy = (other.y - self.y) as f32;
        (dx * dx + dy * dy).sqrt()
    }
}

pub struct DragEngine {
    is_dragging: bool,
    distance_manager: DistanceManager,
    finger_counter: FingerCounter,
    last_contacts: Vec<TouchpadContact>,
    last_time: Instant,
    averaging_x: f32,
    averaging_y: f32,
    averaging_count: u32,
    pending_button: Option<ButtonEvent>,
}

impl DragEngine {
    pub fn new() -> Self {
        Self {
            is_dragging: false,
            distance_manager: DistanceManager::new(),
            finger_counter: FingerCounter::new(),
            last_contacts: Vec::new(),
            last_time: Instant::now(),
            averaging_x: 0.0,
            averaging_y: 0.0,
            averaging_count: 0,
            pending_button: None,
        }
    }

    pub fn on_contacts(
        &mut self,
        contacts: &[TouchpadContact],
        config: &Config,
        device_id: &str,
    ) -> Option<Point> {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_time).as_millis() as u32;
        self.last_time = now;
        let has_fingers_released = elapsed > RELEASE_FINGERS_THRESHOLD_MS;

        let are_ids_common = Self::are_ids_common(&self.last_contacts, contacts);
        let (longest_id, longest_delta, longest_dist2d) =
            self.distance_manager
                .longest(&self.last_contacts, contacts, has_fingers_released);

        let (fingers_count, short_delay_moving, long_delay_moving, original_count) =
            self.finger_counter.count(
                contacts,
                are_ids_common,
                longest_dist2d,
                has_fingers_released,
                config,
                device_id,
            );

        // Per-report logging: kept at trace level so the hot path costs nothing
        // unless verbose logging is explicitly enabled.
        trace!(
            "fingers={fingers_count} original={original_count} moving={short_delay_moving}/{long_delay_moving} dist={longest_dist2d} id={longest_id}"
        );

        if fingers_count >= 3
            && are_ids_common
            && long_delay_moving == 3
            && original_count == 3
            && !self.is_dragging
        {
            self.is_dragging = true;
            self.pending_button = Some(ButtonEvent::Down);
            debug!("START DRAG");
        } else if self.is_dragging
            && (short_delay_moving < 2 || (original_count != 3 && original_count >= 2))
        {
            debug!("STOP DRAG");
            self.stop_drag();
        } else if fingers_count >= 2
            && original_count == 3
            && are_ids_common
            && self.is_dragging
            && longest_dist2d > 0.0
        {
            let dev_cfg = config.device_config(device_id);
            if dev_cfg.cursor_move
                && (config.max_finger_move_distance == 0.0
                    || longest_dist2d <= config.max_finger_move_distance)
            {
                let delta = apply_speed_and_acc(longest_delta, elapsed, &dev_cfg);
                if config.cursor_averaging > 1 {
                    self.averaging_x += delta.x;
                    self.averaging_y += delta.y;
                    self.averaging_count += 1;
                    if self.averaging_count >= config.cursor_averaging {
                        let out = Point::new(self.averaging_x, self.averaging_y);
                        self.averaging_x = 0.0;
                        self.averaging_y = 0.0;
                        self.averaging_count = 0;
                        self.last_contacts = contacts.to_vec();
                        return Some(out);
                    }
                } else {
                    self.last_contacts = contacts.to_vec();
                    return Some(delta);
                }
            }
        }

        self.last_contacts = contacts.to_vec();
        None
    }

    pub fn button_event(&mut self) -> Option<ButtonEvent> {
        self.pending_button.take()
    }

    pub fn is_dragging(&self) -> bool {
        self.is_dragging
    }

    pub fn on_release_timer(&mut self) {
        if self.is_dragging {
            debug!("STOP DRAG from timer");
            self.stop_drag();
        }
    }

    fn stop_drag(&mut self) {
        self.is_dragging = false;
        self.pending_button = Some(ButtonEvent::Up);
        self.distance_manager.reset();
        self.finger_counter.reset();
        self.averaging_x = 0.0;
        self.averaging_y = 0.0;
        self.averaging_count = 0;
    }

    fn are_ids_common(a: &[TouchpadContact], b: &[TouchpadContact]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        b.iter().all(|bc| a.iter().any(|ac| ac.id == bc.id))
    }
}

struct DistanceManager {
    quarantine: HashMap<i32, Instant>,
    trusted: Vec<i32>,
}

impl DistanceManager {
    fn new() -> Self {
        Self {
            quarantine: HashMap::new(),
            trusted: Vec::new(),
        }
    }

    fn reset(&mut self) {
        self.quarantine.clear();
        self.trusted.clear();
    }

    fn longest(
        &mut self,
        old: &[TouchpadContact],
        new: &[TouchpadContact],
        has_released: bool,
    ) -> (i32, Point, f32) {
        if has_released {
            self.reset();
            return (0, Point::default(), 0.0);
        }

        self.trusted.retain(|id| new.iter().any(|c| c.id == *id));
        self.quarantine
            .retain(|id, _| new.iter().any(|c| c.id == *id));

        for c in new {
            if let Some(t) = self.quarantine.get(&c.id) {
                if t.elapsed() > Duration::from_millis(RELEASE_FINGERS_THRESHOLD_MS as u64) {
                    self.trusted.push(c.id);
                    self.quarantine.remove(&c.id);
                }
            } else {
                self.quarantine.insert(c.id, Instant::now());
            }
        }

        let mut longest_id = 0;
        let mut longest_delta = Point::default();
        let mut longest_dist = 0.0f32;

        for nc in new {
            if !self.trusted.contains(&nc.id) {
                continue;
            }
            for oc in old {
                if nc.id != oc.id {
                    continue;
                }
                let d = nc.dist2d(oc);
                if d > longest_dist {
                    longest_dist = d;
                    longest_id = nc.id;
                    longest_delta = Point::new((nc.x - oc.x) as f32, (nc.y - oc.y) as f32);
                }
                break;
            }
        }
        (longest_id, longest_delta, longest_dist)
    }
}

struct FingerCounter {
    original_count: i32,
    short_delay_move: f32,
    long_delay_move: f32,
    short_delay_count: i32,
    long_delay_count: i32,
}

impl FingerCounter {
    fn new() -> Self {
        Self {
            original_count: 0,
            short_delay_move: 0.0,
            long_delay_move: 0.0,
            short_delay_count: 0,
            long_delay_count: 0,
        }
    }

    fn reset(&mut self) {
        self.original_count = 0;
        self.short_delay_move = 0.0;
        self.long_delay_move = 0.0;
        self.short_delay_count = 0;
        self.long_delay_count = 0;
    }

    fn count(
        &mut self,
        contacts: &[TouchpadContact],
        are_ids_common: bool,
        longest_dist2d: f32,
        has_released: bool,
        config: &Config,
        device_id: &str,
    ) -> (i32, i32, i32, i32) {
        if !are_ids_common && (contacts.len() <= 1 || has_released) {
            self.original_count = 0;
        }
        if !are_ids_common || has_released {
            self.short_delay_move = 0.0;
            self.long_delay_move = 0.0;
            return (
                0,
                self.short_delay_count,
                self.long_delay_count,
                self.original_count,
            );
        }

        let dist = apply_speed(longest_dist2d, config, device_id);
        if dist >= 1.0 {
            self.short_delay_move += dist;
            self.long_delay_move += dist;
        }

        if self.short_delay_move >= config.stop_threshold {
            self.short_delay_count = contacts.len() as i32;
            self.short_delay_move = 0.0;
        }
        if self.long_delay_move > config.start_threshold {
            self.long_delay_count = contacts.len() as i32;
            self.long_delay_move = 0.0;
            if self.original_count <= 1 {
                self.original_count = contacts.len() as i32;
            }
        }

        (
            contacts.len() as i32,
            self.short_delay_count,
            self.long_delay_count,
            self.original_count,
        )
    }
}

fn apply_speed(distance: f32, config: &Config, device_id: &str) -> f32 {
    distance * (config.device_config(device_id).cursor_speed / 60.0)
}

fn apply_speed_and_acc(delta: Point, elapsed_ms: u32, dev_cfg: &DeviceConfig) -> Point {
    let mut d = delta;
    d.multiply(dev_cfg.cursor_speed / 120.0);

    let mouse_velocity = (d.length() / elapsed_ms.max(1) as f32).min(4.0);
    let a = dev_cfg.cursor_acceleration / 10.0;
    let pointer_velocity = if a != 0.0 {
        let exp = 2.6
            * a
            * (mouse_velocity - 1.0 + (3.0 - ((0.8_f32 / 0.3_f32) - 1.0_f32).log2()) / (2.6 * a))
            - 3.0;
        let k = exp.exp();
        let sigmoid = k / (1.0 + k);
        0.7 + 0.8 * sigmoid
    } else {
        1.0
    };

    d.multiply(pointer_velocity);
    d
}
