use windows_sys::Win32::NetworkManagement::WiFi::{
    DOT11_BSS_TYPE, DOT11_PHY_TYPE, WLAN_RATE_SET, dot11_BSS_type_independent,
    dot11_BSS_type_infrastructure, dot11_phy_type_dmg, dot11_phy_type_dsss, dot11_phy_type_eht,
    dot11_phy_type_erp, dot11_phy_type_fhss, dot11_phy_type_he, dot11_phy_type_hrdsss,
    dot11_phy_type_ht, dot11_phy_type_irbaseband, dot11_phy_type_ofdm, dot11_phy_type_vht,
};

pub fn ssid_to_string(ssid_len: u32, ssid: &[u8; 32]) -> String {
    let len = ssid_len.min(32) as usize;
    if len == 0 {
        return "<hidden>".to_string();
    }
    let raw = &ssid[..len];
    match std::str::from_utf8(raw) {
        Ok(text) if text.chars().all(|c| !c.is_control()) => text.to_owned(),
        _ => raw
            .iter()
            .map(|v| format!("{v:02X}"))
            .collect::<Vec<_>>()
            .join(""),
    }
}

pub fn bssid_to_string(bssid: [u8; 6]) -> String {
    bssid
        .iter()
        .map(|v| format!("{v:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

pub fn bss_type_to_string(v: DOT11_BSS_TYPE) -> &'static str {
    if v == dot11_BSS_type_infrastructure {
        "Infrastructure"
    } else if v == dot11_BSS_type_independent {
        "Ad-Hoc"
    } else {
        "Unknown"
    }
}

pub fn phy_type_to_string(v: DOT11_PHY_TYPE) -> String {
    if v == dot11_phy_type_fhss {
        "FHSS".to_string()
    } else if v == dot11_phy_type_dsss {
        "DSSS".to_string()
    } else if v == dot11_phy_type_irbaseband {
        "IR".to_string()
    } else if v == dot11_phy_type_ofdm {
        "OFDM".to_string()
    } else if v == dot11_phy_type_hrdsss {
        "HRDSSS".to_string()
    } else if v == dot11_phy_type_erp {
        "ERP".to_string()
    } else if v == dot11_phy_type_ht {
        "HT".to_string()
    } else if v == dot11_phy_type_vht {
        "VHT".to_string()
    } else if v == dot11_phy_type_dmg {
        "DMG".to_string()
    } else if v == dot11_phy_type_he {
        "HE".to_string()
    } else if v == dot11_phy_type_eht {
        "EHT".to_string()
    } else {
        format!("PHY({v})")
    }
}

pub fn channel_from_frequency_khz(freq_khz: u32) -> u16 {
    if freq_khz == 0 {
        return 0;
    }
    let freq_mhz = freq_khz / 1000;
    if (2412..=2472).contains(&freq_mhz) {
        return ((freq_mhz - 2407) / 5) as u16;
    }
    if freq_mhz == 2484 {
        return 14;
    }
    if (5000..=5895).contains(&freq_mhz) {
        return ((freq_mhz - 5000) / 5) as u16;
    }
    if (5955..=7115).contains(&freq_mhz) {
        return ((freq_mhz - 5950) / 5) as u16;
    }
    0
}

pub fn quality_to_rssi_dbm(quality: u32) -> i32 {
    let q = quality.clamp(0, 100) as f32;
    (-100.0 + q * 0.5).round() as i32
}

pub fn max_rate_mbps(rate_set: &WLAN_RATE_SET) -> f32 {
    let count = (rate_set.uRateSetLength as usize / 2).min(rate_set.usRateSet.len());
    if count == 0 {
        return 0.0;
    }
    rate_set.usRateSet[..count]
        .iter()
        .map(|v| (v & 0x7FFF) as f32 * 0.5)
        .fold(0.0, f32::max)
}

pub fn interface_state_to_string(state: i32) -> &'static str {
    match state {
        0 => "NotReady",
        1 => "Connected",
        2 => "AdHocNetworkFormed",
        3 => "Disconnecting",
        4 => "Disconnected",
        5 => "Associating",
        6 => "Discovering",
        7 => "Authenticating",
        _ => "Unknown",
    }
}
