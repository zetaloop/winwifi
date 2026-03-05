mod error;
mod wifi;

use std::{
    collections::{HashMap, VecDeque},
    process::Command,
    time::Duration,
};

use compio::{runtime::spawn, time::interval};
use error::{AppError, AppResult};
use plotters::prelude::{
    BLACK, ChartBuilder, Color, IntoDrawingArea, IntoFont, LineSeries, RGBColor, WHITE,
};
use wifi::{
    convert::bssid_to_string,
    poller::WifiPoller,
    types::{AccessPointRecord, WifiSnapshot},
};
use winio::prelude::*;

const HISTORY_CAPACITY: usize = 180;

fn main() -> AppResult<()> {
    App::new("dev.foxloop.winwifi")?.run_until_event::<MainModel>(())
}

struct MainModel {
    window: Child<Window>,
    interface_combo: Child<ComboBox>,
    location_button: Child<Button>,
    status_label: Child<Label>,
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
}

#[derive(Debug)]
enum MainMessage {
    Noop,
    Close,
    Redraw,
    Tick,
    InterfaceSelected,
    AccessPointSelected,
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

        Ok(Self {
            window,
            interface_combo,
            location_button,
            status_label,
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
        })
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
            MainMessage::OpenLocationSettings => {
                self.open_location_settings()?;
                Ok(false)
            }
        }
    }

    fn render(&mut self, _sender: &ComponentSender<Self>) -> AppResult<()> {
        let csize = self.window.client_size()?;
        let top_height = 40.0;
        let left_width = 460.0;
        let chart_height = 280.0;
        let margin = 10.0;

        self.interface_combo.set_rect(winio::prelude::Rect::new(
            Point::new(margin, margin),
            Size::new(360.0, top_height - margin),
        ))?;
        self.location_button.set_rect(winio::prelude::Rect::new(
            Point::new(380.0, margin),
            Size::new(140.0, top_height - margin),
        ))?;
        self.status_label.set_rect(winio::prelude::Rect::new(
            Point::new(530.0, margin + 3.0),
            Size::new((csize.width - 540.0).max(100.0), top_height - margin),
        ))?;

        self.ap_list.set_rect(winio::prelude::Rect::new(
            Point::new(margin, top_height + margin),
            Size::new(
                left_width - margin,
                (csize.height - top_height - 2.0 * margin).max(120.0),
            ),
        ))?;

        let right_x = left_width + margin;
        let right_w = (csize.width - right_x - margin).max(180.0);
        self.detail_box.set_rect(winio::prelude::Rect::new(
            Point::new(right_x, top_height + margin),
            Size::new(
                right_w,
                (csize.height - top_height - chart_height - margin).max(120.0),
            ),
        ))?;
        self.chart.set_rect(winio::prelude::Rect::new(
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
    fn refresh_snapshot(&mut self) -> AppResult<()> {
        let snapshot = self.poller.collect(self.selected_interface)?;
        self.apply_snapshot(snapshot)?;
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

        self.ap_list
            .set_items(self.aps.iter().map(format_ap_line))?;
        if let Some(selected) = self.selected_bssid
            && let Some(index) = self.aps.iter().position(|ap| ap.bssid == selected)
        {
            self.ap_list.set_selected(index, true)?;
        }

        self.status_label.set_text(snapshot.status)?;
        self.update_detail_text()?;
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

fn format_ap_line(ap: &AccessPointRecord) -> String {
    format!(
        "{:>3}%  CH {:>3}  {:>6.1}M  {:<20}  {}{}",
        ap.signal_quality,
        ap.channel,
        ap.rate_mbps,
        ap.ssid,
        ap.bssid_text,
        if ap.connected { "  [CONNECTED]" } else { "" }
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
