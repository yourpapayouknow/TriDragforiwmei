use std::sync::Mutex;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
    MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEINPUT,
    MOUSE_EVENT_FLAGS,
};

use crate::config::Btn;

// 两次移动间保留的亚像素余量，SendInput 只能按整像素移动
static FRACTION: Mutex<(f32, f32)> = Mutex::new((0.0, 0.0));

// 引擎要求发出的按键边沿
#[derive(Debug, Clone, Copy)]
pub enum BtEv {
    Down,
    Up,
}

// 触摸板坐标系下的二维偏移
#[derive(Debug, Clone, Copy, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    // 由原始坐标构造
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    // 偏移的欧氏长度
    pub fn length(&self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    // 就地对两个分量做缩放
    pub fn multiply(&mut self, m: f32) {
        self.x *= m;
        self.y *= m;
    }
}

// 按相对偏移移动光标，并结转亚像素余量
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

// 按下配置指定的鼠标键
pub fn sndbtndwn(button: Btn) {
    let flag = match button {
        Btn::Left => MOUSEEVENTF_LEFTDOWN,
        Btn::Right => MOUSEEVENTF_RIGHTDOWN,
        Btn::Middle => MOUSEEVENTF_MIDDLEDOWN,
    };
    sndmsinp(0, 0, flag);
}

// 松开配置指定的鼠标键
pub fn sndbtnup(button: Btn) {
    let flag = match button {
        Btn::Left => MOUSEEVENTF_LEFTUP,
        Btn::Right => MOUSEEVENTF_RIGHTUP,
        Btn::Middle => MOUSEEVENTF_MIDDLEUP,
    };
    sndmsinp(0, 0, flag);
}

// 注入一次鼠标输入事件
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
