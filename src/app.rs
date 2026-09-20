// SPDX-License-Identifier: MPL-2.0

pub mod post_production;
pub mod prepare;
pub mod selection_canvas;
pub mod subtitle;

use crate::config::{Config, Language, ProcessingResolution, SubtitleDetector};
use crate::native_video_sub_finder::NativeSearchParams;
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
    dialog_page: Option<DialogPage>,
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
    SelectDialogPage(Option<DialogPage>),
    ToggleNavigation,
    CloseToast(u64),
    SetOcrModel(OcrModel),
    SetSubtitleDetector(SubtitleDetector),
    SetNativeSearchParams(NativeSearchParams),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DialogPage {
    Settings,
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
        let mut sections = vec![
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
        ];

        if self.config.subtitle_detector == SubtitleDetector::OriginalCpp {
            sections.push(self.original_cpp_settings());
        }

        widget::scrollable(widget::settings::view_column(sections)).into()
    }

    fn original_cpp_settings(&self) -> Element<'_, Message> {
        let native = self.config.native_search_params;

        widget::settings::section()
            .title(fl!("original-cpp-parameters"))
            .add(
                widget::settings::togglable(fl!("ocr-image-cleanup"))
                    .description(fl!("run-find-text-lines"))
                    .toggler(native.apply_ocr_image_cleanup, move |enabled| {
                        Message::SetNativeSearchParams(NativeSearchParams {
                            apply_ocr_image_cleanup: enabled,
                            ..native
                        })
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
                    move |threads| {
                        Message::SetNativeSearchParams(NativeSearchParams { threads, ..native })
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
                    move |min_subtitle_frames| {
                        Message::SetNativeSearchParams(NativeSearchParams {
                            min_subtitle_frames,
                            ..native
                        })
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
                    move |text_percent| {
                        Message::SetNativeSearchParams(NativeSearchParams {
                            text_percent,
                            ..native
                        })
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
                    move |min_text_length| {
                        Message::SetNativeSearchParams(NativeSearchParams {
                            min_text_length,
                            ..native
                        })
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
                    move |vertical_edges_line_error| {
                        Message::SetNativeSearchParams(NativeSearchParams {
                            vertical_edges_line_error,
                            ..native
                        })
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
                    move |ila_points_line_error| {
                        Message::SetNativeSearchParams(NativeSearchParams {
                            ila_points_line_error,
                            ..native
                        })
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
                    move |max_frame_gap_down| {
                        Message::SetNativeSearchParams(NativeSearchParams {
                            max_frame_gap_down,
                            ..native
                        })
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
                    move |max_frame_gap_up| {
                        Message::SetNativeSearchParams(NativeSearchParams {
                            max_frame_gap_up,
                            ..native
                        })
                    },
                ),
            ))
            .add(widget::settings::togglable(fl!("use-isa-images")).toggler(
                native.use_isa_images,
                move |use_isa_images| {
                    Message::SetNativeSearchParams(NativeSearchParams {
                        use_isa_images,
                        ..native
                    })
                },
            ))
            .add(widget::settings::togglable(fl!("use-ila-images")).toggler(
                native.use_ila_images,
                move |use_ila_images| {
                    Message::SetNativeSearchParams(NativeSearchParams {
                        use_ila_images,
                        ..native
                    })
                },
            ))
            .add(
                widget::settings::togglable(fl!("replace-isa-with-filtered-image")).toggler(
                    native.replace_isa_with_filtered,
                    move |enabled| {
                        Message::SetNativeSearchParams(NativeSearchParams {
                            replace_isa_with_filtered: enabled,
                            ..native
                        })
                    },
                ),
            )
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
                .style(vse_ui::theme::Container::Card::style),
        )
        .height(Length::Fill)
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
                dialog_page: None,
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
            Message::SelectDialogPage(page) => {
                self.dialog_page = page;
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
            Message::SetNativeSearchParams(value) => {
                match self
                    .config
                    .set_native_search_params(&self.config_handler, value)
                {
                    Ok(_) => Task::none(),
                    Err(error) => Task::done(Message::ErrorReported(Arc::new(eyre::eyre!(
                        "failed to save original C++ parameters: {error}"
                    )))),
                }
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
                self.dialog_page = Some(DialogPage::Error);
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
        let dialog = self.dialog_page.map(|page| {
            let (title, content) = match page {
                DialogPage::Settings => (fl!("settings"), self.settings_view()),
                DialogPage::Error => (fl!("errors"), self.error_view()),
            };

            vse_ui::components::dialog(
                title,
                content,
                widget::button(icon::from_name("window-close-symbolic"))
                    .padding(4)
                    .style(vse_ui::theme::Button::Icon::style)
                    .on_press(Message::SelectDialogPage(None)),
            )
        });

        shell::Shell::new(active.label(), active.details(), content)
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
