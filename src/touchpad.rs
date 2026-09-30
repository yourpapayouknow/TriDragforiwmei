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

use crate::drag_engine::{DragEngine, TouchpadContact};

pub const RELEASE_TIMER_ID: usize = 1;

pub struct TouchpadEngine {
    pub engine: DragEngine,
    devices: HashMap<isize, TouchpadDevice>,
    current_device: Option<isize>,
}

/// Per-device cached data. Preparsed data and value caps never change while a
/// device stays connected, so they are computed once and reused for every
/// WM_INPUT instead of being re-queried on each report.
struct TouchpadDevice {
    info: TouchpadDeviceInfo,
    preparsed: PreparsedData,
    value_caps: Vec<HIDP_VALUE_CAPS>,
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
            devices: HashMap::new(),
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
        // A device was added or removed: drop cached caps so the next report
        // re-queries them against the current device set.
        self.devices.clear();
        self.current_device = None;
    }

    pub fn parse_input(&mut self, lparam: isize) -> Option<Vec<TouchpadContact>> {
        unsafe {
            let buf = self.read_raw_input(lparam)?;
            let raw = ptr::read(buf.as_ptr() as *const RAWINPUT);

            let hdevice = raw.header.hDevice.0 as isize;
            self.current_device = Some(hdevice);

            if !self.devices.contains_key(&hdevice) {
                self.cache_device(hdevice, raw.header.hDevice)?;
            }

            let hid = raw.data.hid;
            let hid_len = (hid.dwSizeHid * hid.dwCount) as usize;
            let hid_offset = buf.len() - hid_len;
            let hid_data = &buf[hid_offset..];

            let device = self.devices.get(&hdevice)?;
            let contacts = self.parse_contacts(
                hid_data,
                hid.dwSizeHid,
                hid.dwCount,
                &device.preparsed,
                &device.value_caps,
            )?;

            Some(contacts)
        }
    }

    /// Fetches device info, preparsed data and value caps once per device.
    unsafe fn cache_device(&mut self, hdevice: isize, handle: HANDLE) -> Option<()> {
        let info = self.fetch_device_info(hdevice)?;
        let preparsed = self.fetch_preparse_data(handle)?;
        let value_caps = query_value_caps(&preparsed)?;
        self.devices.insert(
            hdevice,
            TouchpadDevice {
                info,
                preparsed,
                value_caps,
            },
        );
        Some(())
    }

    unsafe fn read_raw_input(&self, lparam: isize) -> Option<Vec<u8>> {
        let mut size = 0u32;
        let header_size = std::mem::size_of::<RAWINPUTHEADER>() as u32;
        if GetRawInputData(
            HRAWINPUT(lparam as _),
            RID_INPUT,
            None,
            &mut size,
            header_size,
        ) != 0
        {
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
            header_size,
        ) != size
        {
            return None;
        }

        Some(buf)
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
        raw_data: &[u8],
        hid_size: u32,
        hid_count: u32,
        preparsed: &PreparsedData,
        value_caps: &[HIDP_VALUE_CAPS],
    ) -> Option<Vec<TouchpadContact>> {
        let preparsed_ptr = PHIDP_PREPARSED_DATA(preparsed.0.as_ptr() as isize);

        let mut contact_count = 0u32;
        let mut creators: Vec<TouchpadContactBuilder> = Vec::new();
        let mut contacts: Vec<TouchpadContact> = Vec::new();

        for cap in value_caps {
            let usage = unsafe { cap.Anonymous.NotRange.Usage };

            // Each HID report is `hid_size` bytes; a raw input may carry several.
            // Mirror the reference: pass the whole buffer as the report and let
            // HidP_GetUsageValue read at the per-contact offset.
            for contact_index in 0..hid_count {
                let offset = (hid_size * contact_index) as usize;
                if offset >= raw_data.len() {
                    continue;
                }
                let report = &raw_data[offset..];

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
                    while creators.len() <= contact_index as usize {
                        creators.push(TouchpadContactBuilder::default());
                    }
                    match (cap.UsagePage, usage) {
                        (0x0D, 0x51) => creators[contact_index as usize].id = Some(value as i32),
                        (0x01, 0x30) => creators[contact_index as usize].x = Some(value as i32),
                        (0x01, 0x31) => creators[contact_index as usize].y = Some(value as i32),
                        _ => {}
                    }
                }
            }

            // Collect any contact that now has id/x/y filled.
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
            .and_then(|h| self.devices.get(&h))
            .map(|d| d.info.device_id.clone())
            .unwrap_or_else(|| "default".to_string())
    }
}

/// Reads the device's input value caps, sorted so collection 0 (contact count)
/// is processed before the per-contact collections.
unsafe fn query_value_caps(preparsed: &PreparsedData) -> Option<Vec<HIDP_VALUE_CAPS>> {
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
    value_caps.sort_by_key(|c| c.LinkCollection);

    Some(value_caps)
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
