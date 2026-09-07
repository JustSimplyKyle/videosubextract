// SPDX-License-Identifier: MPL-2.0

pub mod post_production;
pub mod prepare;
pub mod selection_canvas;
pub mod subtitle;

use crate::config::{Config, Language, ProcessingResolution, SubtitleDetector};
use crate::native_video_sub_finder::NativeSearchParams;
use crate::ocr::OcrModel;
use crate::video_player::{self, InnerPlayer, create_video_player};
use crate::{fl, i18n, video_player::VideoPlayerController};
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{self, Alignment, Length, Subscription, Task, futures};
use cosmic::prelude::*;

use cosmic::widget::{self, about::About, icon, menu, nav_bar};
use iced::futures::SinkExt;
use rfd::AsyncFileDialog;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const APP_ICON: &[u8] = include_bytes!("../resources/icons/hicolor/scalable/apps/icon.svg");

macro_rules! log {
    ($($arg:tt)*) => {
        Task::done(Message::ErrorReported(Arc::new(eyre::eyre!($($arg)*)))).map(Into::into)
    };
}

pub fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

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
    prepare_page_id: nav_bar::Id,
    subtitle_page_id: nav_bar::Id,
    post_production_page_id: nav_bar::Id,
    video_frame_rate: f64,

    prepare: prepare::Model,
    subtitle: subtitle::Model,
    post_production: post_production::Model,
    toasts: widget::Toasts<Message>,
    errors: Vec<Arc<eyre::Report>>,
}

#[derive(Debug, Clone)]
pub enum Message {
    LaunchUrl(String),
    ToggleContextPage(ContextPage),
    SetOcrModel(OcrModel),
    PickCustomOcr,
    CustomOcrPicked(Option<PathBuf>),
    RemoveCustomOcr(crate::ocr::plugin_loader::DynamicLibrary),
    SetSubtitleDetector(SubtitleDetector),
    SetNativeSearchParams(NativeSearchParams),
    SetPostOcrProcessing(bool),
    SetProcessingResolution(ProcessingResolution),
    SetLanguage(Language),
    UpdateConfig(Config),
    WatchTick(u32),
    Prepare(prepare::Message),
    Subtitle(subtitle::Message),
    PostProduction(post_production::Message),
    CloseToast(widget::ToastId),
    ErrorReported(Arc<eyre::Report>),
}

pub enum Page {
    Prepare,
    Subtitle,
    PostProduction,
}

impl Page {
    pub fn details(&self) -> String {
        match self {
            Page::Prepare => fl!("page-prepare-details"),
            Page::Subtitle => fl!("page-subtitle-details"),
            Page::PostProduction => fl!("page-post-details"),
        }
    }
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
    Error,
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
        i18n::select(config.language.code()).ok();

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
            prepare_page_id: prepare_id,
            subtitle_page_id,
            post_production_page_id,
            video_frame_rate: 24.0,

            prepare: prepare::Model::default(),
            subtitle: subtitle::Model::default(),
            post_production: post_production::Model::default(),
            toasts: widget::Toasts::new(Message::CloseToast),
            errors: Vec::new(),
        };

        let command = app.update_title();

        (app, Task::batch([command]))
    }

    fn header_start(&self) -> Vec<Element<'_, Self::Message>> {
        let menu_bar = menu::bar(vec![menu::Tree::with_children(
            menu::root(fl!("advanced")).apply(Element::from),
            menu::items(
                &self.key_binds,
                vec![
                    menu::Item::Button(fl!("settings"), None, MenuAction::Settings),
                    menu::Item::Button(fl!("about"), None, MenuAction::About),
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

        let build_dialog = |title, element, close_msg| {
            widget::dialog()
                .title(title)
                .control(widget::scrollable(element).height(Length::Fill))
                .primary_action({
                    widget::button::icon(icon::from_name("window-close-symbolic"))
                        .class(cosmic::theme::Button::Destructive)
                        .on_press(close_msg)
                })
                .width(Length::Fill)
                .apply(widget::container)
                .center(700)
                .apply(widget::container)
                .center(Length::Fill)
                .style(|_| widget::container::background(iced::Color::from_rgba(0., 0., 0., 0.45)))
                .apply(Element::from)
        };

        Some(match self.context_page {
            ContextPage::About => {
                let about = widget::about(&self.about, |url| Message::LaunchUrl(url.to_string()));
                build_dialog(
                    fl!("about"),
                    about,
                    Message::ToggleContextPage(ContextPage::About),
                )
            }

            ContextPage::Settings => build_dialog(
                fl!("settings"),
                self.settings_view(),
                Message::ToggleContextPage(ContextPage::Settings),
            ),
            ContextPage::Error => build_dialog(
                fl!("errors"),
                self.error_view(),
                Message::ToggleContextPage(ContextPage::Error),
            ),
        })
    }

    fn nav_view(&self, id: nav_bar::Id) -> Element<'_, Self::Message> {
        self.page_view(id)
    }
    fn view(&self) -> Element<'_, Self::Message> {
        self.page_view(self.nav.active())
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
                            emitter.send(Message::WatchTick(time)).await.ok();
                            time += 1;
                        }
                    },
                )
            }));
        }

        // state mangement?
        match self.nav.active_data::<Page>() {
            Some(Page::Prepare) => {
                subscriptions.push(self.prepare.subscription().map(Message::Prepare));
            }
            Some(Page::Subtitle) => {}
            Some(Page::PostProduction) => {
                subscriptions.push(
                    self.post_production
                        .subscription()
                        .map(Message::PostProduction),
                );
            }
            None => {}
        }

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
                    return log!("failed to save configuration: {e}");
                }
                Task::none()
            }
            Message::SetOcrModel(model) => {
                if model == self.config.ocr_model {
                    return Task::none();
                }

                if let Err(error) = self.config.set_ocr_model(&self.config_handler, model) {
                    return log!("failed to save configuration: {error}");
                }

                Task::none()
            }
            Message::PickCustomOcr => Task::perform(
                async {
                    AsyncFileDialog::new()
                        .add_filter("Dynamic library", &["so", "dylib", "dll"])
                        .pick_file()
                        .await
                        .map(|file| file.path().to_path_buf())
                },
                Message::CustomOcrPicked,
            )
            .map(Into::into),
            Message::CustomOcrPicked(Some(path)) => {
                match crate::ocr::plugin_loader::DynamicLibrary::new(path) {
                    Ok(library) if !self.config.custom_ocrs.contains(&library) => {
                        let mut custom_ocrs = self.config.custom_ocrs.clone();
                        custom_ocrs.push(library);

                        if let Err(error) = self
                            .config
                            .set_custom_ocrs(&self.config_handler, custom_ocrs)
                        {
                            return log!("failed to save custom OCR library: {error}");
                        }
                    }
                    Ok(_) => {}
                    Err(error) => return log!("failed to add custom OCR library: {error:#}"),
                }

                Task::none()
            }
            Message::CustomOcrPicked(None) => Task::none(),
            Message::RemoveCustomOcr(library) => {
                self.config.custom_ocrs.retain(|item| item != &library);

                if self.config.ocr_model == OcrModel::Custom(library) {
                    self.config.ocr_model = OcrModel::default();
                }

                if let Err(error) = self.config.write_entry(&self.config_handler) {
                    return log!("failed to remove custom OCR library: {error}");
                }

                Task::none()
            }
            Message::SetSubtitleDetector(detector) => {
                if let Err(error) = self
                    .config
                    .set_subtitle_detector(&self.config_handler, detector)
                {
                    return log!("failed to save configuration: {error}");
                }
                Task::none()
            }
            Message::SetNativeSearchParams(params) => {
                if let Err(error) = self
                    .config
                    .set_native_search_params(&self.config_handler, params)
                {
                    return log!("failed to save configuration: {error}");
                }
                Task::none()
            }
            Message::SetPostOcrProcessing(enabled) => {
                if let Err(error) = self
                    .config
                    .set_post_ocr_processing(&self.config_handler, enabled)
                {
                    return log!("failed to save configuration: {error}");
                }
                Task::none()
            }
            Message::SetProcessingResolution(resolution) => {
                if let Err(error) = self
                    .config
                    .set_processing_resolution(&self.config_handler, resolution)
                {
                    return log!("failed to save configuration: {error}");
                }
                Task::none()
            }
            Message::SetLanguage(language) => {
                if language == self.config.language {
                    return Task::none();
                }

                if let Err(error) = i18n::select(language.code()) {
                    return log!("failed to select language: {error}");
                }

                if let Err(error) = self.config.set_language(&self.config_handler, language) {
                    return log!("failed to save language preference: {error}");
                }

                self.nav.text_set(self.prepare_page_id, fl!("page-prepare"));
                self.nav
                    .text_set(self.subtitle_page_id, fl!("page-subtitle"));
                self.nav
                    .text_set(self.post_production_page_id, fl!("page-post"));
                self.post_production.refresh_language();
                self.about = self
                    .about
                    .clone()
                    .name(fl!("app-title"))
                    .links([(fl!("repository"), REPOSITORY)]);

                self.update_title()
            }
            Message::LaunchUrl(url) => {
                if let Err(err) = open::that_detached(&url) {
                    return log!("failed to open {url:?}: {err}");
                }
                Task::none()
            }
            Message::Prepare(msg) => {
                let event = self.prepare.update(msg);

                match event {
                    prepare::Event::StartSubtitleSearch(path, selection) => {
                        self.subtitle.start_search(path, selection, &self.config);
                        self.nav.activate(self.subtitle_page_id);
                        self.update_title()
                    }
                    prepare::Event::Run(task) => task.map(Message::Prepare).map(Into::into),
                    prepare::Event::CopySelectionDimensions(dimensions) => {
                        let copy = iced::clipboard::write(dimensions.clone());
                        let toast = self
                            .toasts
                            .push(widget::Toast::new(format!("Copied {dimensions}")))
                            .map(Into::into);
                        Task::batch([copy, toast])
                    }
                    prepare::Event::Error(error) => log!(error),
                    prepare::Event::None => Task::none(),
                }
            }
            Message::Subtitle(msg) => {
                let event = self.subtitle.update(msg, &self.config);

                match event {
                    subtitle::Event::GoToPostProduction => {
                        self.post_production.sync(
                            self.prepare.video_path.as_ref(),
                            &self.subtitle.results,
                            &self.config,
                        );
                        self.nav.activate(self.post_production_page_id);
                        self.update_title()
                    }
                    subtitle::Event::SyncWithPostProduction => {
                        self.post_production.sync(
                            self.prepare.video_path.as_ref(),
                            &self.subtitle.results,
                            &self.config,
                        );
                        Task::none()
                    }
                    subtitle::Event::Run(task) => task.map(Message::Subtitle).map(Into::into),
                    subtitle::Event::Error(error) => log!(error),
                    subtitle::Event::None => Task::none(),
                }
            }
            Message::PostProduction(msg) => match self.post_production.update(msg, &self.config) {
                post_production::Event::Run(task) => {
                    task.map(Message::PostProduction).map(Into::into)
                }
                post_production::Event::Toast(message) => self
                    .toasts
                    .push(widget::Toast::new(message))
                    .map(Into::into),
                post_production::Event::Error(error) => log!(error),
            },
            Message::CloseToast(id) => {
                self.toasts.remove(id);
                Task::none()
            }
            Message::ErrorReported(report) => {
                self.errors.push(report);
                self.context_page = ContextPage::Error;
                self.core.window.show_context = true;
                Task::none()
            }
        }
    }

    fn on_nav_select(&mut self, id: nav_bar::Id) -> Task<cosmic::Action<Self::Message>> {
        if id == self.post_production_page_id {
            self.post_production.sync(
                self.prepare.video_path.as_ref(),
                &self.subtitle.results,
                &self.config,
            );
        }
        self.nav.activate(id);
        self.update_title()
    }
}

impl AppModel {
    fn page_view(&self, id: nav_bar::Id) -> Element<'_, Message> {
        let space_s = cosmic::theme::spacing().space_s;
        let page = self.nav.data(id).unwrap();

        let content: Element<_> = match page {
            Page::Prepare => self.prepare.view().map(Message::Prepare),
            Page::Subtitle => {
                let video_duration = self
                    .prepare
                    .video_controller
                    .as_ref()
                    .map(|x| x.inner.info.video_time)
                    .unwrap_or_default();
                self.subtitle.view(video_duration).map(Message::Subtitle)
            }
            Page::PostProduction => self
                .post_production
                .view(&self.subtitle, self.prepare.video_path.as_ref())
                .map(Message::PostProduction),
        };

        let header = widget::column![
            widget::text::title1(page.to_string()),
            widget::text(page.details())
        ]
        .spacing(cosmic::theme::spacing().space_xxs);

        let content = widget::container(widget::column!(header, content).spacing(space_s))
            .width(Length::Fill)
            .height(Length::Fill)
            .apply(widget::container)
            .width(Length::Fill)
            .padding([0, 50])
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center);

        widget::toaster(&self.toasts, content).into()
    }
    fn error_view(&self) -> Element<'_, Message> {
        use std::fmt::Write;
        let spacing = cosmic::theme::spacing();
        let errors = self
            .errors
            .iter()
            .flat_map(|report| report.chain())
            .zip(1..)
            .fold(String::new(), |mut acc, (x, u)| {
                writeln!(acc, "{u}: {x}").ok();
                acc
            });

        widget::text(errors)
            .selectable()
            .apply(widget::container)
            .class(cosmic::theme::Container::Card)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(spacing.space_l)
            .into()
    }

    fn settings_view(&self) -> Element<'_, Message> {
        let all = OcrModel::all(&self.config);
        let labels = OcrModel::labels(&self.config);
        let resolution_labels = ProcessingResolution::labels();
        let detector_labels = SubtitleDetector::labels();
        let language_labels = Language::labels();

        let selected_ocr_index = all.iter().position(|model| *model == self.config.ocr_model);
        let selected_detector_index = SubtitleDetector::ALL
            .iter()
            .position(|detector| *detector == self.config.subtitle_detector);
        let selected_resolution_index = ProcessingResolution::ALL
            .iter()
            .position(|resolution| *resolution == self.config.processing_resolution);
        let selected_language_index = Language::ALL
            .iter()
            .position(|language| *language == self.config.language);
        let native = self.config.native_search_params;

        let spacing = cosmic::theme::spacing();
        let selected_custom_ocr = match &self.config.ocr_model {
            OcrModel::Custom(library) => Some(library),
            OcrModel::PaddleOcr(_) => None,
        };

        let ocr_model_picker = widget::row![
            widget::dropdown(labels, selected_ocr_index, move |index| {
                Message::SetOcrModel(all[index].clone())
            })
            .width(Length::Fill)
            .gap(f32::from(spacing.space_m)),
            widget::button::icon(icon::from_name("list-add-symbolic"))
                .tooltip(fl!("add-custom-ocr"))
                .on_press(Message::PickCustomOcr),
        ]
        .push_maybe(selected_custom_ocr.map(|x| {
            widget::button::icon(icon::from_name("edit-delete-symbolic"))
                .tooltip(fl!("remove-custom-ocr"))
                .on_press(Message::RemoveCustomOcr(x.clone()))
                .class(cosmic::theme::Button::Destructive)
        }))
        .align_y(Alignment::Center)
        .spacing(spacing.space_s)
        .width(Length::Fill);

        widget::settings::view_column(vec![
            widget::settings::section()
                .title(fl!("internationalization"))
                .add(widget::settings::item(
                    fl!("language"),
                    widget::dropdown(language_labels, selected_language_index, |index| {
                        Message::SetLanguage(Language::ALL[index])
                    }),
                ))
                .into(),
            widget::settings::section()
                .title(fl!("text-recognition"))
                .add(widget::settings::item(fl!("ocr-model"), ocr_model_picker))
                .add(
                    widget::settings::item::builder(fl!("post-ocr-result-processing"))
                        .description(fl!("merge-adjacent-detections"))
                        .toggler(
                            self.config.post_ocr_processing,
                            Message::SetPostOcrProcessing,
                        ),
                )
                .into(),
            widget::settings::section()
                .title(fl!("subtitle-detection"))
                .add(widget::settings::item(
                    fl!("processing-resolution"),
                    widget::dropdown(resolution_labels, selected_resolution_index, |index| {
                        Message::SetProcessingResolution(ProcessingResolution::ALL[index])
                    })
                    .gap(f32::from(spacing.space_m)),
                ))
                .add(widget::settings::item(
                    fl!("implementation"),
                    widget::dropdown(detector_labels, selected_detector_index, |index| {
                        Message::SetSubtitleDetector(SubtitleDetector::ALL[index])
                    })
                    .gap(f32::from(spacing.space_m)),
                ))
                .into(),
        ])
        .push_maybe(
            widget::settings::section()
                .title(fl!("original-cpp-parameters"))
                .add(
                    widget::settings::item::builder(fl!("ocr-image-cleanup"))
                        .description(fl!("run-find-text-lines"))
                        .toggler(native.apply_ocr_image_cleanup, move |enabled| {
                            let mut params = native;
                            params.apply_ocr_image_cleanup = enabled;
                            Message::SetNativeSearchParams(params)
                        }),
                )
                .add(widget::settings::item(
                    fl!("worker-threads"),
                    widget::spin_button(
                        native.threads.to_string(),
                        fl!("worker-threads"),
                        native.threads,
                        1,
                        1,
                        256,
                        move |value| {
                            let mut params = native;
                            params.threads = value;
                            Message::SetNativeSearchParams(params)
                        },
                    ),
                ))
                .add(widget::settings::item(
                    fl!("minimum-subtitle-frames"),
                    widget::spin_button(
                        native.min_subtitle_frames.to_string(),
                        fl!("minimum-subtitle-frames"),
                        native.min_subtitle_frames,
                        1,
                        1,
                        1000,
                        move |value| {
                            let mut params = native;
                            params.min_subtitle_frames = value;
                            Message::SetNativeSearchParams(params)
                        },
                    ),
                ))
                .add(widget::settings::item(
                    fl!("text-percentage"),
                    widget::spin_button(
                        format!("{:.3}", native.text_percent),
                        fl!("text-percentage"),
                        native.text_percent,
                        0.01,
                        0.0,
                        1.0,
                        move |value| {
                            let mut params = native;
                            params.text_percent = value;
                            Message::SetNativeSearchParams(params)
                        },
                    ),
                ))
                .add(widget::settings::item(
                    fl!("minimum-text-length"),
                    widget::spin_button(
                        format!("{:.3}", native.min_text_length),
                        fl!("minimum-text-length"),
                        native.min_text_length,
                        0.001,
                        0.0,
                        1.0,
                        move |value| {
                            let mut params = native;
                            params.min_text_length = value;
                            Message::SetNativeSearchParams(params)
                        },
                    ),
                ))
                .add(widget::settings::item(
                    fl!("vertical-edge-line-error"),
                    widget::spin_button(
                        format!("{:.2}", native.vertical_edges_line_error),
                        fl!("vertical-edge-line-error"),
                        native.vertical_edges_line_error,
                        0.05,
                        0.0,
                        1.0,
                        move |value| {
                            let mut params = native;
                            params.vertical_edges_line_error = value;
                            Message::SetNativeSearchParams(params)
                        },
                    ),
                ))
                .add(widget::settings::item(
                    fl!("ila-points-line-error"),
                    widget::spin_button(
                        format!("{:.2}", native.ila_points_line_error),
                        fl!("ila-points-line-error"),
                        native.ila_points_line_error,
                        0.05,
                        0.0,
                        1.0,
                        move |value| {
                            let mut params = native;
                            params.ila_points_line_error = value;
                            Message::SetNativeSearchParams(params)
                        },
                    ),
                ))
                .add(widget::settings::item(
                    fl!("maximum-frame-gap-down"),
                    widget::spin_button(
                        native.max_frame_gap_down.to_string(),
                        fl!("maximum-frame-gap-down"),
                        native.max_frame_gap_down,
                        1,
                        0,
                        1000,
                        move |value| {
                            let mut params = native;
                            params.max_frame_gap_down = value;
                            Message::SetNativeSearchParams(params)
                        },
                    ),
                ))
                .add(widget::settings::item(
                    fl!("maximum-frame-gap-up"),
                    widget::spin_button(
                        native.max_frame_gap_up.to_string(),
                        fl!("maximum-frame-gap-up"),
                        native.max_frame_gap_up,
                        1,
                        0,
                        1000,
                        move |value| {
                            let mut params = native;
                            params.max_frame_gap_up = value;
                            Message::SetNativeSearchParams(params)
                        },
                    ),
                ))
                .add(
                    widget::settings::item::builder(fl!("use-isa-images")).toggler(
                        native.use_isa_images,
                        move |enabled| {
                            let mut params = native;
                            params.use_isa_images = enabled;
                            Message::SetNativeSearchParams(params)
                        },
                    ),
                )
                .add(
                    widget::settings::item::builder(fl!("use-ila-images")).toggler(
                        native.use_ila_images,
                        move |enabled| {
                            let mut params = native;
                            params.use_ila_images = enabled;
                            Message::SetNativeSearchParams(params)
                        },
                    ),
                )
                .add(
                    widget::settings::item::builder(fl!("replace-isa-with-filtered-image"))
                        .toggler(native.replace_isa_with_filtered, move |enabled| {
                            let mut params = native;
                            params.replace_isa_with_filtered = enabled;
                            Message::SetNativeSearchParams(params)
                        }),
                )
                .apply(|x| {
                    (self.config.subtitle_detector == SubtitleDetector::OriginalCpp).then_some(x)
                }),
        )
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
