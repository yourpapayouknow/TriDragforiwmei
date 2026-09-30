use std::collections::HashMap;
use std::ptr;

use anyhow::Result;
use windows::Win32::Devices::HumanInterfaceDevice::{
    HidP_GetCaps, HidP_GetUsageValue, HidP_GetValueCaps, HidP_Input, HIDP_STATUS_SUCCESS,
    HIDP_VALUE_CAPS, PHIDP_PREPARSED_DATA,
};
use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::UI::Input::{
    GetRawInputData, GetRawInputDeviceInfoW, RegisterRawInputDevices, HRAWINPUT, RAWINPUT,
    RAWINPUTDEVICE, RAWINPUTHEADER, RIDI_DEVICEINFO, RIDI_DEVICENAME, RIDI_PREPARSEDDATA,
    RID_DEVICE_INFO, RID_INPUT, RIM_TYPEHID,
};

use crate::drag_engine::{DrgEng, TpCtc};

// 释放延迟到期后用于结束拖拽的定时器标识
pub const RLS_TMR_ID: usize = 1;

// 精密触摸板的 HID 用途页与用途
const PTP_UP: u16 = 0x000D;
const PTP_US: u16 = 0x0005;

// 原始输入标志：失焦时仍接收输入，并接收设备变更通知
const RIDEV_FLAGS: u32 = 0x0000_2100;

// 持有原始输入注册与设备缓存，并向拖拽引擎供给触点
pub struct TchpdEng {
    pub engine: DrgEng,
    devices: HashMap<isize, DevCaps>,
    curdev: Option<isize>,
}

// 按设备缓存的数据，设备连接期间预解析数据与用途上限不会变化
struct DevCaps {
    devid: String,
    prsdt: PrsDt,
    valcaps: Vec<HIDP_VALUE_CAPS>,
}

impl TchpdEng {
    // 构造缓存为空的引擎
    pub fn new() -> Self {
        Self {
            engine: DrgEng::new(),
            devices: HashMap::new(),
            curdev: None,
        }
    }

    // 注册窗口以接收精密触摸板输入报告
    pub fn register(hwnd: HWND) -> Result<()> {
        let device = RAWINPUTDEVICE {
            usUsagePage: PTP_UP,
            usUsage: PTP_US,
            dwFlags: windows::Win32::UI::Input::RAWINPUTDEVICE_FLAGS(RIDEV_FLAGS),
            hwndTarget: hwnd,
        };
        unsafe {
            RegisterRawInputDevices(&[device], std::mem::size_of::<RAWINPUTDEVICE>() as u32)
                .map_err(|e| anyhow::anyhow!("注册原始输入失败: {e:?}"))?;
        }
        Ok(())
    }

    // 设备增删后使缓存的设备数据失效
    pub fn ondevchg(&mut self, _hdev: HANDLE) {
        self.devices.clear();
        self.curdev = None;
    }

    // 将 WM_INPUT 载荷解析为触点列表，失败时返回 None
    pub fn prsinp(&mut self, lparam: isize) -> Option<Vec<TpCtc>> {
        unsafe {
            let buf = self.rdrawinp(lparam)?;
            let raw = ptr::read(buf.as_ptr() as *const RAWINPUT);

            let hdev = raw.header.hDevice.0 as isize;
            self.curdev = Some(hdev);
            if !self.devices.contains_key(&hdev) {
                let caps = self.fchdevcaps(hdev, raw.header.hDevice)?;
                self.devices.insert(hdev, caps);
            }

            // HID 载荷位于原始输入缓冲区的尾部
            let hid = raw.data.hid;
            let hidlen = (hid.dwSizeHid * hid.dwCount) as usize;
            let hiddata = &buf[buf.len() - hidlen..];

            let caps = self.devices.get(&hdev)?;
            self.prsctc(
                hiddata,
                hid.dwSizeHid,
                hid.dwCount,
                &caps.prsdt,
                &caps.valcaps,
            )
        }
    }

    // 按 WM_INPUT 句柄复制完整的原始输入载荷
    unsafe fn rdrawinp(&self, lparam: isize) -> Option<Vec<u8>> {
        let mut size = 0u32;
        let hdrsz = std::mem::size_of::<RAWINPUTHEADER>() as u32;
        if GetRawInputData(HRAWINPUT(lparam as _), RID_INPUT, None, &mut size, hdrsz) != 0 {
            return None;
        }
        if size == 0 {
            return None;
        }

        let mut buf = vec![0u8; size as usize];
        let mut filled = size;
        if GetRawInputData(
            HRAWINPUT(lparam as _),
            RID_INPUT,
            Some(buf.as_mut_ptr() as _),
            &mut filled,
            hdrsz,
        ) != size
        {
            return None;
        }
        Some(buf)
    }

    // 解析设备标识与用途上限，供后续报告复用
    unsafe fn fchdevcaps(&self, hdev: isize, handle: HANDLE) -> Option<DevCaps> {
        let devid = self.cmpdevid(hdev)?;
        let prsdt = self.fchprsdt(handle)?;
        let valcaps = qryvalcap(&prsdt)?;
        Some(DevCaps {
            devid,
            prsdt,
            valcaps,
        })
    }

    // 读取设备的 HID 预解析数据
    unsafe fn fchprsdt(&self, handle: HANDLE) -> Option<PrsDt> {
        let mut size = 0u32;
        if GetRawInputDeviceInfoW(Some(handle), RIDI_PREPARSEDDATA, None, &mut size) != 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        if GetRawInputDeviceInfoW(
            Some(handle),
            RIDI_PREPARSEDDATA,
            Some(buf.as_mut_ptr() as _),
            &mut size,
        ) != size
        {
            return None;
        }
        Some(PrsDt(buf))
    }

    // 依据用途上限从每个 HID 报告还原一个触点
    unsafe fn prsctc(
        &self,
        rawdata: &[u8],
        hidlen: u32,
        hidcnt: u32,
        prsdt: &PrsDt,
        valcaps: &[HIDP_VALUE_CAPS],
    ) -> Option<Vec<TpCtc>> {
        let pptr = PHIDP_PREPARSED_DATA(prsdt.0.as_ptr() as isize);

        let mut ctccnt = 0u32;
        let mut makers: Vec<CtcMaker> = Vec::new();
        let mut contacts: Vec<TpCtc> = Vec::new();

        for cap in valcaps {
            let usage = cap.Anonymous.NotRange.Usage;

            for idx in 0..hidcnt {
                let off = (hidlen * idx) as usize;
                if off >= rawdata.len() {
                    continue;
                }

                let mut value = 0u32;
                if HidP_GetUsageValue(
                    HidP_Input,
                    cap.UsagePage,
                    Some(cap.LinkCollection),
                    usage,
                    &mut value,
                    pptr,
                    &rawdata[off..],
                ) != HIDP_STATUS_SUCCESS
                {
                    continue;
                }

                if cap.LinkCollection == 0 {
                    if cap.UsagePage == 0x0D && usage == 0x54 {
                        ctccnt = value;
                    }
                } else {
                    while makers.len() <= idx as usize {
                        makers.push(CtcMaker::default());
                    }
                    match (cap.UsagePage, usage) {
                        (0x0D, 0x51) => makers[idx as usize].cid = Some(value as i32),
                        (0x01, 0x30) => makers[idx as usize].cx = Some(value as i32),
                        (0x01, 0x31) => makers[idx as usize].cy = Some(value as i32),
                        _ => {}
                    }
                }
            }

            for maker in &mut makers {
                if let Some(c) = maker.build() {
                    if ctccnt == 0 || contacts.len() < ctccnt as usize {
                        contacts.push(c);
                        maker.clear();
                    }
                }
            }
            if ctccnt != 0 && contacts.len() >= ctccnt as usize {
                break;
            }
        }

        Some(contacts)
    }

    // 将设备名散列为稳定标识，用作配置键
    unsafe fn cmpdevid(&self, hdev: isize) -> Option<String> {
        let mut size = 0u32;
        if GetRawInputDeviceInfoW(Some(HANDLE(hdev as _)), RIDI_DEVICENAME, None, &mut size) != 0 {
            return None;
        }
        if size == 0 {
            return None;
        }
        let mut name = vec![0u16; size as usize];
        if GetRawInputDeviceInfoW(
            Some(HANDLE(hdev as _)),
            RIDI_DEVICENAME,
            Some(name.as_mut_ptr() as _),
            &mut size,
        ) == u32::MAX
        {
            return None;
        }
        let text = String::from_utf16_lossy(&name);
        // 确认设备确为精密触摸板后再采纳
        let info = self.fchdevinf(hdev)?;
        if info.0 != PTP_UP || info.1 != PTP_US {
            return None;
        }
        Some(format!("{:x}", md5::compute(text.trim_end_matches('\0'))))
    }

    // 读取原始输入 HID 设备的用途页与用途
    unsafe fn fchdevinf(&self, hdev: isize) -> Option<(u16, u16)> {
        let mut size = 0u32;
        if GetRawInputDeviceInfoW(Some(HANDLE(hdev as _)), RIDI_DEVICEINFO, None, &mut size) != 0 {
            return None;
        }
        let mut info: RID_DEVICE_INFO = std::mem::zeroed();
        info.cbSize = size;
        if GetRawInputDeviceInfoW(
            Some(HANDLE(hdev as _)),
            RIDI_DEVICEINFO,
            Some(&mut info as *mut _ as _),
            &mut size,
        ) == u32::MAX
        {
            return None;
        }
        if info.dwType != RIM_TYPEHID {
            return None;
        }
        let hid = info.Anonymous.hid;
        Some((hid.usUsagePage, hid.usUsage))
    }

    // 最近一次报告来源触摸板的标识
    pub fn curdevid(&self) -> String {
        self.curdev
            .and_then(|h| self.devices.get(&h))
            .map(|d| d.devid.clone())
            .unwrap_or_else(|| "default".to_string())
    }
}

// 持有的 HID 预解析数据缓冲区
struct PrsDt(Vec<u8>);

// 读取 HID 用途上限，并排序使集合 0（触点数）最先处理
unsafe fn qryvalcap(prsdt: &PrsDt) -> Option<Vec<HIDP_VALUE_CAPS>> {
    let pptr = PHIDP_PREPARSED_DATA(prsdt.0.as_ptr() as isize);

    let mut caps = std::mem::zeroed();
    if HidP_GetCaps(pptr, &mut caps) != HIDP_STATUS_SUCCESS {
        return None;
    }

    let mut len = caps.NumberInputValueCaps;
    if len == 0 {
        return None;
    }
    let mut valcaps = vec![std::mem::zeroed::<HIDP_VALUE_CAPS>(); len as usize];
    if HidP_GetValueCaps(HidP_Input, valcaps.as_mut_ptr(), &mut len, pptr) != HIDP_STATUS_SUCCESS {
        return None;
    }
    valcaps.truncate(len as usize);
    valcaps.sort_by_key(|c| c.LinkCollection);
    Some(valcaps)
}

// 累积组装触点所需的 id/x/y 三个字段
#[derive(Default)]
struct CtcMaker {
    cid: Option<i32>,
    cx: Option<i32>,
    cy: Option<i32>,
}

impl CtcMaker {
    // 三个字段齐备后返回触点
    fn build(&self) -> Option<TpCtc> {
        Some(TpCtc {
            id: self.cid?,
            x: self.cx?,
            y: self.cy?,
        })
    }

    // 为下一个触点重置累积器
    fn clear(&mut self) {
        self.cid = None;
        self.cx = None;
        self.cy = None;
    }
}
