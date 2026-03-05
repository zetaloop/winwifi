use std::collections::HashMap;

use crate::wifi::{
    convert::{
        bss_type_to_string, bssid_to_string, channel_from_frequency_khz, interface_state_to_string,
        phy_type_to_string, quality_to_rssi_dbm,
    },
    native::{AvailableNetworkRaw, BssEntryRaw, WifiResult, WlanClient},
    types::{AccessPointRecord, WifiInterfaceSummary, WifiSnapshot},
};

#[derive(Debug)]
pub struct WifiPoller {
    client: WlanClient,
    tick_count: u64,
}

impl WifiPoller {
    pub fn new() -> WifiResult<Self> {
        Ok(Self {
            client: WlanClient::new()?,
            tick_count: 0,
        })
    }

    pub fn collect(&mut self, selected_interface: Option<usize>) -> WifiResult<WifiSnapshot> {
        self.tick_count = self.tick_count.saturating_add(1);
        let interfaces = self.client.list_interfaces()?;
        if interfaces.is_empty() {
            return Ok(WifiSnapshot {
                interfaces: Vec::new(),
                active_interface: 0,
                aps: Vec::new(),
                permission_denied: false,
                status: format!(
                    "WLAN v{}，未发现可用无线网卡",
                    self.client.negotiated_version
                ),
            });
        }

        let active_interface = selected_interface
            .unwrap_or(0)
            .min(interfaces.len().saturating_sub(1));
        let selected_guid = interfaces[active_interface].guid;

        let mut permission_denied = false;
        if self.tick_count.is_multiple_of(2)
            && let Err(err) = self.client.trigger_scan(&selected_guid)
        {
            if err.is_access_denied() {
                permission_denied = true;
            } else {
                return Err(err);
            }
        }

        let available = match self.client.get_available_networks(&selected_guid) {
            Ok(v) => v,
            Err(err) if err.is_access_denied() => {
                permission_denied = true;
                Vec::new()
            }
            Err(err) => return Err(err),
        };
        let bss_entries = match self.client.get_bss_entries(&selected_guid) {
            Ok(v) => v,
            Err(err) if err.is_access_denied() => {
                permission_denied = true;
                Vec::new()
            }
            Err(err) => return Err(err),
        };
        let bss_entries = dedupe_bss_entries(bss_entries);
        let connection = match self.client.get_current_connection(&selected_guid) {
            Ok(v) => v,
            Err(err) if err.is_access_denied() => {
                permission_denied = true;
                None
            }
            Err(err) => return Err(err),
        };

        let available_map = build_available_map(&available);
        let mut aps = bss_entries
            .into_iter()
            .map(|bss| {
                let key = (bss.ssid.clone(), bss.bss_type);
                let available_hint = available_map.get(&key);
                let phy = available_hint.map_or(bss.phy_type, |v| v.phy_type);
                let signal_quality = available_hint
                    .map_or(bss.link_quality, |v| bss.link_quality.max(v.signal_quality));
                let connected = connection
                    .as_ref()
                    .is_some_and(|conn| conn.bssid == bss.bssid);
                let (rx_rate_mbps, tx_rate_mbps) = if connected {
                    connection.as_ref().map_or((None, None), |conn| {
                        (Some(conn.rx_rate_mbps), Some(conn.tx_rate_mbps))
                    })
                } else {
                    (None, None)
                };
                AccessPointRecord {
                    bssid: bss.bssid,
                    bssid_text: bssid_to_string(bss.bssid),
                    ssid: bss.ssid,
                    mode: format!(
                        "{}/{}",
                        bss_type_to_string(bss.bss_type),
                        phy_type_to_string(phy)
                    ),
                    channel: channel_from_frequency_khz(bss.center_freq_khz),
                    rate_mbps: bss.max_rate_mbps,
                    signal_quality,
                    rssi_dbm: bss.rssi,
                    center_freq_khz: bss.center_freq_khz,
                    connected,
                    rx_rate_mbps,
                    tx_rate_mbps,
                }
            })
            .collect::<Vec<_>>();
        aps.sort_by(|a, b| {
            b.signal_quality
                .cmp(&a.signal_quality)
                .then_with(|| b.rssi_dbm.cmp(&a.rssi_dbm))
                .then_with(|| a.bssid_text.cmp(&b.bssid_text))
        });

        let status = if permission_denied {
            "定位权限未允许，部分 WiFi API 被拒绝，请在系统设置中开启位置权限".to_string()
        } else if aps.is_empty() {
            if let Some(connection) = connection {
                format!(
                    "已连接 {} [{} / {}]，信号 {}% (~{} dBm)",
                    connection.ssid,
                    bss_type_to_string(connection.bss_type),
                    phy_type_to_string(connection.phy_type),
                    connection.signal_quality,
                    quality_to_rssi_dbm(connection.signal_quality)
                )
            } else {
                "扫描完成，未发现 BSS 条目".to_string()
            }
        } else {
            format!(
                "{} | AP {} 个 | 接口 {}",
                interfaces[active_interface].description,
                aps.len(),
                interface_state_to_string(interfaces[active_interface].state)
            )
        };

        Ok(WifiSnapshot {
            interfaces: interfaces
                .into_iter()
                .map(|interface| WifiInterfaceSummary {
                    description: interface.description,
                    state: interface_state_to_string(interface.state).to_string(),
                })
                .collect(),
            active_interface,
            aps,
            permission_denied,
            status,
        })
    }
}

#[derive(Debug)]
struct AvailableHint {
    phy_type: i32,
    signal_quality: u32,
}

fn build_available_map(networks: &[AvailableNetworkRaw]) -> HashMap<(String, i32), AvailableHint> {
    let mut map = HashMap::with_capacity(networks.len());
    for item in networks {
        let key = (item.ssid.clone(), item.bss_type);
        map.entry(key)
            .and_modify(|v: &mut AvailableHint| {
                if item.connected {
                    v.phy_type = item.phy_type;
                }
                v.signal_quality = v.signal_quality.max(item.signal_quality);
            })
            .or_insert(AvailableHint {
                phy_type: item.phy_type,
                signal_quality: item.signal_quality,
            });
    }
    map
}

fn dedupe_bss_entries(entries: Vec<BssEntryRaw>) -> Vec<BssEntryRaw> {
    let mut map: HashMap<([u8; 6], String, i32, i32, u32), BssEntryRaw> =
        HashMap::with_capacity(entries.len());
    for entry in entries {
        let key = (
            entry.bssid,
            entry.ssid.clone(),
            entry.bss_type,
            entry.phy_type,
            entry.center_freq_khz,
        );
        map.entry(key)
            .and_modify(|exist| {
                if is_better_bss_entry(&entry, exist) {
                    *exist = entry.clone();
                }
            })
            .or_insert(entry);
    }
    map.into_values().collect()
}

fn is_better_bss_entry(candidate: &BssEntryRaw, current: &BssEntryRaw) -> bool {
    candidate
        .link_quality
        .cmp(&current.link_quality)
        .then_with(|| candidate.rssi.cmp(&current.rssi))
        .then_with(|| candidate.max_rate_mbps.total_cmp(&current.max_rate_mbps))
        .is_gt()
}
