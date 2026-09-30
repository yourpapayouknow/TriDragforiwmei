use std::sync::Mutex;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
    MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEINPUT,
    MOUSE_EVENT_FLAGS,
};

use crate::config::Btn;

/// Sub-pixel remainder carried between moves, since SendInput moves in whole
/// pixels only.
static FRACTION: Mutex<(f32, f32)> = Mutex::new((0.0, 0.0));

/// Which edge of a mouse button the engine wants to emit.
#[derive(Debug, Clone, Copy)]
pub enum BtEv {
    Down,
    Up,
}

/// A 2D offset in touchpad units.
#[derive(Debug, Clone, Copy, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    /// Builds a point from raw coordinates.
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Euclidean length of the offset.
    pub fn length(&self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    /// Scales both components in place.
    pub fn multiply(&mut self, m: f32) {
        self.x *= m;
        self.y *= m;
    }
}

/// Moves the cursor by a relative offset, carrying the sub-pixel remainder.
pub fn sndmov(dx: f32, dy: f32) {
    let (ix, iy) = {
        let mut frac = FRACTION.lock().unwrap();
        let totx = dx + frac.0;
        let toty = dy + frac.1;
        let ix = totx as i32;
        let iy = toty as i32;
        frac.0 = totx - ix as f32;
        frac.1 = toty - iy as f32;
        (ix, iy)
    };

    if ix == 0 && iy == 0 {
        return;
    }

    sndmsinp(ix, iy, MOUSEEVENTF_MOVE);
}

/// Presses the configured mouse button.
pub fn sndbtndwn(button: Btn) {
    let flag = match button {
        Btn::Left => MOUSEEVENTF_LEFTDOWN,
        Btn::Right => MOUSEEVENTF_RIGHTDOWN,
        Btn::Middle => MOUSEEVENTF_MIDDLEDOWN,
    };
    sndmsinp(0, 0, flag);
}

/// Releases the configured mouse button.
pub fn sndbtnup(button: Btn) {
    let flag = match button {
        Btn::Left => MOUSEEVENTF_LEFTUP,
        Btn::Right => MOUSEEVENTF_RIGHTUP,
        Btn::Middle => MOUSEEVENTF_MIDDLEUP,
    };
    sndmsinp(0, 0, flag);
}

/// Injects a single mouse input event.
fn sndmsinp(dx: i32, dy: i32, flags: MOUSE_EVENT_FLAGS) {
    let input = INPUT {
        r#type: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}
