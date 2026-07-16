// SPDX-License-Identifier: MPL-2.0

pub mod post_production;
pub mod prepare;
pub mod selection_canvas;
pub mod subtitle;

use crate::config::Config;
use crate::ocr::OcrModel;
use crate::subfinder::{Params, SubtitleSearch};
use crate::video_player::{self, InnerPlayer, VideoFrame, create_video_player};
use crate::{fl, video_player::VideoPlayerController};
use cosmic::app::context_drawer;
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{self, Alignment, Length, Subscription, Task, futures};
use cosmic::prelude::*;

use cosmic::widget::{self, about::About, icon, menu, nav_bar};
use eyre::Context;
use iced::futures::SinkExt;
use image::{DynamicImage, RgbaImage};
use opencv::core::{MatTraitConst, MatTraitConstManual};
use opencv::sys::std_vectorLcv_PtrLcv_dnn_BackendWrapperGG_set_size_t_const_PtrLBackendWrapperG;
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
    config_handler: cosmic_config::Config,
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
    SetOcrModel(OcrModel),
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
    Settings,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuAction {
    About,
    Settings,
}

impl menu::action::MenuAction for MenuAction {
    type Message = Message;
    fn message(&self) -> Self::Message {
        match self {
            Self::About => Message::ToggleContextPage(ContextPage::About),
            Self::Settings => Message::ToggleContextPage(ContextPage::Settings),
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
            .icon(icon::from_name("applications-graphics-symbolic"))
            .id();

        let post_production_page_id = nav
            .insert()
            .text(fl!("page-post"))
            .data::<Page>(Page::PostProduction)
            .icon(icon::from_name("applications-engineering-symbolic"))
            .id();

        let about = About::default()
            .name(fl!("app-title"))
            .icon(widget::icon::from_svg_bytes(APP_ICON))
            .version(env!("CARGO_PKG_VERSION"))
            .links([(fl!("repository"), REPOSITORY)])
            .license(env!("CARGO_PKG_LICENSE"));

        let (config_handler, config) =
            match cosmic_config::Config::new(Self::APP_ID, Config::VERSION) {
                Ok(context) => {
                    let config = match Config::get_entry(&context) {
                        Ok(config) => config,
                        Err((_errors, config)) => config,
                    };
                    (context, config)
                }
                Err(error) => {
                    panic!("failed to load configuration: {error}");
                }
            };
        let mut app = Self {
            core,
            context_page: ContextPage::default(),
            about,
            nav,
            key_binds: HashMap::new(),
            config_handler,
            config,
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
            menu::root(fl!("advanced")).apply(Element::from),
            menu::items(
                &self.key_binds,
                vec![
                    menu::Item::Button("Settings", None, MenuAction::Settings),
                    menu::Item::Button("About", None, MenuAction::About),
                ],
            ),
        )]);
        vec![menu_bar.into()]
    }

    fn nav_model(&self) -> Option<&nav_bar::Model> {
        Some(&self.nav)
    }

    fn dialog(&self) -> Option<Element<'_, Self::Message>> {
        if !self.core.window.show_context {
            return None;
        }

        let spacing = cosmic::theme::spacing();

        let build_dialog = |title, element, close_msg| {
            widget::dialog()
                .title(title)
                .control(widget::scrollable(element).height(Length::Fill))
                .width(Length::Fill)
                .apply(widget::container)
                .center(700)
                .apply(|x| {
                    let s = iced::widget::Stack::new();
                    let btn = widget::button::icon(icon::from_name("navbar-closed-symbolic"))
                        .class(cosmic::theme::Button::Destructive)
                        .on_press(close_msg)
                        .apply(widget::container)
                        .align_right(Length::Fill)
                        .padding(spacing.space_m);
                    s.push(x).push(btn)
                })
                .apply(widget::container)
                .center(Length::Fill)
                .style(|_| widget::container::background(iced::Color::from_rgba(0., 0., 0., 0.45)))
                .apply(Element::from)
        };

        Some(match self.context_page {
            ContextPage::About => {
                let about = widget::about(&self.about, |url| Message::LaunchUrl(url.to_string()));
                build_dialog(
                    "About",
                    about,
                    Message::ToggleContextPage(ContextPage::About),
                )
            }

            ContextPage::Settings => build_dialog(
                "Settings",
                self.settings_view(),
                Message::ToggleContextPage(ContextPage::Settings),
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
                if let Err(e) = self.config.write_entry(&self.config_handler) {
                    eprintln!("failed to save configuration: {e}");
                }
                Task::none()
            }
            Message::SetOcrModel(model) => {
                if model == self.config.ocr_model {
                    return Task::none();
                }

                if let Err(error) = self.config.set_ocr_model(&self.config_handler, model) {
                    eprintln!("failed to save configuration: {error}");
                }

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
                        self.subtitle
                            .start_search(path, selection, self.config.ocr_model);
                        self.nav.activate(self.subtitle_page_id);
                        self.update_title()
                    }
                    prepare::Event::Run(task) => task.map(Message::Prepare).map(Into::into),
                    prepare::Event::None => Task::none(),
                }
            }
            Message::Subtitle(msg) => {
                let event = self.subtitle.update(msg, &self.config);

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
    fn settings_view(&self) -> Element<'_, Message> {
        let selected = OcrModel::ALL
            .iter()
            .position(|model| *model == self.config.ocr_model);

        let spacing = cosmic::theme::spacing();

        widget::settings::view_column(vec![
            widget::settings::section()
                .title("Text recognition")
                .add(widget::settings::item(
                    "OCR model",
                    widget::dropdown(&OcrModel::LABELS, selected, |index| {
                        Message::SetOcrModel(OcrModel::ALL[index])
                    })
                    .gap(f32::from(spacing.space_m)),
                ))
                .into(),
        ])
        .into()
    }

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

pub fn mat_to_image_handle(mat: &opencv::core::Mat) -> eyre::Result<RgbaImage> {
    let rows = u32::try_from(mat.rows()).context("image has a negative height")?;
    let cols = u32::try_from(mat.cols()).context("image has a negative width")?;
    let channels = mat.channels();

    // A Mat ROI can have a stride wider than its visible rows. `copy_to` makes
    // an independent, packed copy; `try_clone` would only clone the Mat header.
    let mut packed = opencv::core::Mat::default();
    mat.copy_to(&mut packed)
        .context("failed to copy image into packed storage")?;
    let pixels = packed
        .data_bytes()
        .context("failed to access packed image bytes")?;

    let pixel_count = (cols as usize)
        .checked_mul(rows as usize)
        .ok_or_else(|| eyre::eyre!("image dimensions are too large"))?;
    let expected_len = pixel_count
        .checked_mul(channels as usize)
        .ok_or_else(|| eyre::eyre!("image buffer is too large"))?;
    if !matches!(channels, 1 | 3 | 4) || pixels.len() != expected_len {
        return Err(eyre::eyre!(
            "expected a packed 8-bit grayscale, BGR, or BGRA Mat; got {channels} channels and {} bytes for a {cols}x{rows} image",
            pixels.len()
        ));
    }

    let mut rgba = Vec::with_capacity(pixel_count * 4);
    match channels {
        1 => rgba.extend(pixels.iter().flat_map(|&v| [v, v, v, 255])),
        3 => rgba.extend(pixels.chunks_exact(3).flat_map(|p| [p[2], p[1], p[0], 255])),
        4 => rgba.extend(
            pixels
                .chunks_exact(4)
                .flat_map(|p| [p[2], p[1], p[0], p[3]]),
        ),
        _ => unreachable!("channel count was checked above"),
    }

    RgbaImage::from_raw(cols, rows, rgba)
        .ok_or_else(|| eyre::eyre!("failed to construct RGBA image"))
}
