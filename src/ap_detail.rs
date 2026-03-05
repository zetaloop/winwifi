use std::{convert::Infallible, rc::Rc};

use send_wrapper::SendWrapper;
use windows_core::{HSTRING, Interface, Weak};
use winio::prelude::*;
use winio_callback::Callback;
use winui3::Microsoft::UI::Xaml::{
    Controls as MUXC, FrameworkElement, HorizontalAlignment, TextAlignment, TextWrapping,
    VerticalAlignment,
};

const CARD_PADDING: f64 = 10.0;
const KEY_COLUMN_WIDTH: f64 = 132.0;
const LINE_HEIGHT: f64 = 28.0;

#[derive(Debug, Clone, Default)]
pub struct DetailViewModel {
    pub title: String,
    pub subtitle: String,
    pub rows: Vec<(String, String)>,
    pub message: Option<String>,
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

pub struct ApDetailCard {
    _idle: SendWrapper<Rc<Callback<()>>>,
    widget: WinuiWidget,
    canvas: MUXC::Canvas,
    model: DetailViewModel,
    viewport_width: f64,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ApDetailCardMessage {}

impl ApDetailCard {
    fn new(parent: impl AsContainer) -> winio::Result<Self> {
        let scroll_view = MUXC::ScrollViewer::new()?;
        scroll_view.SetHorizontalScrollBarVisibility(MUXC::ScrollBarVisibility::Disabled)?;
        scroll_view.SetVerticalScrollBarVisibility(MUXC::ScrollBarVisibility::Auto)?;

        let canvas = MUXC::Canvas::new()?;
        scroll_view.SetContent(&canvas)?;

        Ok(Self {
            _idle: SendWrapper::new(Rc::new(Callback::new())),
            widget: WinuiWidget::new(parent, scroll_view.cast()?)?,
            canvas,
            model: DetailViewModel::default(),
            viewport_width: 320.0,
        })
    }

    pub fn set_rect(&mut self, rect: Rect) -> winio::Result<()> {
        self.widget.set_loc(rect.origin)?;
        self.widget.set_size(rect.size)?;
        self.viewport_width = rect.size.width;
        self.redraw()
    }

    pub fn set_model(&mut self, model: DetailViewModel) -> winio::Result<()> {
        self.model = model;
        self.redraw()
    }

    pub fn set_message(&mut self, message: impl Into<String>) -> winio::Result<()> {
        self.model = DetailViewModel {
            title: "WiFi Details".to_string(),
            subtitle: String::new(),
            rows: Vec::new(),
            message: Some(message.into()),
        };
        self.redraw()
    }

    fn redraw(&mut self) -> winio::Result<()> {
        self.canvas.Children()?.Clear()?;

        let content_width = (self.viewport_width - CARD_PADDING * 2.0).max(200.0);
        let mut y = CARD_PADDING;

        let title = make_text(
            if self.model.title.is_empty() {
                "WiFi Details"
            } else {
                &self.model.title
            },
            content_width,
            30.0,
            TextAlignment::Left,
            TextWrapping::NoWrap,
        )?;
        MUXC::Canvas::SetLeft(&title, CARD_PADDING)?;
        MUXC::Canvas::SetTop(&title, y)?;
        self.canvas.Children()?.Append(&title)?;
        y += 34.0;

        if !self.model.subtitle.is_empty() {
            let subtitle = make_text(
                &self.model.subtitle,
                content_width,
                24.0,
                TextAlignment::Left,
                TextWrapping::NoWrap,
            )?;
            MUXC::Canvas::SetLeft(&subtitle, CARD_PADDING)?;
            MUXC::Canvas::SetTop(&subtitle, y)?;
            self.canvas.Children()?.Append(&subtitle)?;
            y += 26.0;
        }

        if let Some(message) = &self.model.message {
            let message = make_text(
                message,
                content_width,
                140.0,
                TextAlignment::Left,
                TextWrapping::Wrap,
            )?;
            MUXC::Canvas::SetLeft(&message, CARD_PADDING)?;
            MUXC::Canvas::SetTop(&message, y + 6.0)?;
            self.canvas.Children()?.Append(&message)?;
            y += 150.0;
        } else {
            let value_width = (content_width - KEY_COLUMN_WIDTH).max(80.0);
            for (key, value) in &self.model.rows {
                let key = make_text(
                    key,
                    KEY_COLUMN_WIDTH - 8.0,
                    LINE_HEIGHT,
                    TextAlignment::Left,
                    TextWrapping::NoWrap,
                )?;
                MUXC::Canvas::SetLeft(&key, CARD_PADDING)?;
                MUXC::Canvas::SetTop(&key, y)?;
                self.canvas.Children()?.Append(&key)?;

                let value = make_text(
                    value,
                    value_width,
                    LINE_HEIGHT,
                    TextAlignment::Left,
                    TextWrapping::NoWrap,
                )?;
                MUXC::Canvas::SetLeft(&value, CARD_PADDING + KEY_COLUMN_WIDTH)?;
                MUXC::Canvas::SetTop(&value, y)?;
                self.canvas.Children()?.Append(&value)?;
                y += LINE_HEIGHT;
            }
        }

        self.canvas.SetWidth(content_width + CARD_PADDING * 2.0)?;
        self.canvas.SetHeight((y + CARD_PADDING).max(160.0))?;
        Ok(())
    }
}

fn make_text(
    text: &str,
    width: f64,
    height: f64,
    align: TextAlignment,
    wrapping: TextWrapping,
) -> winio::Result<MUXC::TextBlock> {
    let tb = MUXC::TextBlock::new()?;
    let display = if matches!(wrapping, TextWrapping::NoWrap) {
        truncate_chars(text, (width / 7.0) as usize)
    } else {
        text.to_string()
    };
    tb.SetText(&HSTRING::from(display))?;
    tb.SetWidth(width)?;
    tb.SetHeight(height)?;
    tb.SetTextAlignment(align)?;
    tb.SetTextWrapping(wrapping)?;
    tb.SetVerticalAlignment(VerticalAlignment::Center)?;
    Ok(tb)
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

impl Component for ApDetailCard {
    type Error = winio::Error;
    type Event = Infallible;
    type Init<'a> = BorrowedContainer<'a>;
    type Message = ApDetailCardMessage;

    async fn init(init: Self::Init<'_>, _sender: &ComponentSender<Self>) -> winio::Result<Self> {
        Self::new(init)
    }

    async fn start(&mut self, _sender: &ComponentSender<Self>) -> ! {
        std::future::pending().await
    }

    async fn update(
        &mut self,
        _message: Self::Message,
        _sender: &ComponentSender<Self>,
    ) -> winio::Result<bool> {
        Ok(false)
    }
}

impl AsWidget for ApDetailCard {
    fn as_widget(&self) -> BorrowedWidget<'_> {
        self.widget.as_widget()
    }
}
