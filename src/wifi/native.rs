use std::{ffi::c_void, ptr::addr_of, ptr::null, ptr::null_mut};

use thiserror::Error;
use windows_sys::{
    Win32::{
        Foundation::{ERROR_ACCESS_DENIED, ERROR_INVALID_STATE, HANDLE},
        NetworkManagement::WiFi::*,
    },
    core::GUID,
};

use crate::wifi::convert::{max_rate_mbps, ssid_to_string};

#[derive(Clone)]
pub struct InterfaceInfoRaw {
    pub guid: GUID,
    pub description: String,
    pub state: WLAN_INTERFACE_STATE,
}

#[derive(Debug, Clone)]
pub struct BssEntryRaw {
    pub ssid: String,
    pub bssid: [u8; 6],
    pub bss_type: DOT11_BSS_TYPE,
    pub phy_type: DOT11_PHY_TYPE,
    pub rssi: i32,
    pub link_quality: u32,
    pub center_freq_khz: u32,
    pub max_rate_mbps: f32,
}

#[derive(Debug, Clone)]
pub struct AvailableNetworkRaw {
    pub ssid: String,
    pub bss_type: DOT11_BSS_TYPE,
    pub phy_type: DOT11_PHY_TYPE,
    pub signal_quality: u32,
    pub connected: bool,
}

#[derive(Debug, Clone)]
pub struct CurrentConnectionRaw {
    pub ssid: String,
    pub bssid: [u8; 6],
    pub bss_type: DOT11_BSS_TYPE,
    pub phy_type: DOT11_PHY_TYPE,
    pub signal_quality: u32,
    pub rx_rate_mbps: f32,
    pub tx_rate_mbps: f32,
}

#[derive(Debug, Error, Clone)]
pub enum WifiError {
    #[error("{operation} failed with Win32 error code {code}")]
    Win32 { operation: &'static str, code: u32 },
    #[error("{operation} returned a null pointer")]
    NullPointer { operation: &'static str },
}

impl WifiError {
    pub fn is_access_denied(&self) -> bool {
        matches!(self, Self::Win32 { code, .. } if *code == ERROR_ACCESS_DENIED)
    }
}

pub type WifiResult<T> = std::result::Result<T, WifiError>;

fn expect_success(operation: &'static str, code: u32) -> WifiResult<()> {
    if code == 0 {
        return Ok(());
    }
    Err(WifiError::Win32 { operation, code })
}

fn utf16_z_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|v| *v == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

struct WlanMemory {
    ptr: *mut c_void,
}

impl WlanMemory {
    fn new<T>(ptr: *mut T) -> Self {
        Self { ptr: ptr.cast() }
    }
}

impl Drop for WlanMemory {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { WlanFreeMemory(self.ptr) };
        }
    }
}

#[derive(Debug)]
pub struct WlanClient {
    handle: HANDLE,
    pub negotiated_version: u32,
}

impl WlanClient {
    pub fn new() -> WifiResult<Self> {
        let mut negotiated_version = 0;
        let mut handle: HANDLE = null_mut();
        let code = unsafe { WlanOpenHandle(2, null(), &mut negotiated_version, &mut handle) };
        expect_success("WlanOpenHandle", code)?;
        Ok(Self {
            handle,
            negotiated_version,
        })
    }

    pub fn list_interfaces(&self) -> WifiResult<Vec<InterfaceInfoRaw>> {
        let mut list_ptr: *mut WLAN_INTERFACE_INFO_LIST = null_mut();
        let code = unsafe { WlanEnumInterfaces(self.handle, null(), &mut list_ptr) };
        expect_success("WlanEnumInterfaces", code)?;
        if list_ptr.is_null() {
            return Err(WifiError::NullPointer {
                operation: "WlanEnumInterfaces",
            });
        }
        let _mem = WlanMemory::new(list_ptr);
        let list = unsafe { &*list_ptr };
        let items = unsafe {
            let first = addr_of!((*list_ptr).InterfaceInfo).cast::<WLAN_INTERFACE_INFO>();
            std::slice::from_raw_parts(first, list.dwNumberOfItems as usize)
        };
        let mut result = Vec::with_capacity(items.len());
        for item in items {
            result.push(InterfaceInfoRaw {
                guid: item.InterfaceGuid,
                description: utf16_z_to_string(&item.strInterfaceDescription),
                state: item.isState,
            });
        }
        Ok(result)
    }

    pub fn trigger_scan(&self, interface_guid: &GUID) -> WifiResult<()> {
        let code = unsafe { WlanScan(self.handle, interface_guid, null(), null(), null()) };
        expect_success("WlanScan", code)
    }

    pub fn get_available_networks(
        &self,
        interface_guid: &GUID,
    ) -> WifiResult<Vec<AvailableNetworkRaw>> {
        let mut list_ptr: *mut WLAN_AVAILABLE_NETWORK_LIST = null_mut();
        let code = unsafe {
            WlanGetAvailableNetworkList(self.handle, interface_guid, 0, null(), &mut list_ptr)
        };
        expect_success("WlanGetAvailableNetworkList", code)?;
        if list_ptr.is_null() {
            return Err(WifiError::NullPointer {
                operation: "WlanGetAvailableNetworkList",
            });
        }
        let _mem = WlanMemory::new(list_ptr);
        let list = unsafe { &*list_ptr };
        let items = unsafe {
            let first = addr_of!((*list_ptr).Network).cast::<WLAN_AVAILABLE_NETWORK>();
            std::slice::from_raw_parts(first, list.dwNumberOfItems as usize)
        };
        let mut result = Vec::with_capacity(items.len());
        for item in items {
            let phy = if item.uNumberOfPhyTypes > 0 {
                item.dot11PhyTypes[0]
            } else {
                dot11_phy_type_unknown
            };
            result.push(AvailableNetworkRaw {
                ssid: ssid_to_string(item.dot11Ssid.uSSIDLength, &item.dot11Ssid.ucSSID),
                bss_type: item.dot11BssType,
                phy_type: phy,
                signal_quality: item.wlanSignalQuality,
                connected: (item.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED) != 0,
            });
        }
        Ok(result)
    }

    pub fn get_bss_entries(&self, interface_guid: &GUID) -> WifiResult<Vec<BssEntryRaw>> {
        let mut all = Vec::new();
        all.extend(self.get_bss_entries_by_type(interface_guid, dot11_BSS_type_infrastructure)?);
        all.extend(self.get_bss_entries_by_type(interface_guid, dot11_BSS_type_independent)?);
        Ok(all)
    }

    fn get_bss_entries_by_type(
        &self,
        interface_guid: &GUID,
        bss_type: DOT11_BSS_TYPE,
    ) -> WifiResult<Vec<BssEntryRaw>> {
        let mut list_ptr: *mut WLAN_BSS_LIST = null_mut();
        let code = unsafe {
            WlanGetNetworkBssList(
                self.handle,
                interface_guid,
                null(),
                bss_type,
                false.into(),
                null(),
                &mut list_ptr,
            )
        };
        expect_success("WlanGetNetworkBssList", code)?;
        if list_ptr.is_null() {
            return Err(WifiError::NullPointer {
                operation: "WlanGetNetworkBssList",
            });
        }
        let _mem = WlanMemory::new(list_ptr);
        let list = unsafe { &*list_ptr };
        let items = unsafe {
            let first = addr_of!((*list_ptr).wlanBssEntries).cast::<WLAN_BSS_ENTRY>();
            std::slice::from_raw_parts(first, list.dwNumberOfItems as usize)
        };
        let mut result = Vec::with_capacity(items.len());
        for item in items {
            result.push(BssEntryRaw {
                ssid: ssid_to_string(item.dot11Ssid.uSSIDLength, &item.dot11Ssid.ucSSID),
                bssid: item.dot11Bssid,
                bss_type: item.dot11BssType,
                phy_type: item.dot11BssPhyType,
                rssi: item.lRssi,
                link_quality: item.uLinkQuality,
                center_freq_khz: item.ulChCenterFrequency,
                max_rate_mbps: max_rate_mbps(&item.wlanRateSet),
            });
        }
        Ok(result)
    }

    pub fn get_current_connection(
        &self,
        interface_guid: &GUID,
    ) -> WifiResult<Option<CurrentConnectionRaw>> {
        let mut data_size = 0;
        let mut data_ptr: *mut c_void = null_mut();
        let mut op_code_type = 0;
        let code = unsafe {
            WlanQueryInterface(
                self.handle,
                interface_guid,
                wlan_intf_opcode_current_connection,
                null(),
                &mut data_size,
                &mut data_ptr,
                &mut op_code_type,
            )
        };
        if code == ERROR_INVALID_STATE {
            return Ok(None);
        }
        expect_success("WlanQueryInterface", code)?;
        if data_ptr.is_null() {
            return Err(WifiError::NullPointer {
                operation: "WlanQueryInterface",
            });
        }
        let _mem = WlanMemory::new(data_ptr);
        let attrs = unsafe { &*(data_ptr.cast::<WLAN_CONNECTION_ATTRIBUTES>()) };
        if attrs.isState != wlan_interface_state_connected {
            return Ok(None);
        }
        let assoc = attrs.wlanAssociationAttributes;
        Ok(Some(CurrentConnectionRaw {
            ssid: ssid_to_string(assoc.dot11Ssid.uSSIDLength, &assoc.dot11Ssid.ucSSID),
            bssid: assoc.dot11Bssid,
            bss_type: assoc.dot11BssType,
            phy_type: assoc.dot11PhyType,
            signal_quality: assoc.wlanSignalQuality,
            rx_rate_mbps: assoc.ulRxRate as f32 / 1_000_000.0,
            tx_rate_mbps: assoc.ulTxRate as f32 / 1_000_000.0,
        }))
    }
}

impl Drop for WlanClient {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                let _ = WlanCloseHandle(self.handle, null());
            }
        }
    }
}
