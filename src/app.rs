// SPDX-License-Identifier: MPL-2.0

pub mod post_production;
pub mod prepare;
pub mod selection_canvas;
pub mod subtitle;

use crate::config::{Config, Language, ProcessingResolution, SubtitleDetector};
use crate::ocr::OcrModel;
pub(crate) use crate::video_player::{
    self, InnerPlayer, VideoPlayerController, create_video_player,
};
use crate::{fl, i18n};
use cosmic_config::{self, CosmicConfigEntry};
pub(crate) use iced::{Alignment, Element, Length, Subscription, Task};
use std::{sync::Arc, time::Duration};
use vse_ui::shell;
pub(crate) use vse_ui::widget;
pub(crate) use vse_ui::widget::icon;

const APP_ID: &str = "dev.justsimplykyle.videosubextract";

pub fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    )
}

pub struct AppModel {
    active_page: Page,
    navigation_open: bool,
    settings_open: bool,
    next_toast_id: u64,
    toasts: Vec<(u64, String)>,
    config_handler: cosmic_config::Config,
    config: Config,
    video_frame_rate: f64,
    prepare: prepare::Model,
    subtitle: subtitle::Model,
    post_production: post_production::Model,
    errors: Vec<Arc<eyre::Report>>,
}

#[derive(Debug, Clone)]
pub enum Message {
    SelectPage(Page),
    ToggleNavigation,
    ToggleSettings,
    CloseToast(u64),
    SetOcrModel(OcrModel),
    SetSubtitleDetector(SubtitleDetector),
    SetPostOcrProcessing(bool),
    SetProcessingResolution(ProcessingResolution),
    SetLanguage(Language),
    Prepare(prepare::Message),
    Subtitle(subtitle::Message),
    PostProduction(post_production::Message),
    ErrorReported(Arc<eyre::Report>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Page {
    Prepare,
    Subtitle,
    PostProduction,
}
impl Page {
    fn label(self) -> String {
        match self {
            Self::Prepare => fl!("page-prepare"),
            Self::Subtitle => fl!("page-subtitle"),
            Self::PostProduction => fl!("page-post"),
        }
    }
    fn details(self) -> String {
        match self {
            Self::Prepare => fl!("page-prepare-details"),
            Self::Subtitle => fl!("page-subtitle-details"),
            Self::PostProduction => fl!("page-post-details"),
        }
    }
}

impl AppModel {
    fn settings_view(&self) -> Element<'_, Message> {
        let ocr_models = OcrModel::all(&self.config);
        let selected_ocr = ocr_models
            .iter()
            .position(|model| model == &self.config.ocr_model);
        let selected_language = Language::ALL
            .iter()
            .position(|value| value == &self.config.language);
        let selected_detector = SubtitleDetector::ALL
            .iter()
            .position(|value| value == &self.config.subtitle_detector);
        let selected_resolution = ProcessingResolution::ALL
            .iter()
            .position(|value| value == &self.config.processing_resolution);

        widget::scrollable(widget::settings::view_column(vec![
            widget::settings::section()
                .title(fl!("internationalization"))
                .add(widget::settings::item(
                    fl!("language"),
                    widget::dropdown(Language::labels(), selected_language, |index| {
                        Message::SetLanguage(Language::ALL[index])
                    }),
                ))
                .into(),
            widget::settings::section()
                .title(fl!("text-recognition"))
                .add(widget::settings::item(
                    fl!("ocr-model"),
                    widget::dropdown(OcrModel::labels(&self.config), selected_ocr, move |index| {
                        Message::SetOcrModel(ocr_models[index].clone())
                    }),
                ))
                .add(
                    widget::settings::togglable(fl!("post-ocr-result-processing"))
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
                    widget::dropdown(
                        ProcessingResolution::labels(),
                        selected_resolution,
                        |index| Message::SetProcessingResolution(ProcessingResolution::ALL[index]),
                    ),
                ))
                .add(widget::settings::item(
                    fl!("implementation"),
                    widget::dropdown(SubtitleDetector::labels(), selected_detector, |index| {
                        Message::SetSubtitleDetector(SubtitleDetector::ALL[index])
                    }),
                ))
                .into(),
        ]))
        .into()
    }

    pub fn boot() -> (Self, Task<Message>) {
        let config_handler = cosmic_config::Config::new(APP_ID, Config::VERSION)
            .expect("failed to load configuration");
        let config = Config::get_entry(&config_handler).unwrap_or_else(|(_, config)| config);
        i18n::select(config.language.code()).ok();
        (
            Self {
                active_page: Page::Prepare,
                navigation_open: true,
                settings_open: false,
                next_toast_id: 0,
                toasts: Vec::new(),
                config_handler,
                config,
                video_frame_rate: 24.0,
                prepare: prepare::Model::default(),
                subtitle: subtitle::Model::default(),
                post_production: post_production::Model::default(),
                errors: Vec::new(),
            },
            Task::none(),
        )
    }

    pub fn title(&self) -> String {
        format!("{} — {}", fl!("app-title"), self.active_page.label())
    }

    pub fn theme(&self) -> iced::Theme {
        vse_ui::theme::iced_theme()
    }
    pub fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            self.subtitle
                .subscription(self.video_frame_rate)
                .map(Message::Subtitle),
        ];
        match self.active_page {
            Page::Prepare => subscriptions.push(self.prepare.subscription().map(Message::Prepare)),
            Page::PostProduction => subscriptions.push(
                self.post_production
                    .subscription()
                    .map(Message::PostProduction),
            ),
            Page::Subtitle => {}
        }
        Subscription::batch(subscriptions)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SelectPage(page) => {
                if page == Page::PostProduction {
                    self.post_production.sync(
                        self.prepare.video_path.as_ref(),
                        self.subtitle.results(),
                        subtitle::ResultsChanged::Full,
                        &self.config,
                    );
                }
                self.active_page = page;
                Task::none()
            }
            Message::ToggleNavigation => {
                self.navigation_open = !self.navigation_open;
                Task::none()
            }
            Message::ToggleSettings => {
                self.settings_open = !self.settings_open;
                Task::none()
            }
            Message::CloseToast(id) => {
                self.toasts.retain(|(toast_id, _)| *toast_id != id);
                Task::none()
            }
            Message::SetLanguage(value) => {
                let _ = i18n::select(value.code());
                let _ = self.config.set_language(&self.config_handler, value);
                self.post_production.refresh_language();
                Task::none()
            }
            Message::SetOcrModel(value) => {
                let _ = self.config.set_ocr_model(&self.config_handler, value);
                Task::none()
            }
            Message::SetSubtitleDetector(value) => {
                let _ = self
                    .config
                    .set_subtitle_detector(&self.config_handler, value);
                Task::none()
            }
            Message::SetPostOcrProcessing(value) => {
                let _ = self
                    .config
                    .set_post_ocr_processing(&self.config_handler, value);
                Task::none()
            }
            Message::SetProcessingResolution(value) => {
                let _ = self
                    .config
                    .set_processing_resolution(&self.config_handler, value);
                Task::none()
            }
            Message::Prepare(message) => match self.prepare.update(message) {
                prepare::Event::StartSubtitleSearch(path, selection) => {
                    self.subtitle.start_search(path, selection, &self.config);
                    self.active_page = Page::Subtitle;
                    Task::none()
                }
                prepare::Event::Run(task) => task.map(Message::Prepare),
                prepare::Event::CopySelectionDimensions(value) => {
                    iced::clipboard::write(value).discard()
                }
                prepare::Event::Error(error) => Task::done(Message::ErrorReported(Arc::new(error))),
                prepare::Event::None => Task::none(),
            },
            Message::Subtitle(message) => match self.subtitle.update(message, &self.config) {
                subtitle::Event::GoToPostProduction => {
                    self.post_production.sync(
                        self.prepare.video_path.as_ref(),
                        self.subtitle.results(),
                        subtitle::ResultsChanged::Full,
                        &self.config,
                    );
                    self.active_page = Page::PostProduction;
                    Task::none()
                }
                subtitle::Event::SyncWithPostProduction(changed) => {
                    self.post_production.sync(
                        self.prepare.video_path.as_ref(),
                        self.subtitle.results(),
                        changed,
                        &self.config,
                    );
                    Task::none()
                }
                subtitle::Event::None => Task::none(),
                subtitle::Event::Run(task) => task.map(Message::Subtitle),
                subtitle::Event::Error(error) => {
                    Task::done(Message::ErrorReported(Arc::new(error)))
                }
            },
            Message::PostProduction(message) => {
                match self.post_production.update(message, &self.config) {
                    post_production::Event::Run(task) => task.map(Message::PostProduction),
                    post_production::Event::Toast(message) => {
                        let id = self.next_toast_id;
                        self.next_toast_id = self.next_toast_id.wrapping_add(1);
                        self.toasts.push((id, message));
                        Task::none()
                    }
                    post_production::Event::Error(error) => {
                        Task::done(Message::ErrorReported(Arc::new(error)))
                    }
                }
            }
            Message::ErrorReported(error) => {
                self.errors.push(error);
                Task::none()
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let content = match self.active_page {
            Page::Prepare => self.prepare.view().map(Message::Prepare),
            Page::Subtitle => self
                .subtitle
                .view(
                    self.prepare
                        .video_controller
                        .as_ref()
                        .map(|controller| controller.inner.info.video_time)
                        .unwrap_or_default(),
                )
                .map(Message::Subtitle),
            Page::PostProduction => self
                .post_production
                .view(&self.subtitle, self.prepare.video_path.as_ref())
                .map(Message::PostProduction),
        };
        let active = self.active_page;
        let navigation = if self.navigation_open {
            vec![
                shell::NavigationItem {
                    label: Page::Prepare.label(),
                    selected: active == Page::Prepare,
                    on_press: Message::SelectPage(Page::Prepare),
                },
                shell::NavigationItem {
                    label: Page::Subtitle.label(),
                    selected: active == Page::Subtitle,
                    on_press: Message::SelectPage(Page::Subtitle),
                },
                shell::NavigationItem {
                    label: Page::PostProduction.label(),
                    selected: active == Page::PostProduction,
                    on_press: Message::SelectPage(Page::PostProduction),
                },
            ]
        } else {
            Vec::new()
        };
        let dialog = self.settings_open.then(|| {
            vse_ui::components::dialog(
                fl!("settings"),
                self.settings_view(),
                widget::button(icon::from_name("window-close-symbolic"))
                    .padding(4)
                    .style(vse_ui::theme::Button::Icon::style)
                    .on_press(Message::ToggleSettings),
            )
        });

        shell::Shell::new(active.label(), active.details(), content)
            .navigation(navigation)
            .header_controls(Message::ToggleNavigation, Message::ToggleSettings)
            .toasts(
                self.toasts
                    .iter()
                    .map(|(id, message)| (message.clone(), Message::CloseToast(*id)))
                    .collect(),
            )
            .dialog(dialog)
            .into()
    }
}
