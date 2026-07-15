// SPDX-License-Identifier: MPL-2.0

pub mod post_production;
pub mod prepare;
pub mod selection_canvas;
pub mod subtitle;

use crate::OCR;
use crate::config::Config;
use crate::subfinder::{Params, SubtitleSearch};
use crate::video_player::{self, InnerPlayer, VideoFrame, create_video_player};
use crate::{fl, video_player::VideoPlayerController};
use cosmic::app::context_drawer;
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{self, Alignment, Length, Subscription, Task, futures};
use cosmic::prelude::*;

use cosmic::widget::{self, about::About, icon, menu, nav_bar};
use iced::futures::SinkExt;
use image::{DynamicImage, RgbaImage};
use opencv::core::{MatTraitConst, MatTraitConstManual};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const APP_ICON: &[u8] = include_bytes!("../resources/icons/hicolor/scalable/apps/icon.svg");

pub struct AppModel {
    core: cosmic::Core,
    context_page: ContextPage,
    about: About,
    nav: nav_bar::Model,
    key_binds: HashMap<menu::KeyBind, MenuAction>,
    config: Config,
    time: u32,
    watch_is_active: bool,
    subtitle_page_id: nav_bar::Id,
    post_production_page_id: nav_bar::Id,
    video_frame_rate: f64,

    prepare: prepare::Model,
    subtitle: subtitle::Model,
    post_production: post_production::Model,
}

#[derive(Debug, Clone)]
pub enum Message {
    LaunchUrl(String),
    ToggleContextPage(ContextPage),
    UpdateConfig(Config),
    WatchTick(u32),
    Prepare(prepare::Message),
    Subtitle(subtitle::Message),
    PostProduction(post_production::Message),
}

pub enum Page {
    Prepare,
    Subtitle,
    PostProduction,
}

impl std::fmt::Display for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let x = match self {
            Self::Prepare => fl!("page-prepare"),
            Self::Subtitle => fl!("page-subtitle"),
            Self::PostProduction => fl!("page-post"),
        };
        write!(f, "{x}")
    }
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum ContextPage {
    #[default]
    About,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuAction {
    About,
}

impl menu::action::MenuAction for MenuAction {
    type Message = Message;
    fn message(&self) -> Self::Message {
        match self {
            Self::About => Message::ToggleContextPage(ContextPage::About),
        }
    }
}

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::multi::Executor;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = "dev.justsimplykyle.videosubextract";

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(
        core: cosmic::Core,
        _flags: Self::Flags,
    ) -> (Self, Task<cosmic::Action<Self::Message>>) {
        let mut nav = nav_bar::Model::default();

        let prepare_id = nav
            .insert()
            .text(fl!("page-prepare"))
            .data::<Page>(Page::Prepare)
            .icon(icon::from_name("applications-system-symbolic"))
            .id();

        nav.activate(prepare_id);

        let subtitle_page_id = nav
            .insert()
            .text(fl!("page-subtitle"))
            .data::<Page>(Page::Subtitle)
            .icon(icon::from_name("applications-games-symbolic"))
            .id();

        let post_production_page_id = nav
            .insert()
            .text(fl!("page-post"))
            .data::<Page>(Page::PostProduction)
            .icon(icon::from_name("applications-games-symbolic"))
            .id();

        let about = About::default()
            .name(fl!("app-title"))
            .icon(widget::icon::from_svg_bytes(APP_ICON))
            .version(env!("CARGO_PKG_VERSION"))
            .links([(fl!("repository"), REPOSITORY)])
            .license(env!("CARGO_PKG_LICENSE"));

        let mut app = Self {
            core,
            context_page: ContextPage::default(),
            about,
            nav,
            key_binds: HashMap::new(),
            config: cosmic_config::Config::new(Self::APP_ID, Config::VERSION)
                .map(|context| match Config::get_entry(&context) {
                    Ok(config) => config,
                    Err((_errors, config)) => config,
                })
                .unwrap_or_default(),
            time: 0,
            watch_is_active: false,
            subtitle_page_id,
            post_production_page_id,
            video_frame_rate: 24.0,

            prepare: prepare::Model::default(),
            subtitle: subtitle::Model::default(),
            post_production: post_production::Model::default(),
        };

        let command = app.update_title();
        (app, command)
    }

    fn header_start(&self) -> Vec<Element<'_, Self::Message>> {
        let menu_bar = menu::bar(vec![menu::Tree::with_children(
            menu::root(fl!("view")).apply(Element::from),
            menu::items(
                &self.key_binds,
                vec![menu::Item::Button(fl!("about"), None, MenuAction::About)],
            ),
        )]);
        vec![menu_bar.into()]
    }

    fn nav_model(&self) -> Option<&nav_bar::Model> {
        Some(&self.nav)
    }

    fn context_drawer(&self) -> Option<context_drawer::ContextDrawer<'_, Self::Message>> {
        if !self.core.window.show_context {
            return None;
        }

        Some(match self.context_page {
            ContextPage::About => context_drawer::about(
                &self.about,
                |url| Message::LaunchUrl(url.to_string()),
                Message::ToggleContextPage(ContextPage::About),
            ),
        })
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let space_s = cosmic::theme::spacing().space_s;
        let active = self.nav.active_data::<Page>().unwrap();

        let content: Element<_> = match active {
            Page::Prepare => self.prepare.view().map(Message::Prepare),
            Page::Subtitle => {
                let total_frames = self
                    .prepare
                    .video_controller
                    .as_ref()
                    .map(|x| x.inner.info.total_frames);
                self.subtitle
                    .view(total_frames, self.video_frame_rate)
                    .map(Message::Subtitle)
            }
            Page::PostProduction => self
                .post_production
                .view(self.subtitle.search_active)
                .map(Message::PostProduction),
        };

        let header = widget::row::with_capacity(2)
            .push(widget::text::title1(fl!("welcome")))
            .push(widget::text::title3(active.to_string()))
            .align_y(Alignment::End)
            .spacing(space_s);

        widget::container(widget::column!(header, content).spacing(space_s))
            .width(Length::Fill)
            .height(Length::Fill)
            .apply(widget::container)
            .width(Length::Fill)
            .padding([0, 50])
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .into()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let mut subscriptions = vec![
            self.core()
                .watch_config::<Config>(Self::APP_ID)
                .map(|update| Message::UpdateConfig(update.config)),
        ];

        if self.watch_is_active {
            subscriptions.push(Subscription::run(|| {
                iced::stream::channel(
                    1,
                    |mut emitter: futures::channel::mpsc::Sender<_>| async move {
                        let mut time = 1;
                        let mut interval = tokio::time::interval(Duration::from_secs(1));
                        loop {
                            interval.tick().await;
                            _ = emitter.send(Message::WatchTick(time)).await;
                            time += 1;
                        }
                    },
                )
            }));
        }

        subscriptions.push(self.prepare.subscription().map(Message::Prepare));
        subscriptions.push(
            self.subtitle
                .subscription(self.video_frame_rate)
                .map(Message::Subtitle),
        );

        Subscription::batch(subscriptions)
    }

    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::WatchTick(time) => {
                self.time = time;
                Task::none()
            }
            Message::ToggleContextPage(context_page) => {
                if self.context_page == context_page {
                    self.core.window.show_context = !self.core.window.show_context;
                } else {
                    self.context_page = context_page;
                    self.core.window.show_context = true;
                }
                Task::none()
            }
            Message::UpdateConfig(config) => {
                self.config = config;
                Task::none()
            }
            Message::LaunchUrl(url) => {
                match open::that_detached(&url) {
                    Ok(()) => {}
                    Err(err) => eprintln!("failed to open {url:?}: {err}"),
                }
                Task::none()
            }
            Message::Prepare(msg) => {
                let event = self.prepare.update(msg);

                match event {
                    prepare::Event::StartSubtitleSearch(path, selection) => {
                        self.subtitle.start_search(path, selection);
                        self.nav.activate(self.subtitle_page_id);
                        self.update_title()
                    }
                    prepare::Event::Run(task) => task.map(Message::Prepare).map(Into::into),
                    prepare::Event::None => Task::none(),
                }
            }
            Message::Subtitle(msg) => {
                let event = self.subtitle.update(msg);

                match event {
                    subtitle::Event::GoToPostProduction => {
                        self.post_production.feedback = None; // clear UI feedback internally
                        self.nav.activate(self.post_production_page_id);
                        self.update_title()
                    }
                    subtitle::Event::Run(task) => task.map(Message::Subtitle).map(Into::into),
                    subtitle::Event::None => Task::none(),
                }
            }
            Message::PostProduction(msg) => self
                .post_production
                .update(
                    msg,
                    &mut self.subtitle.results,
                    self.prepare.video_path.as_ref(),
                )
                .map(Message::PostProduction)
                .map(Into::into),
        }
    }

    fn on_nav_select(&mut self, id: nav_bar::Id) -> Task<cosmic::Action<Self::Message>> {
        self.nav.activate(id);
        self.update_title()
    }
}

impl AppModel {
    pub fn update_title(&mut self) -> Task<cosmic::Action<Message>> {
        let mut window_title = fl!("app-title");
        if let Some(page) = self.nav.text(self.nav.active()) {
            window_title.push_str(" — ");
            window_title.push_str(page);
        }
        self.core
            .main_window_id()
            .map_or_else(Task::none, |id| self.set_window_title(window_title, id))
    }
}

pub fn mat_to_dynamic_image(mat: &opencv::core::Mat) -> eyre::Result<DynamicImage> {
    mat_to_image_handle(mat).and_then(|x| {
        RgbaImage::from_raw(x.width(), x.height(), x.into_raw())
            .map(DynamicImage::ImageRgba8)
            .ok_or_else(|| eyre::eyre!("invalid image dimensions"))
    })
}

pub fn mat_to_image_handle(mat: &opencv::core::Mat) -> eyre::Result<RgbaImage> {
    let rows = mat.rows() as u32;
    let cols = mat.cols() as u32;
    let bgr = mat.data_bytes()?;
    let rgba = bgr
        .chunks_exact(3)
        .flat_map(|p| [p[2], p[1], p[0], 255u8])
        .collect::<Vec<_>>();
    RgbaImage::from_raw(cols, rows, rgba).ok_or_else(|| eyre::eyre!("isn't a valid rgbaimage"))
}
