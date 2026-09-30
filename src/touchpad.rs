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

/// Timer id used to end a drag after the release delay.
pub const RLS_TMR_ID: usize = 1;

/// HID usage page and usage identifying a Windows Precision Touchpad.
const PTP_UP: u16 = 0x000D;
const PTP_US: u16 = 0x0005;

/// Raw input flags: receive input while unfocused and get device notifications.
const RIDEV_FLAGS: u32 = 0x0000_2100;

/// Owns the raw-input registration and device caches, and feeds the drag engine.
pub struct TchpdEng {
    pub engine: DrgEng,
    devices: HashMap<isize, DevCaps>,
    curdev: Option<isize>,
}

/// Per-device data cached once, since preparsed data and value caps never
/// change while a device stays connected.
struct DevCaps {
    devid: String,
    prsdt: PrsDt,
    valcaps: Vec<HIDP_VALUE_CAPS>,
}

impl TchpdEng {
    /// Creates an engine with empty caches.
    pub fn new() -> Self {
        Self {
            engine: DrgEng::new(),
            devices: HashMap::new(),
            curdev: None,
        }
    }

    /// Subscribes the window to precision touchpad input reports.
    pub fn register(hwnd: HWND) -> Result<()> {
        let device = RAWINPUTDEVICE {
            usUsagePage: PTP_UP,
            usUsage: PTP_US,
            dwFlags: windows::Win32::UI::Input::RAWINPUTDEVICE_FLAGS(RIDEV_FLAGS),
            hwndTarget: hwnd,
        };
        unsafe {
            RegisterRawInputDevices(&[device], std::mem::size_of::<RAWINPUTDEVICE>() as u32)
                .map_err(|e| anyhow::anyhow!("RegisterRawInputDevices failed: {e:?}"))?;
        }
        Ok(())
    }

    /// Invalidates cached device data after a device add or removal.
    pub fn ondevchg(&mut self, _hdev: HANDLE) {
        self.devices.clear();
        self.curdev = None;
    }

    /// Parses a WM_INPUT payload into the contact list, or None on failure.
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

            // The HID payload sits at the tail of the raw input buffer.
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

    /// Copies the full raw input payload for the given WM_INPUT handle.
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

    /// Resolves device identity and value caps, cached for later reports.
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

    /// Reads the device's preparsed HID data.
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

    /// Rebuilds one contact per HID report from the decoded value caps.
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

    /// Hashes the raw device name into a stable identifier for config keys.
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
        // Confirm the device really is a precision touchpad before trusting it.
        let info = self.fchdevinf(hdev)?;
        if info.0 != PTP_UP || info.1 != PTP_US {
            return None;
        }
        Some(format!("{:x}", md5::compute(text.trim_end_matches('\0'))))
    }

    /// Reads the usage page and usage of a raw input HID device.
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

    /// Identifier of the touchpad that produced the most recent report.
    pub fn curdevid(&self) -> String {
        self.curdev
            .and_then(|h| self.devices.get(&h))
            .map(|d| d.devid.clone())
            .unwrap_or_else(|| "default".to_string())
    }
}

/// Owned preparsed HID data buffer.
struct PrsDt(Vec<u8>);

/// HID value caps sorted so collection 0 (contact count) is read first.
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

/// Accumulates the id/x/y fields a contact is assembled from.
#[derive(Default)]
struct CtcMaker {
    cid: Option<i32>,
    cx: Option<i32>,
    cy: Option<i32>,
}

impl CtcMaker {
    /// Returns the contact once all three fields have been filled.
    fn build(&self) -> Option<TpCtc> {
        Some(TpCtc {
            id: self.cid?,
            x: self.cx?,
            y: self.cy?,
        })
    }

    /// Resets the accumulator for the next contact.
    fn clear(&mut self) {
        self.cid = None;
        self.cx = None;
        self.cy = None;
    }
}
