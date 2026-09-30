use std::sync::Mutex;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
    MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEINPUT,
    MOUSE_EVENT_FLAGS,
};

use crate::config::MouseButton;

static FRACTION: Mutex<(f32, f32)> = Mutex::new((0.0, 0.0));

#[derive(Debug, Clone, Copy)]
pub enum ButtonEvent {
    Down,
    Up,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn length(&self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    pub fn multiply(&mut self, m: f32) {
        self.x *= m;
        self.y *= m;
    }
}

pub fn send_move(dx: f32, dy: f32) {
    let (ix, iy) = {
        let mut frac = FRACTION.lock().unwrap();
        let total_x = dx + frac.0;
        let total_y = dy + frac.1;
        let ix = total_x as i32;
        let iy = total_y as i32;
        frac.0 = total_x - ix as f32;
        frac.1 = total_y - iy as f32;
        (ix, iy)
    };

    if ix == 0 && iy == 0 {
        return;
    }

    send_mouse_input(ix, iy, MOUSEEVENTF_MOVE);
}

pub fn send_button_down(button: MouseButton) {
    let flag = match button {
        MouseButton::Left => MOUSEEVENTF_LEFTDOWN,
        MouseButton::Right => MOUSEEVENTF_RIGHTDOWN,
        MouseButton::Middle => MOUSEEVENTF_MIDDLEDOWN,
    };
    send_mouse_input(0, 0, flag);
}

pub fn send_button_up(button: MouseButton) {
    let flag = match button {
        MouseButton::Left => MOUSEEVENTF_LEFTUP,
        MouseButton::Right => MOUSEEVENTF_RIGHTUP,
        MouseButton::Middle => MOUSEEVENTF_MIDDLEUP,
    };
    send_mouse_input(0, 0, flag);
}

fn send_mouse_input(dx: i32, dy: i32, flags: MOUSE_EVENT_FLAGS) {
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
