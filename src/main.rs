mod error;

use std::ops::Deref;

use error::{AppError, AppResult};
use winio::prelude::*;

fn main() -> AppResult<()> {
    App::new("dev.foxloop.winwifi")?.run_until_event::<MainModel>(())
}

struct MainModel {
    window: Child<Window>,
    root: Child<TabView>,
    overview: Child<OverviewPage>,
}

#[derive(Debug)]
enum MainMessage {
    Noop,
    Close,
    Redraw,
}

impl Component for MainModel {
    type Error = AppError;
    type Event = ();
    type Init<'a> = ();
    type Message = MainMessage;

    async fn init(_init: Self::Init<'_>, _sender: &ComponentSender<Self>) -> AppResult<Self> {
        init! {
            window: Window = (()) => {
                text: "WinWiFi",
                size: Size::new(1200.0, 760.0),
                loc: {
                    let monitors = Monitor::all()?;
                    let region = monitors[0].client_scaled();
                    region.origin + region.size / 2.0 - window.size()? / 2.0
                },
            },
            root: TabView = (&window),
            overview: OverviewPage = (()),
        }
        root.push(&overview)?;
        window.show()?;

        Ok(Self {
            window,
            root,
            overview,
        })
    }

    async fn start(&mut self, sender: &ComponentSender<Self>) -> ! {
        start! {
            sender, default: MainMessage::Noop,
            self.window => {
                WindowEvent::Close => MainMessage::Close,
                WindowEvent::Resize | WindowEvent::ThemeChanged => MainMessage::Redraw,
            },
            self.root => {
                TabViewEvent::Select => MainMessage::Redraw,
            },
            self.overview => {},
        }
    }

    async fn update_children(&mut self) -> AppResult<bool> {
        try_join_update!(
            self.window.update(),
            self.root.update(),
            self.overview.update(),
        )
    }

    async fn update(
        &mut self,
        message: Self::Message,
        sender: &ComponentSender<Self>,
    ) -> AppResult<bool> {
        match message {
            MainMessage::Noop => Ok(false),
            MainMessage::Redraw => Ok(true),
            MainMessage::Close => {
                sender.output(());
                Ok(false)
            }
        }
    }

    fn render(&mut self, _sender: &ComponentSender<Self>) -> AppResult<()> {
        self.root.set_rect(self.window.client_size()?.into())?;
        Ok(())
    }

    fn render_children(&mut self) -> AppResult<()> {
        Ok(self.window.render()?)
    }
}

struct OverviewPage {
    window: Child<TabViewItem>,
    text: Child<Label>,
}

#[derive(Debug)]
enum OverviewPageEvent {}

#[derive(Debug)]
enum OverviewPageMessage {}

impl Component for OverviewPage {
    type Error = AppError;
    type Event = OverviewPageEvent;
    type Init<'a> = ();
    type Message = OverviewPageMessage;

    async fn init(_init: Self::Init<'_>, _sender: &ComponentSender<Self>) -> AppResult<Self> {
        init! {
            window: TabViewItem = (()) => {
                text: "实时监测",
            },
            text: Label = (&window) => {
                text: "正在初始化 WiFi 监测模块…",
            },
        }
        Ok(Self { window, text })
    }

    async fn update_children(&mut self) -> AppResult<bool> {
        update_children!(self.window, self.text)
    }

    fn render(&mut self, _sender: &ComponentSender<Self>) -> AppResult<()> {
        let csize = self.window.size()?;
        let mut root = layout! {
            StackPanel::new(Orient::Vertical),
            self.text,
        };
        root.set_size(csize)?;
        Ok(())
    }
}

impl Deref for OverviewPage {
    type Target = TabViewItem;

    fn deref(&self) -> &Self::Target {
        &self.window
    }
}
