#[derive(Clone, Debug)]
pub struct WifiInterfaceSummary {
    pub description: String,
    pub state: String,
}

#[derive(Clone, Debug)]
pub struct AccessPointRecord {
    pub bssid: [u8; 6],
    pub bssid_text: String,
    pub ssid: String,
    pub mode: String,
    pub channel: u16,
    pub rate_mbps: f32,
    pub signal_quality: u32,
    pub rssi_dbm: i32,
    pub center_freq_khz: u32,
    pub connected: bool,
    pub rx_rate_mbps: Option<f32>,
    pub tx_rate_mbps: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct WifiSnapshot {
    pub interfaces: Vec<WifiInterfaceSummary>,
    pub active_interface: usize,
    pub aps: Vec<AccessPointRecord>,
    pub permission_denied: bool,
    pub status: String,
}
