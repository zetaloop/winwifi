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
const TABLE_HEADER_HEIGHT: f64 = 30.0;
const TABLE_ROW_HEIGHT: f64 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Signal,
    Channel,
    Rate,
    Mode,
    Ssid,
    Bssid,
}

const TABLE_COLUMNS: [(&str, SortKey, f64); 6] = [
    ("SIGNAL", SortKey::Signal, 0.13),
    ("CHAN", SortKey::Channel, 0.10),
    ("RATE", SortKey::Rate, 0.11),
    ("MODE", SortKey::Mode, 0.18),
    ("SSID", SortKey::Ssid, 0.25),
    ("BSSID", SortKey::Bssid, 0.23),
];

struct ColumnLayout {
    title: &'static str,
    key: SortKey,
    x: f64,
    width: f64,
}

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
    ap_table: Child<Canvas>,
    detail_box: Child<TextBox>,
    chart: Child<Canvas>,
    poller: WifiPoller,
    interface_items: Vec<String>,
    selected_interface: Option<usize>,
    aps: Vec<AccessPointRecord>,
    selected_bssid: Option<[u8; 6]>,
    histories: HashMap<String, VecDeque<i32>>,
    permission_denied: bool,
    table_cursor: Point,
    table_scroll: usize,
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
    TableMouseMove(Point),
    TableMouseDown(MouseButton),
    TableWheel(Vector),
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
            ap_table: Canvas = (&window),
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
            ap_table,
            detail_box,
            chart,
            poller,
            interface_items: Vec::new(),
            selected_interface: None,
            aps: Vec::new(),
            selected_bssid: None,
            histories: HashMap::new(),
            permission_denied: false,
            table_cursor: Point::zero(),
            table_scroll: 0,
            sort_key: SortKey::Signal,
            sort_desc: true,
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
            self.ap_table => {
                CanvasEvent::MouseMove(p) => MainMessage::TableMouseMove(p),
                CanvasEvent::MouseDown(btn) => MainMessage::TableMouseDown(btn),
                CanvasEvent::MouseWheel(w) => MainMessage::TableWheel(w),
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
            self.ap_table,
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
            MainMessage::TableMouseMove(p) => {
                self.table_cursor = p;
                Ok(false)
            }
            MainMessage::TableMouseDown(button) => {
                if button == MouseButton::Left && self.handle_table_left_click()? {
                    self.update_detail_text()?;
                    return Ok(true);
                }
                Ok(false)
            }
            MainMessage::TableWheel(w) => Ok(self.handle_table_wheel(w.y)?),
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

        self.ap_table.set_rect(Rect::new(
            Point::new(margin, top_height + margin),
            Size::new(
                left_width - margin,
                (csize.height - top_height - 2.0 * margin).max(120.0),
            ),
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

        self.normalize_table_scroll()?;
        self.draw_table()?;
        self.draw_chart()?;
        Ok(())
    }

    fn render_children(&mut self) -> AppResult<()> {
        Ok(self.window.render()?)
    }
}

impl MainModel {
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

        self.normalize_table_scroll()?;
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

    fn table_column_layout(&self, width: f64) -> Vec<ColumnLayout> {
        let mut x = 0.0;
        let mut result = Vec::with_capacity(TABLE_COLUMNS.len());
        for (index, (title, key, ratio)) in TABLE_COLUMNS.iter().enumerate() {
            let mut column_width = width * ratio;
            if index + 1 == TABLE_COLUMNS.len() {
                column_width = (width - x).max(20.0);
            }
            result.push(ColumnLayout {
                title,
                key: *key,
                x,
                width: column_width.max(20.0),
            });
            x += column_width;
        }
        result
    }

    fn table_visible_rows(&self, table_height: f64) -> usize {
        let rows_height = (table_height - TABLE_HEADER_HEIGHT).max(TABLE_ROW_HEIGHT);
        (rows_height / TABLE_ROW_HEIGHT).floor().max(1.0) as usize
    }

    fn normalize_table_scroll(&mut self) -> AppResult<()> {
        let size = self.ap_table.size().unwrap_or(Size::new(560.0, 400.0));
        let visible_rows = self.table_visible_rows(size.height);
        let max_scroll = self.aps.len().saturating_sub(visible_rows);
        self.table_scroll = self.table_scroll.min(max_scroll);
        Ok(())
    }

    fn ensure_selected_visible(&mut self) -> AppResult<()> {
        let Some(selected) = self.selected_bssid else {
            return Ok(());
        };
        let Some(index) = self.aps.iter().position(|ap| ap.bssid == selected) else {
            return Ok(());
        };
        let size = self.ap_table.size().unwrap_or(Size::new(560.0, 400.0));
        let visible_rows = self.table_visible_rows(size.height);
        if index < self.table_scroll {
            self.table_scroll = index;
        } else if index >= self.table_scroll + visible_rows {
            self.table_scroll = index.saturating_sub(visible_rows.saturating_sub(1));
        }
        Ok(())
    }

    fn handle_table_wheel(&mut self, delta_y: f64) -> AppResult<bool> {
        if self.aps.is_empty() || delta_y.abs() < f64::EPSILON {
            return Ok(false);
        }
        let size = self.ap_table.size()?;
        let visible_rows = self.table_visible_rows(size.height);
        let max_scroll = self.aps.len().saturating_sub(visible_rows);
        if max_scroll == 0 {
            return Ok(false);
        }
        let step = ((delta_y.abs() / 80.0).round() as usize).max(1);
        let old_scroll = self.table_scroll;
        if delta_y > 0.0 {
            self.table_scroll = self.table_scroll.saturating_sub(step);
        } else {
            self.table_scroll = (self.table_scroll + step).min(max_scroll);
        }
        Ok(self.table_scroll != old_scroll)
    }

    fn handle_table_left_click(&mut self) -> AppResult<bool> {
        let size = self.ap_table.size()?;
        let p = self.table_cursor;
        if p.x < 0.0 || p.y < 0.0 || p.x > size.width || p.y > size.height {
            return Ok(false);
        }

        let columns = self.table_column_layout(size.width);
        if p.y <= TABLE_HEADER_HEIGHT {
            for column in &columns {
                if p.x >= column.x && p.x < column.x + column.width {
                    if self.sort_key == column.key {
                        self.sort_desc = !self.sort_desc;
                    } else {
                        self.sort_key = column.key;
                        self.sort_desc = matches!(column.key, SortKey::Signal | SortKey::Rate);
                    }
                    self.sort_aps();
                    self.ensure_selected_visible()?;
                    return Ok(true);
                }
            }
            return Ok(false);
        }

        let row = ((p.y - TABLE_HEADER_HEIGHT) / TABLE_ROW_HEIGHT).floor() as usize;
        let index = self.table_scroll + row;
        if let Some(ap) = self.aps.get(index) {
            self.selected_bssid = Some(ap.bssid);
            self.ensure_selected_visible()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn draw_table(&mut self) -> AppResult<()> {
        let size = self.ap_table.size()?;
        let columns = self.table_column_layout(size.width);
        let visible_rows = self.table_visible_rows(size.height);
        let mut ctx = self.ap_table.context()?;
        let is_dark = ColorTheme::current()? == ColorTheme::Dark;

        let background = SolidColorBrush::new(if is_dark {
            Color::new(28, 28, 28, 255)
        } else {
            Color::new(250, 250, 250, 255)
        });
        let header_background = SolidColorBrush::new(if is_dark {
            Color::new(45, 45, 45, 255)
        } else {
            Color::new(235, 235, 235, 255)
        });
        let selected_background = SolidColorBrush::new(if is_dark {
            Color::new(58, 85, 130, 255)
        } else {
            Color::new(208, 225, 255, 255)
        });
        let text_brush = SolidColorBrush::new(if is_dark {
            Color::new(240, 240, 240, 255)
        } else {
            Color::new(30, 30, 30, 255)
        });
        let pen = BrushPen::new(&text_brush, if is_dark { 0.6 } else { 0.4 });
        let header_font = DrawingFontBuilder::new()
            .family("Segoe UI")
            .size(13.0)
            .halign(HAlign::Left)
            .valign(VAlign::Center)
            .build();
        let row_font = DrawingFontBuilder::new()
            .family("Consolas")
            .size(12.0)
            .halign(HAlign::Left)
            .valign(VAlign::Center)
            .build();

        ctx.fill_rect(&background, Rect::new(Point::zero(), size))?;
        ctx.fill_rect(
            &header_background,
            Rect::new(Point::zero(), Size::new(size.width, TABLE_HEADER_HEIGHT)),
        )?;
        ctx.draw_rect(&pen, Rect::new(Point::zero(), size))?;

        for column in &columns {
            let arrow = if self.sort_key == column.key {
                if self.sort_desc { " ↓" } else { " ↑" }
            } else {
                ""
            };
            let title = format!("{}{}", column.title, arrow);
            let text = fit_text(&ctx, header_font.clone(), &title, column.width - 10.0)?;
            ctx.draw_str(
                &text_brush,
                header_font.clone(),
                Point::new(column.x + 5.0, TABLE_HEADER_HEIGHT / 2.0),
                text,
            )?;
            if column.x > 0.0 {
                ctx.draw_line(
                    &pen,
                    Point::new(column.x, 0.0),
                    Point::new(column.x, size.height),
                )?;
            }
        }
        ctx.draw_line(
            &pen,
            Point::new(0.0, TABLE_HEADER_HEIGHT),
            Point::new(size.width, TABLE_HEADER_HEIGHT),
        )?;

        for row in 0..visible_rows {
            let index = self.table_scroll + row;
            if index >= self.aps.len() {
                break;
            }
            let ap = &self.aps[index];
            let y = TABLE_HEADER_HEIGHT + row as f64 * TABLE_ROW_HEIGHT;
            if self.selected_bssid == Some(ap.bssid) {
                ctx.fill_rect(
                    &selected_background,
                    Rect::new(Point::new(0.0, y), Size::new(size.width, TABLE_ROW_HEIGHT)),
                )?;
            }

            for column in &columns {
                let raw = match column.key {
                    SortKey::Signal => format!("{}% / {}dBm", ap.signal_quality, ap.rssi_dbm),
                    SortKey::Channel => ap.channel.to_string(),
                    SortKey::Rate => format!("{:.1}M", ap.rate_mbps),
                    SortKey::Mode => ap.mode.clone(),
                    SortKey::Ssid => {
                        if ap.connected {
                            format!("{} [C]", ap.ssid)
                        } else {
                            ap.ssid.clone()
                        }
                    }
                    SortKey::Bssid => ap.bssid_text.clone(),
                };
                let cell = fit_text(&ctx, row_font.clone(), &raw, column.width - 8.0)?;
                ctx.draw_str(
                    &text_brush,
                    row_font.clone(),
                    Point::new(column.x + 4.0, y + TABLE_ROW_HEIGHT / 2.0),
                    cell,
                )?;
            }

            ctx.draw_line(
                &pen,
                Point::new(0.0, y + TABLE_ROW_HEIGHT),
                Point::new(size.width, y + TABLE_ROW_HEIGHT),
            )?;
        }

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

fn fit_text(
    ctx: &DrawingContext<'_>,
    font: DrawingFont,
    text: &str,
    max_width: f64,
) -> AppResult<String> {
    if max_width <= 6.0 {
        return Ok(String::new());
    }
    let size = ctx.measure_str(font.clone(), text)?;
    if size.width <= max_width {
        return Ok(text.to_string());
    }
    let ellipsis = "…";
    if ctx.measure_str(font.clone(), ellipsis)?.width > max_width {
        return Ok(String::new());
    }

    let mut chars = text.chars().collect::<Vec<_>>();
    while !chars.is_empty() {
        chars.pop();
        let candidate = format!("{}{}", chars.iter().collect::<String>(), ellipsis);
        if ctx.measure_str(font.clone(), &candidate)?.width <= max_width {
            return Ok(candidate);
        }
    }
    Ok(ellipsis.to_string())
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
