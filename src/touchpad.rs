use std::collections::HashMap;
use std::ptr;

use anyhow::Result;
use log::debug;
use windows::Win32::Devices::HumanInterfaceDevice::{
    HidP_GetCaps, HidP_GetUsageValue, HidP_GetValueCaps, HidP_Input, HIDP_REPORT_TYPE,
    HIDP_STATUS_SUCCESS, HIDP_VALUE_CAPS, PHIDP_PREPARSED_DATA,
};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::UI::Input::{
    GetRawInputData, GetRawInputDeviceInfoW, HRAWINPUT, RegisterRawInputDevices, RAWINPUT,
    RAWINPUTDEVICE, RAWINPUTHEADER, RID_DEVICE_INFO, RIDI_DEVICEINFO, RIDI_DEVICENAME,
    RIDI_PREPARSEDDATA, RID_INPUT, RIM_TYPEHID,
};
use windows::Win32::Foundation::HWND;

use crate::drag_engine::{DragEngine, TouchpadContact};

pub const RELEASE_TIMER_ID: usize = 1;

pub struct TouchpadEngine {
    pub engine: DragEngine,
    device_infos: HashMap<isize, TouchpadDeviceInfo>,
    current_device: Option<isize>,
}

#[derive(Debug, Clone)]
pub struct TouchpadDeviceInfo {
    pub device_id: String,
    pub vendor_id: String,
    pub product_id: String,
}

impl TouchpadEngine {
    pub fn new() -> Self {
        Self {
            engine: DragEngine::new(),
            device_infos: HashMap::new(),
            current_device: None,
        }
    }

    pub fn register(hwnd: HWND) -> Result<()> {
        let device = RAWINPUTDEVICE {
            usUsagePage: 0x000D,
            usUsage: 0x0005,
            dwFlags: windows::Win32::UI::Input::RAWINPUTDEVICE_FLAGS(0x00002100),
            hwndTarget: hwnd,
        };
        unsafe {
            let r = RegisterRawInputDevices(&[device], std::mem::size_of::<RAWINPUTDEVICE>() as u32);
            if r.is_err() {
                return Err(anyhow::anyhow!("RegisterRawInputDevices failed: {:?}", r.err()));
            }
        }
        Ok(())
    }

    pub fn on_device_change(&mut self, _hdev: HANDLE) {
        // Re-enumerate devices if needed; raw input registration already captures WM_INPUT.
    }

    pub fn parse_input(&mut self, lparam: isize) -> Option<Vec<TouchpadContact>> {
        unsafe {
            let mut size = 0u32;
            let header_size = std::mem::size_of::<RAWINPUTHEADER>() as u32;
            if GetRawInputData(
                HRAWINPUT(lparam as _),
                RID_INPUT,
                None,
                &mut size,
                header_size,
            ) == u32::MAX
            {
                return None;
            }
            if size == 0 {
                return None;
            }

            let mut buf = vec![0u8; size as usize];
            if GetRawInputData(
                HRAWINPUT(lparam as _),
                RID_INPUT,
                Some(buf.as_mut_ptr() as _),
                &mut size,
                header_size,
            ) != size
            {
                return None;
            }

            let raw = ptr::read(buf.as_ptr() as *const RAWINPUT);
            let hdevice = raw.header.hDevice.0 as isize;
            self.current_device = Some(hdevice);
            if !self.device_infos.contains_key(&hdevice) {
                if let Some(info) = self.fetch_device_info(hdevice) {
                    self.device_infos.insert(hdevice, info);
                }
            }

            let hid = raw.data.hid;
            let total_bytes = (hid.dwSizeHid * hid.dwCount) as usize;
            let hid_data_offset = buf.len() - total_bytes;
            let hid_data = &buf[hid_data_offset..];

            let mut preparsed_size = 0u32;
            if GetRawInputDeviceInfoW(
                Some(raw.header.hDevice),
                RIDI_PREPARSEDDATA,
                None,
                &mut preparsed_size,
            ) != 0
            {
                return None;
            }
            let mut preparsed = vec![0u8; preparsed_size as usize];
            if GetRawInputDeviceInfoW(
                Some(raw.header.hDevice),
                RIDI_PREPARSEDDATA,
                Some(preparsed.as_mut_ptr() as _),
                &mut preparsed_size,
            ) != preparsed_size
            {
                return None;
            }

            let mut caps = std::mem::zeroed();
            if HidP_GetCaps(PHIDP_PREPARSED_DATA(preparsed.as_ptr() as isize), &mut caps)
                != HIDP_STATUS_SUCCESS
            {
                return None;
            }

            let mut value_caps_len = caps.NumberInputValueCaps;
            if value_caps_len == 0 {
                return None;
            }
            let mut value_caps = vec![std::mem::zeroed::<HIDP_VALUE_CAPS>(); value_caps_len as usize];
            if HidP_GetValueCaps(
                HidP_Input,
                value_caps.as_mut_ptr(),
                &mut value_caps_len,
                PHIDP_PREPARSED_DATA(preparsed.as_ptr() as isize),
            ) != HIDP_STATUS_SUCCESS
            {
                return None;
            }
            value_caps.truncate(value_caps_len as usize);

            let mut contact_count = 0u32;
            let mut creators: Vec<TouchpadContactBuilder> = Vec::new();
            let mut contacts: Vec<TouchpadContact> = Vec::new();

            for cap in value_caps.iter() {
                for idx in 0..hid.dwCount {
                    let report_ptr = hid_data.as_ptr().add((hid.dwSizeHid * idx) as usize);
                    let mut value = 0u32;
                    if HidP_GetUsageValue(
                        HidP_Input,
                        cap.UsagePage,
                        Some(cap.LinkCollection),
                        unsafe { cap.Anonymous.NotRange.Usage },
                        &mut value,
                        PHIDP_PREPARSED_DATA(preparsed.as_ptr() as isize),
                        std::slice::from_raw_parts(report_ptr, total_bytes),
                    ) != HIDP_STATUS_SUCCESS
                    {
                        continue;
                    }

                    if cap.LinkCollection == 0 {
                        match (cap.UsagePage, unsafe { cap.Anonymous.NotRange.Usage }) {
                            (0x0D, 0x54) => contact_count = value,
                            _ => {}
                        }
                    } else {
                        while creators.len() <= idx as usize {
                            creators.push(TouchpadContactBuilder::default());
                        }
                        match (cap.UsagePage, unsafe { cap.Anonymous.NotRange.Usage }) {
                            (0x0D, 0x51) => creators[idx as usize].id = Some(value as i32),
                            (0x01, 0x30) => creators[idx as usize].x = Some(value as i32),
                            (0x01, 0x31) => creators[idx as usize].y = Some(value as i32),
                            _ => {}
                        }
                    }
                }

                for creator in creators.iter_mut() {
                    if let Some(c) = creator.build() {
                        if contact_count == 0 || contacts.len() < contact_count as usize {
                            contacts.push(c);
                            creator.clear();
                        }
                    }
                }
                if contact_count != 0 && contacts.len() >= contact_count as usize {
                    break;
                }
            }

            debug!(
                "Parsed contacts: count={} contacts={:?}",
                contact_count, contacts
            );
            Some(contacts)
        }
    }

    fn fetch_device_info(&self, hdev: isize) -> Option<TouchpadDeviceInfo> {
        unsafe {
            let mut size = 0u32;
            if GetRawInputDeviceInfoW(
                Some(HANDLE(hdev as _)),
                RIDI_DEVICEINFO,
                None,
                &mut size,
            ) != 0
            {
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
            if hid.usUsagePage != 0x000D || hid.usUsage != 0x0005 {
                return None;
            }
            let device_id = compute_device_id(hdev).unwrap_or_else(|| "default".to_string());
            Some(TouchpadDeviceInfo {
                device_id,
                vendor_id: hid.dwVendorId.to_string(),
                product_id: hid.dwProductId.to_string(),
            })
        }
    }

    pub fn current_device_id(&self) -> String {
        self.current_device
            .and_then(|h| self.device_infos.get(&h))
            .map(|i| i.device_id.clone())
            .unwrap_or_else(|| "default".to_string())
    }
}

#[derive(Default)]
struct TouchpadContactBuilder {
    id: Option<i32>,
    x: Option<i32>,
    y: Option<i32>,
}

impl TouchpadContactBuilder {
    fn build(&self) -> Option<TouchpadContact> {
        Some(TouchpadContact {
            id: self.id?,
            x: self.x?,
            y: self.y?,
        })
    }

    fn clear(&mut self) {
        self.id = None;
        self.x = None;
        self.y = None;
    }
}

fn compute_device_id(hdev: isize) -> Option<String> {
    unsafe {
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
        let name_str = String::from_utf16_lossy(&name);
        Some(format!("{:x}", md5::compute(name_str.trim_end_matches('\0'))))
    }
}
