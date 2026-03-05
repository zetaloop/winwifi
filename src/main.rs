#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod error;
mod wifi;

use std::{
    collections::{HashMap, VecDeque},
    io::Write as _,
    process::Command,
    time::Duration,
};

use compio::{runtime::spawn, time::interval};
use error::{AppError, AppResult};
use plotters::prelude::{
    BLACK, ChartBuilder, Color as PlottersColor, IntoDrawingArea, IntoFont, LineSeries, RGBColor,
    WHITE,
};
use wifi::{
    convert::bssid_to_string,
    poller::WifiPoller,
    types::{AccessPointRecord, WifiSnapshot},
};
use winio::prelude::*;

const HISTORY_CAPACITY: usize = 180;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Signal,
    Channel,
    Rate,
    Mode,
    Ssid,
    Bssid,
}

const HEADER_COLUMNS: [(SortKey, f64); 6] = [
    (SortKey::Signal, 0.17),
    (SortKey::Channel, 0.09),
    (SortKey::Rate, 0.11),
    (SortKey::Mode, 0.18),
    (SortKey::Ssid, 0.24),
    (SortKey::Bssid, 0.21),
];

fn run() -> AppResult<()> {
    App::new("dev.foxloop.winwifi")?.run_until_event::<MainModel>(())
}

fn main() {
    if let Err(err) = run() {
        let _ = writeln!(std::io::stderr(), "{err}");
        std::process::exit(1);
    }
}

struct MainModel {
    window: Child<Window>,
    interface_combo: Child<ComboBox>,
    location_button: Child<Button>,
    status_label: Child<Label>,
    header_signal: Child<Button>,
    header_chan: Child<Button>,
    header_rate: Child<Button>,
    header_mode: Child<Button>,
    header_ssid: Child<Button>,
    header_bssid: Child<Button>,
    ap_list: Child<ListBox>,
    detail_box: Child<TextBox>,
    chart: Child<Canvas>,
    poller: WifiPoller,
    interface_items: Vec<String>,
    selected_interface: Option<usize>,
    aps: Vec<AccessPointRecord>,
    selected_bssid: Option<[u8; 6]>,
    histories: HashMap<String, VecDeque<i32>>,
    permission_denied: bool,
    sort_key: SortKey,
    sort_desc: bool,
}

#[derive(Debug)]
enum MainMessage {
    Noop,
    Close,
    Redraw,
    Tick,
    InterfaceSelected,
    AccessPointSelected,
    SortBy(SortKey),
    OpenLocationSettings,
}

impl Component for MainModel {
    type Error = AppError;
    type Event = ();
    type Init<'a> = ();
    type Message = MainMessage;

    async fn init(_init: Self::Init<'_>, sender: &ComponentSender<Self>) -> AppResult<Self> {
        let poller = WifiPoller::new()?;

        init! {
            window: Window = (()) => {
                text: "WinWiFi",
                size: Size::new(1280.0, 860.0),
                loc: {
                    let monitors = Monitor::all()?;
                    let region = monitors[0].client_scaled();
                    region.origin + region.size / 2.0 - window.size()? / 2.0
                },
            },
            interface_combo: ComboBox = (&window) => {
                editable: false,
            },
            location_button: Button = (&window) => {
                text: "位置权限设置",
            },
            status_label: Label = (&window) => {
                text: "正在初始化 WiFi 扫描器…",
            },
            header_signal: Button = (&window),
            header_chan: Button = (&window),
            header_rate: Button = (&window),
            header_mode: Button = (&window),
            header_ssid: Button = (&window),
            header_bssid: Button = (&window),
            ap_list: ListBox = (&window),
            detail_box: TextBox = (&window) => {
                readonly: true,
                text: "等待首帧数据…",
            },
            chart: Canvas = (&window),
        }

        let timer_sender = sender.clone();
        spawn(async move {
            let mut ticker = interval(Duration::from_secs(1));
            loop {
                ticker.tick().await;
                timer_sender.post(MainMessage::Tick);
            }
        })
        .detach();

        sender.post(MainMessage::Tick);
        window.show()?;

        let mut model = Self {
            window,
            interface_combo,
            location_button,
            status_label,
            header_signal,
            header_chan,
            header_rate,
            header_mode,
            header_ssid,
            header_bssid,
            ap_list,
            detail_box,
            chart,
            poller,
            interface_items: Vec::new(),
            selected_interface: None,
            aps: Vec::new(),
            selected_bssid: None,
            histories: HashMap::new(),
            permission_denied: false,
            sort_key: SortKey::Signal,
            sort_desc: true,
        };
        model.update_sort_headers()?;
        Ok(model)
    }

    async fn start(&mut self, sender: &ComponentSender<Self>) -> ! {
        start! {
            sender, default: MainMessage::Noop,
            self.window => {
                WindowEvent::Close => MainMessage::Close,
                WindowEvent::Resize | WindowEvent::ThemeChanged => MainMessage::Redraw,
            },
            self.interface_combo => {
                ComboBoxEvent::Select => MainMessage::InterfaceSelected,
            },
            self.location_button => {
                ButtonEvent::Click => MainMessage::OpenLocationSettings,
            },
            self.header_signal => {
                ButtonEvent::Click => MainMessage::SortBy(SortKey::Signal),
            },
            self.header_chan => {
                ButtonEvent::Click => MainMessage::SortBy(SortKey::Channel),
            },
            self.header_rate => {
                ButtonEvent::Click => MainMessage::SortBy(SortKey::Rate),
            },
            self.header_mode => {
                ButtonEvent::Click => MainMessage::SortBy(SortKey::Mode),
            },
            self.header_ssid => {
                ButtonEvent::Click => MainMessage::SortBy(SortKey::Ssid),
            },
            self.header_bssid => {
                ButtonEvent::Click => MainMessage::SortBy(SortKey::Bssid),
            },
            self.ap_list => {
                ListBoxEvent::Select => MainMessage::AccessPointSelected,
            },
            self.detail_box => {},
            self.chart => {},
        }
    }

    async fn update_children(&mut self) -> AppResult<bool> {
        update_children!(
            self.window,
            self.interface_combo,
            self.location_button,
            self.status_label,
            self.header_signal,
            self.header_chan,
            self.header_rate,
            self.header_mode,
            self.header_ssid,
            self.header_bssid,
            self.ap_list,
            self.detail_box,
            self.chart
        )
    }

    async fn update(
        &mut self,
        message: Self::Message,
        sender: &ComponentSender<Self>,
    ) -> AppResult<bool> {
        match message {
            MainMessage::Noop => Ok(false),
            MainMessage::Close => {
                sender.output(());
                Ok(false)
            }
            MainMessage::Redraw => Ok(true),
            MainMessage::Tick => {
                self.refresh_snapshot()?;
                Ok(true)
            }
            MainMessage::InterfaceSelected => {
                self.selected_interface = self.interface_combo.selection()?;
                sender.post(MainMessage::Tick);
                Ok(false)
            }
            MainMessage::AccessPointSelected => {
                if let Some(index) = self.current_selected_ap_index()? {
                    self.selected_bssid = self.aps.get(index).map(|v| v.bssid);
                    self.update_detail_text()?;
                    return Ok(true);
                }
                Ok(false)
            }
            MainMessage::SortBy(key) => {
                if self.sort_key == key {
                    self.sort_desc = !self.sort_desc;
                } else {
                    self.sort_key = key;
                    self.sort_desc = matches!(key, SortKey::Signal | SortKey::Rate);
                }
                self.sort_aps();
                self.update_sort_headers()?;
                self.refresh_list_view()?;
                self.update_detail_text()?;
                Ok(true)
            }
            MainMessage::OpenLocationSettings => {
                self.open_location_settings()?;
                Ok(false)
            }
        }
    }

    fn render(&mut self, _sender: &ComponentSender<Self>) -> AppResult<()> {
        let csize = self.window.client_size()?;
        let top_height = 40.0;
        let left_width = 560.0;
        let header_height = 30.0;
        let chart_height = 280.0;
        let margin = 10.0;

        self.interface_combo.set_rect(Rect::new(
            Point::new(margin, margin),
            Size::new(360.0, top_height - margin),
        ))?;
        self.location_button.set_rect(Rect::new(
            Point::new(380.0, margin),
            Size::new(140.0, top_height - margin),
        ))?;
        self.status_label.set_rect(Rect::new(
            Point::new(530.0, margin + 3.0),
            Size::new((csize.width - 540.0).max(100.0), top_height - margin),
        ))?;

        let left_x = margin;
        let left_y = top_height + margin;
        let left_w = left_width - margin;
        let list_h = (csize.height - top_height - 2.0 * margin - header_height).max(120.0);

        let mut col_x = left_x;
        for ((_, ratio), header) in HEADER_COLUMNS.iter().zip(self.header_buttons_mut()) {
            let mut col_w = left_w * ratio;
            if col_x + col_w > left_x + left_w {
                col_w = left_x + left_w - col_x;
            }
            header.set_rect(Rect::new(
                Point::new(col_x, left_y),
                Size::new(col_w, header_height),
            ))?;
            col_x += col_w;
        }

        self.ap_list.set_rect(Rect::new(
            Point::new(left_x, left_y + header_height),
            Size::new(left_w, list_h),
        ))?;

        let right_x = left_width + margin;
        let right_w = (csize.width - right_x - margin).max(180.0);
        self.detail_box.set_rect(Rect::new(
            Point::new(right_x, top_height + margin),
            Size::new(
                right_w,
                (csize.height - top_height - chart_height - margin).max(120.0),
            ),
        ))?;
        self.chart.set_rect(Rect::new(
            Point::new(
                right_x,
                (csize.height - chart_height).max(top_height + 60.0),
            ),
            Size::new(right_w, (chart_height - margin).max(140.0)),
        ))?;

        self.draw_chart()?;
        Ok(())
    }

    fn render_children(&mut self) -> AppResult<()> {
        Ok(self.window.render()?)
    }
}

impl MainModel {
    fn header_buttons_mut(&mut self) -> [&mut Child<Button>; 6] {
        [
            &mut self.header_signal,
            &mut self.header_chan,
            &mut self.header_rate,
            &mut self.header_mode,
            &mut self.header_ssid,
            &mut self.header_bssid,
        ]
    }

    fn refresh_snapshot(&mut self) -> AppResult<()> {
        match self.poller.collect(self.selected_interface) {
            Ok(snapshot) => {
                self.apply_snapshot(snapshot)?;
            }
            Err(err) => {
                self.status_label.set_text(format!("扫描失败: {err}"))?;
                self.detail_box
                    .set_text("扫描器遇到错误，请确认 WLAN 服务与无线网卡状态。")?;
            }
        }
        Ok(())
    }

    fn apply_snapshot(&mut self, snapshot: WifiSnapshot) -> AppResult<()> {
        let combo_items = snapshot
            .interfaces
            .iter()
            .map(|iface| format!("{} ({})", iface.description, iface.state))
            .collect::<Vec<_>>();
        if combo_items != self.interface_items {
            self.interface_combo.set_items(combo_items.clone())?;
            self.interface_items = combo_items;
        }
        if !self.interface_items.is_empty() {
            self.interface_combo
                .set_selection(snapshot.active_interface)?;
            self.selected_interface = Some(snapshot.active_interface);
        }

        self.aps = snapshot.aps;
        self.permission_denied = snapshot.permission_denied;
        self.sort_aps();

        for ap in &self.aps {
            let key = ap.bssid_text.clone();
            let history = self.histories.entry(key).or_default();
            history.push_back(ap.rssi_dbm);
            while history.len() > HISTORY_CAPACITY {
                history.pop_front();
            }
        }

        if self
            .selected_bssid
            .is_none_or(|bssid| !self.aps.iter().any(|ap| ap.bssid == bssid))
        {
            self.selected_bssid = self
                .aps
                .iter()
                .find(|ap| ap.connected)
                .or_else(|| self.aps.first())
                .map(|ap| ap.bssid);
        }

        self.refresh_list_view()?;
        self.status_label.set_text(snapshot.status)?;
        self.update_detail_text()?;
        Ok(())
    }

    fn refresh_list_view(&mut self) -> AppResult<()> {
        self.ap_list.set_items(self.aps.iter().map(format_ap_row))?;
        if let Some(selected) = self.selected_bssid
            && let Some(index) = self.aps.iter().position(|ap| ap.bssid == selected)
        {
            self.ap_list.set_selected(index, true)?;
        }
        Ok(())
    }

    fn update_detail_text(&mut self) -> AppResult<()> {
        if self.aps.is_empty() {
            let text = if self.permission_denied {
                "当前未能获取 WiFi BSS 列表。可能是位置权限未允许，点击上方位置权限设置后重试。"
                    .to_string()
            } else {
                "当前没有可展示的 AP 数据。".to_string()
            };
            self.detail_box.set_text(text)?;
            return Ok(());
        }

        let selected = self
            .selected_bssid
            .and_then(|bssid| self.aps.iter().find(|ap| ap.bssid == bssid))
            .or_else(|| self.aps.first());
        if let Some(ap) = selected {
            self.detail_box.set_text(format_ap_detail(ap))?;
        }
        Ok(())
    }

    fn current_selected_ap_index(&self) -> AppResult<Option<usize>> {
        for index in 0..self.aps.len() {
            if self.ap_list.is_selected(index)? {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }

    fn sort_aps(&mut self) {
        self.aps.sort_by(|a, b| {
            let ordering = match self.sort_key {
                SortKey::Signal => a.signal_quality.cmp(&b.signal_quality),
                SortKey::Channel => a.channel.cmp(&b.channel),
                SortKey::Rate => a.rate_mbps.total_cmp(&b.rate_mbps),
                SortKey::Mode => a.mode.cmp(&b.mode),
                SortKey::Ssid => a.ssid.cmp(&b.ssid),
                SortKey::Bssid => a.bssid_text.cmp(&b.bssid_text),
            }
            .then_with(|| a.bssid_text.cmp(&b.bssid_text));
            if self.sort_desc {
                ordering.reverse()
            } else {
                ordering
            }
        });
    }

    fn update_sort_headers(&mut self) -> AppResult<()> {
        self.header_signal.set_text(header_text(
            "SIG",
            self.sort_key,
            self.sort_desc,
            SortKey::Signal,
        ))?;
        self.header_chan.set_text(header_text(
            "CH",
            self.sort_key,
            self.sort_desc,
            SortKey::Channel,
        ))?;
        self.header_rate.set_text(header_text(
            "RATE",
            self.sort_key,
            self.sort_desc,
            SortKey::Rate,
        ))?;
        self.header_mode.set_text(header_text(
            "MODE",
            self.sort_key,
            self.sort_desc,
            SortKey::Mode,
        ))?;
        self.header_ssid.set_text(header_text(
            "SSID",
            self.sort_key,
            self.sort_desc,
            SortKey::Ssid,
        ))?;
        self.header_bssid.set_text(header_text(
            "BSSID",
            self.sort_key,
            self.sort_desc,
            SortKey::Bssid,
        ))?;
        Ok(())
    }

    fn draw_chart(&mut self) -> AppResult<()> {
        let canvas_backend = WinioCanvasBackend::new(&mut self.chart)?;
        let root = canvas_backend.into_drawing_area();
        let is_dark = ColorTheme::current()? == ColorTheme::Dark;
        let bg = if is_dark { BLACK } else { WHITE };
        let fg = if is_dark { WHITE } else { BLACK };
        root.fill(&bg)
            .map_err(|err| AppError::Io(std::io::Error::other(err.to_string())))?;

        let selected = self
            .selected_bssid
            .and_then(|bssid| self.aps.iter().find(|ap| ap.bssid == bssid));
        let Some(ap) = selected else {
            root.present()
                .map_err(|err| AppError::Io(std::io::Error::other(err.to_string())))?;
            return Ok(());
        };
        let key = bssid_to_string(ap.bssid);
        let Some(history) = self.histories.get(&key) else {
            root.present()
                .map_err(|err| AppError::Io(std::io::Error::other(err.to_string())))?;
            return Ok(());
        };

        let mut chart = ChartBuilder::on(&root)
            .margin(8)
            .caption(
                format!("Signal Trend: {} ({})", ap.ssid, ap.bssid_text),
                ("sans-serif", 16).into_font().color(&fg),
            )
            .x_label_area_size(28)
            .y_label_area_size(46)
            .build_cartesian_2d(0..(HISTORY_CAPACITY as i32), -100i32..-20i32)
            .map_err(|err| AppError::Io(std::io::Error::other(err.to_string())))?;
        chart
            .configure_mesh()
            .axis_style(fg)
            .light_line_style(fg.mix(0.08))
            .bold_line_style(fg.mix(0.18))
            .label_style(("sans-serif", 11).into_font().color(&fg))
            .x_desc("Samples")
            .y_desc("RSSI (dBm)")
            .draw()
            .map_err(|err| AppError::Io(std::io::Error::other(err.to_string())))?;

        chart
            .draw_series(LineSeries::new(
                history
                    .iter()
                    .enumerate()
                    .map(|(idx, value)| (idx as i32, *value)),
                &RGBColor(220, 20, 60),
            ))
            .map_err(|err| AppError::Io(std::io::Error::other(err.to_string())))?;

        root.present()
            .map_err(|err| AppError::Io(std::io::Error::other(err.to_string())))?;
        Ok(())
    }

    fn open_location_settings(&mut self) -> AppResult<()> {
        let result = Command::new("cmd")
            .args(["/C", "start", "", "ms-settings:privacy-location"])
            .spawn();
        if let Err(err) = result {
            self.status_label
                .set_text(format!("无法打开系统设置: {err}"))?;
            return Ok(());
        }
        self.status_label
            .set_text("已尝试打开位置权限设置，请授权后等待自动刷新")?;
        Ok(())
    }
}

fn header_text(base: &str, key: SortKey, desc: bool, col: SortKey) -> String {
    if key != col {
        return base.to_string();
    }
    if desc {
        format!("{base} ↓")
    } else {
        format!("{base} ↑")
    }
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    if max_chars <= 1 {
        return "…".to_string();
    }
    let mut out = String::new();
    for ch in input.chars().take(max_chars - 1) {
        out.push(ch);
    }
    out.push('…');
    out
}

fn format_ap_row(ap: &AccessPointRecord) -> String {
    let mode = truncate_chars(&ap.mode, 12);
    let ssid = truncate_chars(&ap.ssid, 18);
    let bssid = truncate_chars(&ap.bssid_text, 17);
    format!(
        "{:>3}%/{:>4}dBm | {:>3} | {:>6.1}M | {:<12} | {:<18} | {}{}",
        ap.signal_quality,
        ap.rssi_dbm,
        ap.channel,
        ap.rate_mbps,
        mode,
        ssid,
        bssid,
        if ap.connected { " [C]" } else { "" }
    )
}

fn format_ap_detail(ap: &AccessPointRecord) -> String {
    let mut lines = vec![
        format!("BSSID: {}", ap.bssid_text),
        format!("SSID: {}", ap.ssid),
        format!("MODE: {}", ap.mode),
        format!("CHAN: {}", ap.channel),
        format!("CENTER_FREQ: {} kHz", ap.center_freq_khz),
        format!("RATE: {:.1} Mbps", ap.rate_mbps),
        format!("SIGNAL: {}% ({} dBm)", ap.signal_quality, ap.rssi_dbm),
    ];
    if let (Some(rx), Some(tx)) = (ap.rx_rate_mbps, ap.tx_rate_mbps) {
        lines.push(format!("RX_RATE: {:.1} Mbps", rx));
        lines.push(format!("TX_RATE: {:.1} Mbps", tx));
    }
    if ap.connected {
        lines.push("STATE: Connected".to_string());
    }
    lines.join("\n")
}
