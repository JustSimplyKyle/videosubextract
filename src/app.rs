// SPDX-License-Identifier: MPL-2.0

pub mod detection;
pub mod post_production;
pub mod prepare;
pub mod selection_canvas;
pub mod toast;
pub mod video_player_widget;

use crate::config::{Config, ExtractionTarget, Language};
pub(crate) use crate::video_player::{
    self, InnerPlayer, VideoPlayerController, create_video_player,
};
use crate::{fl, i18n};
use async_channel::{Receiver, Sender};
use cosmic_config::{self, CosmicConfigEntry};
use futures::{SinkExt, StreamExt};
use iced::alignment::Vertical;
use iced::window::{self, Action};
pub(crate) use iced::{Alignment, Length, Subscription, Task, Widget};
use std::sync::LazyLock;
use std::{sync::Arc, time::Duration};
pub(crate) use vse_ui::widget;
use vse_ui::widget::AnimatedDropdown;
pub(crate) use vse_ui::widget::icon;
use vse_ui::{Apply, icon_button, shell, theme};

trait Composition {
    type Message: 'static;
    type Event;
    type ViewContext<'a>
    where
        Self: 'a;
    type UpdateContext<'a>
    where
        Self: 'a;
    type SubscriptionContext<'a>
    where
        Self: 'a;

    fn view<'a>(&'a self, context: Self::ViewContext<'a>) -> impl Widget<Self::Message> + 'a;

    fn update(&mut self, message: Self::Message, context: Self::UpdateContext<'_>) -> Self::Event;

    fn subscription(&self, context: Self::SubscriptionContext<'_>) -> Subscription<Self::Message>;
}

pub trait ReportLike {
    fn err(error: eyre::Report) -> Self;
    fn none() -> Self;
}

#[macro_export]
macro_rules! impl_report_residual {
    ($ty:ty) => {
        impl<E> std::ops::FromResidual<Result<std::convert::Infallible, E>> for $ty
        where
            E: Into<eyre::Report>,
        {
            fn from_residual(residual: Result<std::convert::Infallible, E>) -> Self {
                match residual {
                    Err(error) => <Self as ReportLike>::err(error.into()),
                    Ok(never) => match never {},
                }
            }
        }
        impl std::ops::FromResidual<Option<std::convert::Infallible>> for $ty {
            fn from_residual(residual: Option<std::convert::Infallible>) -> Self {
                match residual {
                    None => <Self as ReportLike>::none(),
                    Some(never) => match never {},
                }
            }
        }
    };
}

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

pub static WARNING_CHANNEL: LazyLock<(Sender<String>, Receiver<String>)> =
    LazyLock::new(|| async_channel::bounded(2));

pub struct AppModel {
    active_page: Page,
    navigation_open: bool,
    dialog_page: DialogPage,
    dialog_open: bool,
    toasts: toast::Toasts,
    config_handler: Arc<cosmic_config::Config>,
    config: Config,
    video_frame_rate: f64,
    prepare: prepare::Model,
    subtitle: detection::Model,
    chapters: detection::Model,
    post_production: post_production::Model,
    errors: Vec<Arc<eyre::Report>>,
}

#[derive(Debug, Clone)]
pub enum Message {
    SelectPage(Page),
    ShowDialog(DialogPage),
    CloseDialog,
    ToggleNavigation,
    PushToast(String),
    CloseToast(toast::ToastId),
    SetLanguage(Language),
    Prepare(prepare::Message),
    /// A message for the results page of that target.
    Results(ExtractionTarget, detection::Message),
    PostProduction(post_production::Message),
    ErrorReported(Arc<eyre::Report>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Page {
    Prepare,
    Subtitle,
    Chapters,
    PostProduction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DialogPage {
    Settings,
    PrepareSettings,
    Error,
}

impl Page {
    const ALL: [Self; 4] = [
        Self::Prepare,
        Self::Subtitle,
        Self::Chapters,
        Self::PostProduction,
    ];

    const fn target(self) -> Option<ExtractionTarget> {
        match self {
            Self::Subtitle => Some(ExtractionTarget::Subtitles),
            Self::Chapters => Some(ExtractionTarget::Chapters),
            Self::Prepare | Self::PostProduction => None,
        }
    }

    const fn results(target: ExtractionTarget) -> Self {
        match target {
            ExtractionTarget::Subtitles => Self::Subtitle,
            ExtractionTarget::Chapters => Self::Chapters,
        }
    }

    fn label(self) -> String {
        match self {
            Self::Prepare => fl!("page-prepare"),
            Self::Subtitle => fl!("page-subtitle"),
            Self::Chapters => fl!("page-chapters"),
            Self::PostProduction => fl!("page-post"),
        }
    }
    /// `prepare_target` is the configured target, which Prepare's hint describes.
    fn details(self, prepare_target: ExtractionTarget) -> String {
        match (self, prepare_target) {
            (Self::Prepare, ExtractionTarget::Subtitles) => fl!("page-prepare-details"),
            (Self::Prepare, ExtractionTarget::Chapters) => fl!("page-prepare-details-chapters"),
            (Self::Subtitle, _) => fl!("page-subtitle-details"),
            (Self::Chapters, _) => fl!("page-chapters-details"),
            (Self::PostProduction, _) => fl!("page-post-details"),
        }
    }
}

impl AppModel {
    const fn results(&self, target: ExtractionTarget) -> &detection::Model {
        match target {
            ExtractionTarget::Subtitles => &self.subtitle,
            ExtractionTarget::Chapters => &self.chapters,
        }
    }

    /// The results model for `target`, alongside the config it is updated with.
    const fn results_mut(&mut self, target: ExtractionTarget) -> (&mut detection::Model, &Config) {
        let model = match target {
            ExtractionTarget::Subtitles => &mut self.subtitle,
            ExtractionTarget::Chapters => &mut self.chapters,
        };
        (model, &self.config)
    }

    fn sync_post_production(
        &mut self,
        target: ExtractionTarget,
        changed: detection::ResultsChanged,
    ) {
        // Field access rather than `self.results`, which would borrow all of `self`.
        let results = match target {
            ExtractionTarget::Subtitles => &self.subtitle,
            ExtractionTarget::Chapters => &self.chapters,
        };
        self.post_production.sync(
            post_production::SyncSource {
                video_path: self.prepare.video_path(),
                video_duration: self.prepare.video_duration(),
                target,
                chapter_params: results.chapter_params(),
                results: results.results(),
            },
            changed,
            &self.config,
        );
    }

    fn sync_post_production_fully(&mut self) {
        for target in ExtractionTarget::ALL {
            self.sync_post_production(target, detection::ResultsChanged::Full);
        }
    }

    fn handle_results_event(
        &mut self,
        target: ExtractionTarget,
        event: detection::Event,
    ) -> Task<Message> {
        match event {
            detection::Event::GoToPostProduction => {
                self.sync_post_production_fully();
                self.active_page = Page::PostProduction;
                Task::none()
            }
            detection::Event::SyncWithPostProduction(changed) => {
                self.sync_post_production(target, changed);
                Task::none()
            }
            detection::Event::None => Task::none(),
            detection::Event::Run(task) => {
                task.map(move |message| Message::Results(target, message))
            }
            detection::Event::Error(error) => Task::done(Message::ErrorReported(Arc::new(error))),
        }
    }

    fn settings_view(&self) -> impl Widget<Message> + '_ {
        let selected_language = Language::ALL
            .iter()
            .position(|value| value == &self.config.language);

        widget::scrollable(widget::settings::view_column(vec![
            widget::settings::section()
                .title(fl!("internationalization"))
                .add(widget::settings::item(
                    fl!("language"),
                    widget::dropdown(Language::labels(), selected_language, |index| {
                        Message::SetLanguage(Language::ALL[index])
                    })
                    .apply(AnimatedDropdown::from),
                ))
                .boxed(),
        ]))
        .height(Length::Fill)
    }

    fn error_view(&self) -> impl Widget<Message> + '_ {
        use std::fmt::Write;

        let errors = self
            .errors
            .iter()
            .flat_map(|report| report.chain())
            .enumerate()
            .fold(String::new(), |mut output, (index, error)| {
                let _ = writeln!(output, "{}: {}", index + 1, error);
                output
            });

        widget::scrollable(
            widget::container(widget::text(errors))
                .width(Length::Fill)
                .padding(vse_ui::theme::spacing().space_l)
                .style(vse_ui::theme::container::card),
        )
        .height(Length::Fill)
    }

    pub fn boot() -> (Self, Task<Message>) {
        let config_handler = cosmic_config::Config::new(APP_ID, Config::VERSION)
            .expect("failed to load configuration");
        let config = Config::get_entry(&config_handler).unwrap_or_else(|(_, config)| config);
        i18n::select(config.language.code()).ok();
        let prepare = prepare::Model::new(&config);
        (
            Self {
                active_page: Page::Prepare,
                navigation_open: true,
                dialog_page: DialogPage::Settings,
                dialog_open: false,
                toasts: toast::Toasts::default(),
                config_handler: Arc::new(config_handler),
                config,
                video_frame_rate: 24.0,
                prepare,
                subtitle: detection::Model::new(ExtractionTarget::Subtitles),
                chapters: detection::Model::new(ExtractionTarget::Chapters),
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
            self.prepare
                .settings_subscription(&self.config_handler)
                .map(Message::Prepare),
            Self::monitor_warning_channel(),
        ];
        // Searches keep running in the background whichever page is showing.
        for target in ExtractionTarget::ALL {
            let active = self.active_page.target() == Some(target);
            subscriptions.push(
                self.results(target)
                    .subscription((self.video_frame_rate, active))
                    // Subscription closures can't capture; `with` carries the target instead.
                    .with(target)
                    .map(|(target, message)| Message::Results(target, message)),
            );
        }
        match self.active_page {
            Page::Prepare => {
                subscriptions.push(self.prepare.subscription(()).map(Message::Prepare));
            }
            Page::PostProduction => subscriptions.push(
                self.post_production
                    .subscription(())
                    .map(Message::PostProduction),
            ),
            Page::Subtitle | Page::Chapters => {}
        }
        Subscription::batch(subscriptions)
    }

    fn monitor_warning_channel() -> Subscription<Message> {
        let stream = || {
            iced::stream::channel(
                2,
                async move |mut output: futures::channel::mpsc::Sender<Message>| {
                    while let Ok(x) = WARNING_CHANNEL.1.recv().await {
                        output.send(Message::PushToast(x)).await.ok();
                    }
                },
            )
        };
        Subscription::run(stream)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SelectPage(page) => {
                if page == Page::PostProduction {
                    self.sync_post_production_fully();
                }
                self.active_page = page;
                Task::none()
            }
            Message::ToggleNavigation => {
                self.navigation_open = !self.navigation_open;
                Task::none()
            }
            Message::ShowDialog(page) => {
                self.dialog_page = page;
                self.dialog_open = true;
                Task::none()
            }
            Message::CloseDialog => {
                self.dialog_open = false;
                Task::none()
            }
            Message::CloseToast(id) => {
                self.toasts.close(id);
                Task::none()
            }
            Message::SetLanguage(value) => {
                let _ = i18n::select(value.code());
                let _ = self.config.set_language(&self.config_handler, value);
                Task::none()
            }
            Message::Prepare(message) => match self.prepare.update(message, &mut self.config) {
                prepare::Event::StartSubtitleSearch(path, selection) => {
                    let target = self.config.extraction_target;
                    let (results, config) = self.results_mut(target);
                    results.start_search(path, selection, config);
                    self.active_page = Page::results(target);
                    Task::none()
                }
                prepare::Event::Run(task) => task.map(Message::Prepare),
                prepare::Event::CopySelectionDimensions(value) => {
                    let copy = iced::clipboard::write(value.clone());
                    let update =
                        self.update(Message::PushToast(format!("{value} copied to clipboard")));
                    Task::batch([copy.discard(), update])
                }
                prepare::Event::OpenSettings => {
                    self.dialog_page = DialogPage::PrepareSettings;
                    self.dialog_open = true;
                    Task::none()
                }
                prepare::Event::Error(error) => Task::done(Message::ErrorReported(Arc::new(error))),
                prepare::Event::None => Task::none(),
            },
            Message::Results(target, message) => {
                let (results, config) = self.results_mut(target);
                let event = results.update(message, config);
                self.handle_results_event(target, event)
            }
            Message::PostProduction(message) => {
                match self.post_production.update(message, &self.config) {
                    post_production::Event::Run(task) => task.map(Message::PostProduction),
                    post_production::Event::Toast(message) => {
                        self.toasts.push(message, Message::CloseToast)
                    }
                    post_production::Event::Error(error) => {
                        Task::done(Message::ErrorReported(Arc::new(error)))
                    }
                    post_production::Event::None => Task::none(),
                }
            }
            Message::ErrorReported(error) => {
                self.errors.push(error);
                self.dialog_page = DialogPage::Error;
                self.dialog_open = true;
                Task::none()
            }
            Message::PushToast(msg) => self.toasts.push(msg, Message::CloseToast),
        }
    }

    pub fn view(&self) -> impl Widget<Message> + '_ {
        let content = match self.active_page {
            Page::Prepare => self
                .prepare
                .view(&self.config)
                .map(Message::Prepare)
                .boxed(),
            Page::Subtitle | Page::Chapters => {
                let target = self
                    .active_page
                    .target()
                    .expect("results pages have a target");
                self.results(target)
                    .view(self.prepare.video_duration())
                    .map(move |message| Message::Results(target, message))
                    .boxed()
            }
            Page::PostProduction => self
                .post_production
                .view(post_production::ViewArgs {
                    subtitles: &self.subtitle,
                    chapters: &self.chapters,
                    video_path: self.prepare.video_path(),
                })
                .map(Message::PostProduction)
                .boxed(),
        };
        let active = self.active_page;
        let navigation = if self.navigation_open {
            Page::ALL
                .map(|page| shell::NavigationItem {
                    label: page.label(),
                    selected: active == page,
                    on_press: Message::SelectPage(page),
                })
                .into()
        } else {
            Vec::new()
        };
        let dialog = widget::dialog(self.dialog_open, || {
            let (title, content) = match self.dialog_page {
                DialogPage::Settings => (fl!("settings"), self.settings_view().boxed()),
                DialogPage::PrepareSettings => (
                    fl!("settings"),
                    self.prepare
                        .settings_view(&self.config)
                        .map(Message::Prepare)
                        .boxed(),
                ),
                DialogPage::Error => (fl!("errors"), self.error_view().boxed()),
            };

            vse_ui::components::dialog(
                title,
                content,
                icon_button!("window-close-symbolic")
                    .padding(theme::spacing().space_xxs)
                    .style(vse_ui::theme::button::icon)
                    .on_press(Message::CloseDialog),
            )
        });

        let header = widget::column![
            widget::text(active.label()).size(36),
            widget::text(active.details(self.config.extraction_target)).size(14),
        ]
        .spacing(theme::spacing().space_s);

        let header = widget::row![
            header,
            widget::space().width(Length::Fill),
            (self.active_page == Page::Prepare).then(|| self
                .prepare
                .title_actions(&self.config)
                .map(Message::Prepare)),
        ]
        .align_y(Vertical::Center);

        let page = widget::column! {
            header,
            content,
        }
        .spacing(theme::spacing().space_m);

        shell::Shell::new(page)
            .navigation(navigation)
            .header_controls(
                Message::ToggleNavigation,
                Message::ShowDialog(DialogPage::Settings),
            )
            .toasts(
                self.toasts
                    .iter()
                    .map(|(id, message)| (message.clone(), Message::CloseToast(*id)))
                    .collect(),
            )
            .dialog(dialog)
    }
}
