use std::collections::HashMap;
use std::ptr;

use anyhow::Result;
use log::debug;
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
    #[allow(dead_code)]
    pub vendor_id: String,
    #[allow(dead_code)]
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
            RegisterRawInputDevices(&[device], std::mem::size_of::<RAWINPUTDEVICE>() as u32)
                .map_err(|e| anyhow::anyhow!("RegisterRawInputDevices failed: {e:?}"))?;
        }
        Ok(())
    }

    pub fn on_device_change(&mut self, _hdev: HANDLE) {
        // Re-enumerate devices if needed; raw input registration already captures WM_INPUT.
    }

    pub fn parse_input(&mut self, lparam: isize) -> Option<Vec<TouchpadContact>> {
        unsafe {
            let raw_input = self.read_raw_input(lparam)?;
            let hdevice = raw_input.header.hDevice.0 as isize;
            self.current_device = Some(hdevice);

            if let Some(info) = self.fetch_device_info(hdevice) {
                self.device_infos.insert(hdevice, info);
            }

            let preparsed = self.fetch_preparse_data(raw_input.header.hDevice)?;
            let contacts = self.parse_contacts(&raw_input, &preparsed)?;

            debug!("Parsed contacts: {:?}", contacts);
            Some(contacts)
        }
    }

    unsafe fn read_raw_input(&self, lparam: isize) -> Option<RAWINPUT> {
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

        Some(ptr::read(buf.as_ptr() as *const RAWINPUT))
    }

    unsafe fn fetch_preparse_data(&self, hdevice: HANDLE) -> Option<PreparsedData> {
        let mut size = 0u32;
        if GetRawInputDeviceInfoW(Some(hdevice), RIDI_PREPARSEDDATA, None, &mut size) != 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        if GetRawInputDeviceInfoW(
            Some(hdevice),
            RIDI_PREPARSEDDATA,
            Some(buf.as_mut_ptr() as _),
            &mut size,
        ) != size
        {
            return None;
        }
        Some(PreparsedData(buf))
    }

    unsafe fn parse_contacts(
        &self,
        raw: &RAWINPUT,
        preparsed: &PreparsedData,
    ) -> Option<Vec<TouchpadContact>> {
        let preparsed_ptr = PHIDP_PREPARSED_DATA(preparsed.0.as_ptr() as isize);

        let mut caps = std::mem::zeroed();
        if HidP_GetCaps(preparsed_ptr, &mut caps) != HIDP_STATUS_SUCCESS {
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
            preparsed_ptr,
        ) != HIDP_STATUS_SUCCESS
        {
            return None;
        }
        value_caps.truncate(value_caps_len as usize);

        let hid = raw.data.hid;
        let total_bytes = (hid.dwSizeHid * hid.dwCount) as usize;
        let raw_data = self.raw_hid_data(raw, total_bytes)?;

        let mut contact_count = 0u32;
        let mut creators: Vec<TouchpadContactBuilder> = Vec::new();
        let mut contacts: Vec<TouchpadContact> = Vec::new();

        for cap in &value_caps {
            let usage = unsafe { cap.Anonymous.NotRange.Usage };
            for idx in 0..hid.dwCount {
                let report = &raw_data[(hid.dwSizeHid * idx) as usize..][..total_bytes];
                let mut value = 0u32;
                if HidP_GetUsageValue(
                    HidP_Input,
                    cap.UsagePage,
                    Some(cap.LinkCollection),
                    usage,
                    &mut value,
                    preparsed_ptr,
                    report,
                ) != HIDP_STATUS_SUCCESS
                {
                    continue;
                }

                if cap.LinkCollection == 0 {
                    if cap.UsagePage == 0x0D && usage == 0x54 {
                        contact_count = value;
                    }
                } else {
                    while creators.len() <= idx as usize {
                        creators.push(TouchpadContactBuilder::default());
                    }
                    match (cap.UsagePage, usage) {
                        (0x0D, 0x51) => creators[idx as usize].id = Some(value as i32),
                        (0x01, 0x30) => creators[idx as usize].x = Some(value as i32),
                        (0x01, 0x31) => creators[idx as usize].y = Some(value as i32),
                        _ => {}
                    }
                }
            }

            for creator in &mut creators {
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

        Some(contacts)
    }

    unsafe fn raw_hid_data(&self, raw: &RAWINPUT, total_bytes: usize) -> Option<Vec<u8>> {
        let raw_size = (raw.data.hid.dwSizeHid * raw.data.hid.dwCount) as usize;
        let buf_size = std::mem::size_of::<RAWINPUTHEADER>() + raw_size;
        let mut buf = vec![0u8; buf_size];
        let header_size = std::mem::size_of::<RAWINPUTHEADER>() as u32;
        if GetRawInputData(
            HRAWINPUT(raw.header.hDevice.0 as _),
            RID_INPUT,
            Some(buf.as_mut_ptr() as _),
            &mut (buf_size as u32),
            header_size,
        ) != buf_size as u32
        {
            return None;
        }
        Some(buf[buf_size - total_bytes..].to_vec())
    }

    fn fetch_device_info(&self, hdev: isize) -> Option<TouchpadDeviceInfo> {
        unsafe {
            let mut size = 0u32;
            if GetRawInputDeviceInfoW(Some(HANDLE(hdev as _)), RIDI_DEVICEINFO, None, &mut size)
                != 0
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

struct PreparsedData(Vec<u8>);

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
        Some(format!(
            "{:x}",
            md5::compute(name_str.trim_end_matches('\0'))
        ))
    }
}
