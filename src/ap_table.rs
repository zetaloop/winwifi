use std::rc::Rc;

use send_wrapper::SendWrapper;
use windows_core::{HSTRING, Interface, Weak};
use winio::prelude::*;
use winio_callback::Callback;
use winui3::Microsoft::UI::Xaml::{
    Controls as MUXC, FrameworkElement, HorizontalAlignment, TextAlignment, TextWrapping,
    VerticalAlignment,
};

const CELL_PADDING: f64 = 8.0;
const MIN_COL_WIDTH: f64 = 48.0;
const ROW_HEIGHT: f64 = 32.0;

#[derive(Debug, Clone)]
pub struct ApTableRow {
    pub signal: String,
    pub channel: String,
    pub rate: String,
    pub mode: String,
    pub ssid: String,
    pub bssid: String,
}

struct WinuiWidget {
    handle: FrameworkElement,
    parent: Weak<MUXC::Canvas>,
}

impl WinuiWidget {
    fn new(parent: impl AsContainer, handle: FrameworkElement) -> winio::Result<Self> {
        handle.SetHorizontalAlignment(HorizontalAlignment::Center)?;
        handle.SetVerticalAlignment(VerticalAlignment::Center)?;

        let parent = parent.as_container();
        let canvas = parent.as_winui();
        canvas.Children()?.Append(&handle)?;

        Ok(Self {
            handle,
            parent: canvas.downgrade()?,
        })
    }

    fn set_loc(&self, p: Point) -> winio::Result<()> {
        MUXC::Canvas::SetLeft(&self.handle, p.x)?;
        MUXC::Canvas::SetTop(&self.handle, p.y)?;
        Ok(())
    }

    fn set_size(&self, v: Size) -> winio::Result<()> {
        self.handle.SetWidth(v.width)?;
        self.handle.SetHeight(v.height)?;
        Ok(())
    }

    fn drop_impl(&self) -> winio::Result<()> {
        let Some(parent) = self.parent.upgrade() else {
            return Ok(());
        };
        let children = parent.Children()?;
        let mut index = 0;
        if children.IndexOf(&self.handle, &mut index).is_ok() {
            children.RemoveAt(index)?;
        }
        Ok(())
    }
}

impl Drop for WinuiWidget {
    fn drop(&mut self) {
        let _ = self.drop_impl();
    }
}

impl AsWidget for WinuiWidget {
    fn as_widget(&self) -> BorrowedWidget<'_> {
        BorrowedWidget::winui(&self.handle)
    }
}

pub struct ApTable {
    on_select: SendWrapper<Rc<Callback<()>>>,
    widget: WinuiWidget,
    list_box: MUXC::ListBox,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ApTableEvent {
    Select,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ApTableMessage {}

impl ApTable {
    fn new(parent: impl AsContainer) -> winio::Result<Self> {
        let list_box = MUXC::ListBox::new()?;
        list_box.SetSelectionMode(MUXC::SelectionMode::Single)?;
        list_box.SetHorizontalContentAlignment(HorizontalAlignment::Stretch)?;

        let on_select = SendWrapper::new(Rc::new(Callback::new()));
        {
            let on_select = on_select.clone();
            list_box.SelectionChanged(&MUXC::SelectionChangedEventHandler::new(move |_, _| {
                on_select.signal::<()>(());
                Ok(())
            }))?;
        }

        Ok(Self {
            on_select,
            widget: WinuiWidget::new(parent, list_box.cast()?)?,
            list_box,
        })
    }

    pub fn set_rect(&self, rect: Rect) -> winio::Result<()> {
        self.widget.set_loc(rect.origin)?;
        self.widget.set_size(rect.size)?;
        Ok(())
    }

    pub fn set_rows(&mut self, rows: &[ApTableRow], widths: [f64; 6]) -> winio::Result<()> {
        let items = self.list_box.Items()?;
        items.Clear()?;

        let widths = widths.map(|w| w.max(MIN_COL_WIDTH));
        let row_width = widths.iter().sum::<f64>();
        for row in rows {
            let item = MUXC::ListBoxItem::new()?;
            item.SetHorizontalContentAlignment(HorizontalAlignment::Stretch)?;
            let row_canvas = MUXC::Canvas::new()?;
            row_canvas.SetWidth(row_width)?;
            row_canvas.SetHeight(ROW_HEIGHT)?;

            let mut col_left = 0.0;
            for (idx, text) in [
                row.signal.as_str(),
                row.channel.as_str(),
                row.rate.as_str(),
                row.mode.as_str(),
                row.ssid.as_str(),
                row.bssid.as_str(),
            ]
            .iter()
            .enumerate()
            {
                let align = if idx <= 2 {
                    TextAlignment::Right
                } else {
                    TextAlignment::Left
                };
                let col_width = widths[idx];
                let cell = make_cell(text, col_width, align)?;
                MUXC::Canvas::SetLeft(&cell, col_left + CELL_PADDING)?;
                MUXC::Canvas::SetTop(&cell, 0.0)?;
                row_canvas.Children()?.Append(&cell)?;
                col_left += col_width;
            }

            item.SetContent(&row_canvas)?;
            items.Append(&item)?;
        }
        Ok(())
    }

    pub fn selected_index(&self) -> winio::Result<Option<usize>> {
        let index = self.list_box.SelectedIndex()?;
        if index < 0 {
            Ok(None)
        } else {
            Ok(Some(index as usize))
        }
    }

    pub fn set_selected_index(&self, index: Option<usize>) -> winio::Result<()> {
        self.list_box
            .SetSelectedIndex(index.map_or(-1, |v| v as i32))?;
        Ok(())
    }
}

fn make_cell(text: &str, width: f64, align: TextAlignment) -> winio::Result<MUXC::TextBlock> {
    let cell = MUXC::TextBlock::new()?;
    let content_width = (width - CELL_PADDING * 2.0).max(1.0);
    let display_text = truncate_chars(text, (content_width / 7.0) as usize);
    cell.SetText(&HSTRING::from(display_text))?;
    cell.SetWidth(content_width)?;
    cell.SetHeight(ROW_HEIGHT)?;
    cell.SetTextAlignment(align)?;
    cell.SetTextWrapping(TextWrapping::NoWrap)?;
    cell.SetVerticalAlignment(VerticalAlignment::Center)?;
    Ok(cell)
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    if max_chars < 2 || input.chars().count() <= max_chars {
        return input.to_string();
    }
    let mut out = String::new();
    for ch in input.chars().take(max_chars - 1) {
        out.push(ch);
    }
    out.push('…');
    out
}

impl Component for ApTable {
    type Error = winio::Error;
    type Event = ApTableEvent;
    type Init<'a> = BorrowedContainer<'a>;
    type Message = ApTableMessage;

    async fn init(init: Self::Init<'_>, _sender: &ComponentSender<Self>) -> winio::Result<Self> {
        Self::new(init)
    }

    async fn start(&mut self, sender: &ComponentSender<Self>) -> ! {
        loop {
            self.on_select.wait().await;
            sender.output(ApTableEvent::Select);
        }
    }

    async fn update(
        &mut self,
        _message: Self::Message,
        _sender: &ComponentSender<Self>,
    ) -> winio::Result<bool> {
        Ok(false)
    }
}

impl AsWidget for ApTable {
    fn as_widget(&self) -> BorrowedWidget<'_> {
        self.widget.as_widget()
    }
}
