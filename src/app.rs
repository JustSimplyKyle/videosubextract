// SPDX-License-Identifier: MPL-2.0

pub mod post_production;
pub mod prepare;
pub mod selection_canvas;
pub mod subtitle;
pub mod video_player_widget;

use crate::config::{Config, Language};
pub(crate) use crate::video_player::{
    self, InnerPlayer, VideoPlayerController, create_video_player,
};
use crate::{fl, i18n};
use async_channel::{Receiver, Sender};
use cosmic_config::{self, CosmicConfigEntry};
use futures::{SinkExt, StreamExt};
use iced::alignment::Vertical;
use iced::window::{self, Action};
pub(crate) use iced::{Alignment, Element, Length, Subscription, Task};
use std::sync::LazyLock;
use std::{sync::Arc, time::Duration};
pub(crate) use vse_ui::widget;
pub(crate) use vse_ui::widget::icon;
use vse_ui::{Apply, shell, theme};

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

    fn view<'a>(&'a self, context: Self::ViewContext<'a>) -> Element<'a, Self::Message>;

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
    dialog_page: Option<DialogPage>,
    next_toast_id: u64,
    toasts: Vec<(u64, String)>,
    config_handler: Arc<cosmic_config::Config>,
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
    SelectDialogPage(Option<DialogPage>),
    ToggleNavigation,
    PushToast(String),
    CloseToast(u64),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DialogPage {
    Settings,
    PrepareSettings,
    Error,
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
                    }),
                ))
                .into(),
        ]))
        .height(Length::Fill)
        .into()
    }

    fn error_view(&self) -> Element<'_, Message> {
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
        .into()
    }

    pub fn boot() -> (Self, Task<Message>) {
        let config_handler = cosmic_config::Config::new(APP_ID, Config::VERSION)
            .expect("failed to load configuration");
        let config = Config::get_entry(&config_handler).unwrap_or_else(|(_, config)| config);
        i18n::select(config.language.code()).ok();
        let prepare = prepare::Model::new(config.processing_resolution);
        (
            Self {
                active_page: Page::Prepare,
                navigation_open: true,
                dialog_page: None,
                next_toast_id: 0,
                toasts: Vec::new(),
                config_handler: Arc::new(config_handler),
                config,
                video_frame_rate: 24.0,
                prepare,
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
            self.prepare
                .settings_subscription(&self.config_handler)
                .map(Message::Prepare),
            self.subtitle
                .subscription(self.video_frame_rate)
                .map(Message::Subtitle),
            Self::monitor_warning_channel(),
        ];
        match self.active_page {
            Page::Prepare => {
                subscriptions.push(self.prepare.subscription(()).map(Message::Prepare));
            }
            Page::PostProduction => subscriptions.push(
                self.post_production
                    .subscription(())
                    .map(Message::PostProduction),
            ),
            Page::Subtitle => {
                // subtitles search should always run in the background regardless of current active page
            }
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
                    self.post_production.sync(
                        self.prepare.video_path(),
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
            Message::SelectDialogPage(page) => {
                self.dialog_page = page;
                Task::none()
            }
            Message::CloseToast(remove_id) => {
                self.toasts.retain(|(toast_id, _)| *toast_id != remove_id);
                Task::none()
            }
            Message::SetLanguage(value) => {
                let _ = i18n::select(value.code());
                let _ = self.config.set_language(&self.config_handler, value);
                Task::none()
            }
            Message::Prepare(message) => match self.prepare.update(message, &mut self.config) {
                prepare::Event::StartSubtitleSearch(path, selection) => {
                    self.subtitle.start_search(path, selection, &self.config);
                    self.active_page = Page::Subtitle;
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
                    self.dialog_page = Some(DialogPage::PrepareSettings);
                    Task::none()
                }
                prepare::Event::Error(error) => Task::done(Message::ErrorReported(Arc::new(error))),
                prepare::Event::None => Task::none(),
            },
            Message::Subtitle(message) => match self.subtitle.update(message, &self.config) {
                subtitle::Event::GoToPostProduction => {
                    self.post_production.sync(
                        self.prepare.video_path(),
                        self.subtitle.results(),
                        subtitle::ResultsChanged::Full,
                        &self.config,
                    );
                    self.active_page = Page::PostProduction;
                    Task::none()
                }
                subtitle::Event::SyncWithPostProduction(changed) => {
                    self.post_production.sync(
                        self.prepare.video_path(),
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
                    post_production::Event::None => Task::none(),
                }
            }
            Message::ErrorReported(error) => {
                self.errors.push(error);
                self.dialog_page = Some(DialogPage::Error);
                Task::none()
            }
            Message::PushToast(msg) => {
                let id = self.next_toast_id;
                self.next_toast_id = self.next_toast_id.wrapping_add(1);
                self.toasts.push((id, msg));
                Task::none()
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let content = match self.active_page {
            Page::Prepare => self.prepare.view(&self.config).map(Message::Prepare),
            Page::Subtitle => self
                .subtitle
                .view(
                    self.prepare
                        .video_player
                        .as_ref()
                        .map_or(Duration::ZERO, |player| {
                            player.controller().inner.info.video_time
                        }),
                )
                .map(Message::Subtitle),
            Page::PostProduction => self
                .post_production
                .view(post_production::ViewArgs {
                    subtitles: &self.subtitle,
                    video_path: self.prepare.video_path(),
                })
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
        let dialog = self.dialog_page.map(|page| {
            let (title, content) = match page {
                DialogPage::Settings => (fl!("settings"), self.settings_view()),
                DialogPage::PrepareSettings => (
                    fl!("settings"),
                    self.prepare
                        .settings_view(&self.config)
                        .map(Message::Prepare),
                ),
                DialogPage::Error => (fl!("errors"), self.error_view()),
            };

            vse_ui::components::dialog(
                title,
                content,
                widget::button(icon::from_name("window-close-symbolic"))
                    .padding(theme::spacing().space_xxs)
                    .style(vse_ui::theme::button::icon)
                    .on_press(Message::SelectDialogPage(None)),
            )
        });

        let header = widget::column![
            widget::text(active.label()).size(36),
            widget::text(active.details()).size(14),
        ]
        .spacing(theme::spacing().space_s);

        let header = widget::row![
            header,
            widget::space().width(Length::Fill),
            match self.active_page {
                Page::Prepare => {
                    self.prepare.title_actions().map(Message::Prepare)
                }
                _ => {
                    widget::space().apply(Element::from)
                }
            }
        ]
        .align_y(Vertical::Center)
        .apply(Element::from);

        let page = widget::column! {
            header,
            content,
        }
        .spacing(theme::spacing().space_m);

        shell::Shell::new(page)
            .navigation(navigation)
            .header_controls(
                Message::ToggleNavigation,
                Message::SelectDialogPage(Some(DialogPage::Settings)),
            )
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
