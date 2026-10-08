use crate::config::{Config, ProcessingResolution, SubtitleDetector};
use crate::impl_report_residual;
use crate::native_video_sub_finder::NativeSearchParams;
use crate::ocr::OcrModel;
use crate::{app::ReportLike, apply_traits::ApplyConditional};
use iced::Color;
use iced::font::Weight;
use std::sync::LazyLock;
use vse_ui::theme::WithRadius;

use super::*;
use cosmic::Element;
use iced::{Font, alignment::Vertical};
use vse_ui::{
    self as cosmic, Apply,
    theme::{self, COSMIC},
    widget::segmented_button::{Entity, SingleSelectModel, SingleSelectModelBuilder},
};

pub struct Model {
    pub video_player: video_player_widget::Model,
    pub screenshot_selection: Option<iced::Rectangle>,
    pub screenshot_selection_scaled: Option<iced::Rectangle>,
    pub canvas_dimensions: iced::Rectangle,
    pub canvas_generation: u32,
    processing_resolutions: SingleSelectModel<ProcessingResolution>,
    original_cpp_expanded: bool,
    compact_settings: bool,
}

static CONFIG_WRITE_CHANNEL: LazyLock<(
    async_channel::Sender<SettingsMessage>,
    async_channel::Receiver<SettingsMessage>,
)> = LazyLock::new(async_channel::unbounded);

#[derive(Clone)]
struct ConfigWriteStreamData {
    handler: Arc<cosmic_config::Config>,
}

impl std::hash::Hash for ConfigWriteStreamData {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.handler) as usize).hash(state);
    }
}

impl Default for Model {
    fn default() -> Self {
        Self::new(ProcessingResolution::default())
    }
}

#[derive(derive_more::Debug, Clone)]
pub enum Message {
    ResetSelection,
    Canvas(selection_canvas::Message),
    VideoPlayer(video_player_widget::Message),
    CopySelectionDimensions(String),
    StartSubtitleDisplay,
    Settings(SettingsMessage),
    SelectProcessingResolution(Entity),
    ToggleOriginalCppSettings,
    OpenSettings,
    SetCompactSettings(bool),
    ConfigWriteError(String),
}

#[derive(Debug, Clone)]
pub enum SettingsMessage {
    SetOcrModel(OcrModel),
    SetSubtitleDetector(SubtitleDetector),
    SetNativeSearchParams(NativeSearchParams),
    SetPostOcrProcessing(bool),
    SetProcessingResolution(ProcessingResolution),
}

impl SettingsMessage {
    fn persist(self, handler: &cosmic_config::Config) -> Result<(), cosmic_config::Error> {
        use cosmic_config::ConfigSet;

        match self {
            Self::SetOcrModel(value) => handler.set("ocr_model", value),
            Self::SetSubtitleDetector(value) => handler.set("subtitle_detector", value),
            Self::SetNativeSearchParams(value) => handler.set("native_search_params", value),
            Self::SetPostOcrProcessing(value) => handler.set("post_ocr_processing", value),
            Self::SetProcessingResolution(value) => handler.set("processing_resolution", value),
        }
    }
}

#[derive(Debug)]
pub enum Event {
    StartSubtitleSearch(std::path::PathBuf, Option<iced::Rectangle>),
    Run(Task<Message>),
    CopySelectionDimensions(String),
    OpenSettings,
    Error(eyre::Report),
    None,
}

impl ReportLike for Event {
    fn err(e: eyre::Report) -> Self {
        Self::Error(e)
    }
    fn none() -> Self {
        Self::None
    }
}

impl_report_residual!(Event);

const fn playback_map(x: video_player_widget::PlayerMessage) -> Message {
    Message::VideoPlayer(video_player_widget::Message::Playback(x))
}

fn selection_details_translation(
    selection: iced::Rectangle,
    details: iced::Rectangle,
    viewport: iced::Rectangle,
    canvas_size: iced::Size,
) -> iced::Vector {
    // The selection is canvas-local; the float's bounds and viewport are absolute.
    let canvas = iced::Rectangle::new(details.position(), canvas_size);
    let visible = canvas.intersection(&viewport).unwrap_or(viewport);
    let gap = 8.0;
    let anchor = details.position() + iced::Vector::new(selection.x, selection.y);
    let above = anchor.y - details.height - gap;
    let y = if above >= visible.y {
        above
    } else {
        anchor.y + gap
    };
    let x = anchor.x.clamp(
        visible.x,
        (visible.x + visible.width - details.width).max(visible.x),
    );
    let y = y.clamp(
        visible.y,
        (visible.y + visible.height - details.height).max(visible.y),
    );

    iced::Vector::new(x - details.x, y - details.y)
}

impl Model {
    pub fn new(processing_resolution: ProcessingResolution) -> Self {
        let labels = ProcessingResolution::labels();
        let mut processing_resolutions =
            SingleSelectModelBuilder::from_array(ProcessingResolution::ALL.map(|resolution| {
                let index = ProcessingResolution::ALL
                    .iter()
                    .position(|candidate| *candidate == resolution)
                    .unwrap_or_default();
                (labels[index].clone(), resolution)
            }))
            .with_first_as_active()
            .build();
        let active = processing_resolutions
            .entries()
            .iter()
            .find_map(|(id, entry)| (entry.data() == &processing_resolution).then_some(*id));
        if let Some(active) = active {
            processing_resolutions.activate(active);
        }

        Self {
            video_player: video_player_widget::Model::default(),
            screenshot_selection: None,
            screenshot_selection_scaled: None,
            canvas_dimensions: iced::Rectangle::default(),
            canvas_generation: 0,
            processing_resolutions,
            original_cpp_expanded: false,
            compact_settings: false,
        }
    }

    /// Other pages can be open before a video has been loaded.
    pub fn video_path(&self) -> Option<&std::path::PathBuf> {
        match &self.video_player {
            Some(player) => Some(player.path()),
            None => None,
        }
    }

    fn update(&mut self, message: Message, config: &mut Config) -> Event {
        let needs_recompute = Self::scaled_selection_needs_recomputation(&message);

        let task = match message {
            Message::ResetSelection => {
                self.screenshot_selection = None;
                self.canvas_generation = self.canvas_generation.wrapping_add(1);
                Event::None
            }
            Message::Canvas(x) => match x {
                selection_canvas::Message::ScreenshotRegion(rectangle) => {
                    self.screenshot_selection = rectangle;
                    Event::None
                }
                selection_canvas::Message::CanvasSize(rectangle) => {
                    self.screenshot_selection = self.screenshot_selection.map(|selection| {
                        selection_canvas::rescale_rectangle(
                            selection,
                            self.canvas_dimensions.size(),
                            rectangle.size(),
                        )
                    });
                    self.canvas_dimensions = rectangle;
                    Event::None
                }
            },
            Message::VideoPlayer(message) => match self.video_player.update(message, ()) {
                video_player_widget::Event::Run(task) => Event::Run(task.map(Message::VideoPlayer)),
                video_player_widget::Event::Error(error) => Event::Error(error),
                video_player_widget::Event::None => Event::None,
            },
            Message::CopySelectionDimensions(dimensions) => {
                Event::CopySelectionDimensions(dimensions)
            }
            Message::OpenSettings => Event::OpenSettings,
            Message::SetCompactSettings(compact) => {
                self.compact_settings = compact;
                Event::None
            }
            Message::ConfigWriteError(error) => Event::Error(eyre::eyre!(error)),
            Message::SelectProcessingResolution(id) => {
                self.processing_resolutions.activate(id);
                self.update(
                    Message::Settings(SettingsMessage::SetProcessingResolution(
                        *self.processing_resolutions.active_data(),
                    )),
                    config,
                )
            }
            Message::ToggleOriginalCppSettings => {
                self.original_cpp_expanded = !self.original_cpp_expanded;
                Event::None
            }
            Message::StartSubtitleDisplay => match &self.video_player {
                Some(player) => Event::StartSubtitleSearch(
                    player.path().clone(),
                    self.screenshot_selection_scaled,
                ),
                None => Event::None,
            },

            Message::Settings(message) => {
                match &message {
                    SettingsMessage::SetOcrModel(value) => {
                        config.ocr_model = value.clone();
                    }
                    SettingsMessage::SetSubtitleDetector(value) => {
                        config.subtitle_detector = *value;
                    }
                    SettingsMessage::SetNativeSearchParams(value) => {
                        config.native_search_params = *value;
                    }
                    SettingsMessage::SetPostOcrProcessing(value) => {
                        config.post_ocr_processing = *value;
                    }
                    SettingsMessage::SetProcessingResolution(value) => {
                        config.processing_resolution = *value;
                        let active = self
                            .processing_resolutions
                            .entries()
                            .iter()
                            .find_map(|(id, entry)| (entry.data() == value).then_some(*id));
                        if let Some(active) = active {
                            self.processing_resolutions.activate(active);
                        }
                    }
                }
                CONFIG_WRITE_CHANNEL.0.try_send(message)?;
                Event::None
            }
        };

        if needs_recompute {
            self.recompute_scaled_selection();
        }
        task
    }

    async fn persist_setting(
        message: SettingsMessage,
        handler: Arc<cosmic_config::Config>,
    ) -> eyre::Result<(), String> {
        smol::unblock(move || {
            let setting = format!("{message:?}");
            message
                .persist(&handler)
                .map_err(|error| format!("failed to save {setting}: {error}"))
        })
        .await
    }

    fn view<'a>(&'a self, config: &'a Config) -> Element<'a, Message> {
        widget::responsive(move |size| -> Element<'a, Message> {
            let spacing = theme::spacing();
            let settings_width = (size.width * 7.0 / 17.0).clamp(400.0, 500.0);
            let video_width = size.width - settings_width - spacing.space_m;
            let settings = widget::container(self.settings_view(config))
                .padding(spacing.space_l)
                .style(theme::container::settings_sidebar);
            let controls = self.video_player.as_ref().map(Self::video_controls);

            if video_width <= settings_width {
                const MIN_SETTINGS_HEIGHT: f32 = 240.0;
                let video = widget::container(widget::column![self.video_preview()])
                    .width(Length::Fill)
                    .height(Length::Shrink);

                let settings: Element<'a, Message> = if self.compact_settings {
                    let button = widget::button(widget::text(fl!("settings")))
                        .padding(spacing.space_xs)
                        .on_press(Message::OpenSettings);
                    // `the widget::space()` here is for allowing the sensor to know how much space there is left
                    widget::column![button, widget::space().height(Length::Fill)]
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .into()
                } else {
                    settings.width(Length::Fill).height(Length::Shrink).into()
                };

                let settings = widget::sensor(settings).on_resize(move |size| {
                    let compact = size.height < MIN_SETTINGS_HEIGHT;
                    (compact != self.compact_settings)
                        .then_some(Message::SetCompactSettings(compact))
                });

                widget::column![video, controls, settings]
                    .spacing(spacing.space_s)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            } else {
                // Match the stacked branch's container/column structure so
                // the selection canvas retains its state across the switch.
                let video = widget::container(
                    widget::column![self.video_preview(), controls].spacing(spacing.space_s),
                )
                .width(Length::Fill)
                .height(Length::Fill);
                let settings = settings
                    .width(Length::Fixed(settings_width))
                    .height(Length::Fill);

                widget::row![video, settings]
                    .spacing(spacing.space_m)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            }
        })
        .into()
    }

    fn video_preview(&self) -> Element<'_, Message> {
        let Some(player) = &self.video_player else {
            return widget::space().into();
        };
        let img = player
            .image(COSMIC.corner_radii.radius_l[0])
            .map(playback_map);

        let selection_canvas = widget::canvas(selection_canvas::SelectionProgram::default())
            .width(Length::Fill)
            .height(Length::Fill)
            .apply(Element::from)
            .map(Message::Canvas);

        let canvas_size = self.canvas_dimensions.size();
        let s = self
            .screenshot_selection
            .and_then(|x| Some((x, self.screenshot_selection_scaled?)))
            .map(|(s, scaled)| {
                let format_dimensions = |s: iced::Rectangle| {
                    format!("{:.0} x {:.0} @ {:.0}, {:.0}", s.width, s.height, s.x, s.y)
                };
                widget::text(format_dimensions(scaled))
                    .font(Font::MONOSPACE.weight(Weight::Bold))
                    .color(Color::BLACK)
                    .apply(widget::button)
                    .on_press(
                        scaled
                            .apply(format_dimensions)
                            .apply(|x| x.chars().filter(|x| !x.is_whitespace()).collect::<String>())
                            .apply(Message::CopySelectionDimensions),
                    )
                    .style(|x, y| theme::button::suggested(x, y).with_radius(COSMIC.radius_s()[0]))
                    .apply(widget::float)
                    .translate(move |original, viewport| {
                        selection_details_translation(s, original, viewport, canvas_size)
                    })
                    .apply(Element::from)
            });

        widget::stack![img, selection_canvas, s].into()
    }

    pub fn title_actions(&self) -> Element<'_, Message> {
        let load_video = if self.video_player.is_some() {
            widget::button(widget::text(fl!("change-video")))
        } else {
            widget::button(widget::text(fl!("load-video"))).style(theme::button::suggested)
        }
        .on_press(Message::VideoPlayer(video_player_widget::Message::Loading(
            video_player_widget::LoadingMessage::PickVideo,
        )))
        .padding(theme::spacing().space_xs);

        let start_subtitle = widget::button(widget::text(fl!("find-subtitles")))
            .apply_if(self.video_player.is_some(), |x| {
                x.on_press(Message::StartSubtitleDisplay)
                    .style(theme::button::suggested)
            })
            .padding(theme::spacing().space_xs);

        widget::row![load_video, start_subtitle]
            .spacing(theme::spacing().space_xs)
            .into()
    }

    pub fn settings_view<'a>(&'a self, config: &'a Config) -> Element<'a, Message> {
        let ocr_models = OcrModel::all(config);
        let selected_ocr = ocr_models
            .iter()
            .position(|model| model == &config.ocr_model);
        let selected_detector = SubtitleDetector::ALL
            .iter()
            .position(|value| value == &config.subtitle_detector);
        let mut sections = vec![
            Element::<SettingsMessage>::from(
                widget::settings::section()
                    .title(fl!("text-recognition"))
                    .add(widget::settings::item(
                        fl!("ocr-model"),
                        widget::dropdown(OcrModel::labels(config), selected_ocr, move |index| {
                            SettingsMessage::SetOcrModel(ocr_models[index].clone())
                        }),
                    ))
                    .add(
                        widget::settings::togglable(fl!("post-ocr-result-processing"))
                            .description(fl!("merge-adjacent-detections"))
                            .toggler(
                                config.post_ocr_processing,
                                SettingsMessage::SetPostOcrProcessing,
                            ),
                    ),
            )
            .map(Message::Settings),
            widget::settings::section()
                .title(fl!("subtitle-detection"))
                .add(
                    widget::settings::item(
                        fl!("processing-resolution"),
                        widget::segmented_control::horizontal(&self.processing_resolutions)
                            .fill()
                            .on_activate(Message::SelectProcessingResolution),
                    )
                    .vertical(),
                )
                .add(widget::settings::item(
                    fl!("implementation"),
                    widget::dropdown(SubtitleDetector::labels(), selected_detector, |index| {
                        Message::Settings(SettingsMessage::SetSubtitleDetector(
                            SubtitleDetector::ALL[index],
                        ))
                    }),
                ))
                .into(),
        ];

        if config.subtitle_detector == SubtitleDetector::OriginalCpp {
            sections.push(
                widget::settings::collapsible_section(
                    fl!("original-cpp-parameters"),
                    self.original_cpp_expanded,
                    Message::ToggleOriginalCppSettings,
                    Self::original_cpp_settings(config).map(Message::Settings),
                )
                .into(),
            );
        }

        widget::scrollable(widget::settings::view_column(sections))
            .height(Length::Fill)
            .into()
    }

    fn original_cpp_settings(config: &Config) -> Element<'_, SettingsMessage> {
        let native = config.native_search_params;

        widget::settings::section()
            .add(
                widget::settings::togglable(fl!("ocr-image-cleanup"))
                    .description(fl!("run-find-text-lines"))
                    .toggler(native.apply_ocr_image_cleanup, move |enabled| {
                        SettingsMessage::SetNativeSearchParams(NativeSearchParams {
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
                        SettingsMessage::SetNativeSearchParams(NativeSearchParams {
                            threads,
                            ..native
                        })
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
                        SettingsMessage::SetNativeSearchParams(NativeSearchParams {
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
                        SettingsMessage::SetNativeSearchParams(NativeSearchParams {
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
                        SettingsMessage::SetNativeSearchParams(NativeSearchParams {
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
                        SettingsMessage::SetNativeSearchParams(NativeSearchParams {
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
                        SettingsMessage::SetNativeSearchParams(NativeSearchParams {
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
                        SettingsMessage::SetNativeSearchParams(NativeSearchParams {
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
                        SettingsMessage::SetNativeSearchParams(NativeSearchParams {
                            max_frame_gap_up,
                            ..native
                        })
                    },
                ),
            ))
            .add(widget::settings::togglable(fl!("use-isa-images")).toggler(
                native.use_isa_images,
                move |use_isa_images| {
                    SettingsMessage::SetNativeSearchParams(NativeSearchParams {
                        use_isa_images,
                        ..native
                    })
                },
            ))
            .add(widget::settings::togglable(fl!("use-ila-images")).toggler(
                native.use_ila_images,
                move |use_ila_images| {
                    SettingsMessage::SetNativeSearchParams(NativeSearchParams {
                        use_ila_images,
                        ..native
                    })
                },
            ))
            .add(
                widget::settings::togglable(fl!("replace-isa-with-filtered-image")).toggler(
                    native.replace_isa_with_filtered,
                    move |enabled| {
                        SettingsMessage::SetNativeSearchParams(NativeSearchParams {
                            replace_isa_with_filtered: enabled,
                            ..native
                        })
                    },
                ),
            )
            .into()
    }

    fn video_controls(player: &video_player_widget::Player) -> Element<'_, Message> {
        let icon = |name| {
            widget::icon::from_name(name)
                .apply(widget::icon_button)
                .padding(theme::spacing().space_xs)
        };
        let seek_duration = Duration::from_secs(5);
        let pause = icon("media-playback-pause-symbolic");
        let start = icon("media-playback-start-symbolic");
        let forward = icon("media-seek-forward-symbolic").on_press(Message::VideoPlayer(
            video_player_widget::Message::Playback(
                video_player_widget::PlayerMessage::SeekForward(seek_duration),
            ),
        ));
        let backward = icon("media-seek-backward-symbolic").on_press(Message::VideoPlayer(
            video_player_widget::Message::Playback(
                video_player_widget::PlayerMessage::SeekBackward(seek_duration),
            ),
        ));
        let play_button = if player.is_paused() { start } else { pause }.on_press(
            Message::VideoPlayer(video_player_widget::Message::Playback(
                video_player_widget::PlayerMessage::PauseToggle,
            )),
        );

        let slider = player.slider().map(playback_map);
        let current = player
            .current_time()
            .apply(format_duration)
            .apply(widget::text)
            .font(Font::MONOSPACE);

        let total = player
            .video_duration()
            .apply(format_duration)
            .apply(widget::text)
            .font(Font::MONOSPACE);

        let row = widget::row!(backward, play_button, forward, current, slider, total)
            .spacing(theme::spacing().space_s)
            .align_y(Vertical::Center)
            .apply(widget::container)
            .padding(theme::spacing().space_m)
            .width(Length::Fill)
            .style(vse_ui::theme::container::card);
        row.into()
    }

    fn subscription(&self) -> Subscription<Message> {
        self.video_player.subscription(()).map(Message::VideoPlayer)
    }

    pub fn settings_subscription(
        &self,
        handler: &Arc<cosmic_config::Config>,
    ) -> Subscription<Message> {
        Subscription::run_with(
            ConfigWriteStreamData {
                handler: Arc::clone(handler),
            },
            |data| config_write_stream(data.handler.clone()),
        )
    }

    fn recompute_scaled_selection(&mut self) {
        let Some(player) = &self.video_player else {
            self.screenshot_selection_scaled = None;
            return;
        };
        let Some((_, size)) = player.allocation() else {
            self.screenshot_selection_scaled = None;
            return;
        };
        let Some(ele) = self.screenshot_selection else {
            self.screenshot_selection_scaled = None;
            return;
        };

        let canvas_size = self.canvas_dimensions.size();

        if canvas_size.width <= 0.0 || canvas_size.height <= 0.0 {
            self.screenshot_selection_scaled = None;
            return;
        }

        let scaled = selection_canvas::rescale_rectangle(ele, canvas_size, *size);

        self.screenshot_selection_scaled = Some(scaled);
    }

    const fn scaled_selection_needs_recomputation(message: &Message) -> bool {
        matches!(
            message,
            Message::VideoPlayer(video_player_widget::Message::Playback(
                video_player_widget::PlayerMessage::VideoFrameAllocated(_)
            )) | Message::Canvas(_)
                | Message::ResetSelection
        )
    }
}

fn config_write_stream(
    handler: Arc<cosmic_config::Config>,
) -> impl futures::Stream<Item = Message> + Send {
    config_write_stream_from(CONFIG_WRITE_CHANNEL.1.clone(), handler)
}

fn config_write_stream_from(
    receiver: async_channel::Receiver<SettingsMessage>,
    handler: Arc<cosmic_config::Config>,
) -> impl futures::Stream<Item = Message> + Send {
    use futures::SinkExt;

    iced::stream::channel(1, async move |mut output| {
        while let Ok(message) = receiver.recv().await {
            if let Err(error) = Model::persist_setting(message, handler.clone()).await
                && output.send(Message::ConfigWriteError(error)).await.is_err()
            {
                break;
            }
        }
    })
}

impl Composition for Model {
    type Message = Message;
    type Event = Event;
    type ViewContext<'a> = &'a Config;
    type UpdateContext<'a> = &'a mut Config;
    type SubscriptionContext<'a> = ();

    fn view<'a>(&'a self, config: Self::ViewContext<'a>) -> Element<'a, Self::Message> {
        Self::view(self, config)
    }

    fn update(&mut self, message: Self::Message, config: Self::UpdateContext<'_>) -> Self::Event {
        Self::update(self, message, config)
    }

    fn subscription(&self, (): Self::SubscriptionContext<'_>) -> Subscription<Self::Message> {
        Self::subscription(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config_handler(name: &str) -> cosmic_config::Config {
        cosmic_config::Config::with_custom_path(
            name,
            7,
            std::env::temp_dir().join(format!(
                "videosubextract-prepare-tests-{}",
                std::process::id()
            )),
        )
        .expect("isolated test config")
    }

    #[test]
    fn processing_resolution_model_uses_the_configured_selection() {
        for resolution in ProcessingResolution::ALL {
            let model = Model::new(resolution);
            assert_eq!(*model.processing_resolutions.active_data(), resolution);
        }
    }

    #[test]
    fn selecting_a_processing_resolution_updates_the_config() {
        let mut model = Model::default();
        let mut config = Config::default();
        let resolution = ProcessingResolution::UltraHd4k;
        let id = model
            .processing_resolutions
            .entries()
            .iter()
            .find_map(|(id, entry)| (entry.data() == &resolution).then_some(*id))
            .expect("4K resolution entry");

        let event = model.update(Message::SelectProcessingResolution(id), &mut config);

        assert_eq!(*model.processing_resolutions.active_data(), resolution);
        assert_eq!(config.processing_resolution, resolution);
        assert!(matches!(event, Event::None), "{:#?}", event);
    }

    #[test]
    fn config_subscription_saves_queued_changes_in_order() {
        use cosmic_config::ConfigGet;
        use futures::StreamExt;

        let handler = test_config_handler("ordered-writes").apply(Arc::new);
        let (sender, receiver) = async_channel::unbounded();
        let first = SettingsMessage::SetProcessingResolution(ProcessingResolution::Hd720);
        let last = SettingsMessage::SetProcessingResolution(ProcessingResolution::UltraHd4k);

        sender.try_send(first).unwrap();
        sender.try_send(last).unwrap();
        sender.close();
        let errors =
            smol::block_on(config_write_stream_from(receiver, handler.clone()).collect::<Vec<_>>());
        assert!(errors.is_empty(), "{errors:?}");
        assert!(sender.is_empty());
        assert_eq!(
            handler
                .get::<ProcessingResolution>("processing_resolution")
                .unwrap(),
            ProcessingResolution::UltraHd4k
        );
    }

    #[test]
    fn config_subscription_identity_stays_stable_when_rebuilt() {
        use iced::advanced::subscription::{Hasher, into_recipes};
        use std::hash::Hasher as _;

        let model = Model::default();
        let handler = Arc::new(test_config_handler("subscription-identity"));
        let hash = |subscription| {
            let recipes = into_recipes(subscription);
            assert_eq!(recipes.len(), 1);
            let mut hasher = Hasher::default();
            recipes[0].hash(&mut hasher);
            hasher.finish()
        };

        assert_eq!(
            hash(model.settings_subscription(&handler)),
            hash(model.settings_subscription(&handler)),
        );
    }

    #[test]
    fn config_write_failures_are_reported_and_the_worker_continues() {
        use futures::StreamExt;

        // A system-only handler has no writable user config directory.
        let handler = Arc::new(cosmic_config::Config::system("write-errors", 7).unwrap());
        let (sender, receiver) = async_channel::unbounded();
        sender
            .try_send(SettingsMessage::SetPostOcrProcessing(false))
            .unwrap();
        sender
            .try_send(SettingsMessage::SetPostOcrProcessing(true))
            .unwrap();
        sender.close();

        let errors =
            smol::block_on(config_write_stream_from(receiver, handler).collect::<Vec<_>>());
        assert_eq!(errors.len(), 2);
        let mut model = Model::default();
        let mut config = Config::default();
        for error in errors {
            assert!(matches!(error, Message::ConfigWriteError(_)));
            assert!(matches!(model.update(error, &mut config), Event::Error(_)));
        }
    }

    #[test]
    fn original_cpp_parameters_start_collapsed_and_can_be_expanded() {
        let mut model = Model::default();
        let mut config = Config::default();
        assert!(!model.original_cpp_expanded);

        let event = model.update(Message::ToggleOriginalCppSettings, &mut config);

        assert!(model.original_cpp_expanded);
        assert!(matches!(event, Event::None));
    }

    #[test]
    fn selection_details_stay_visible_at_every_canvas_corner() {
        let details =
            iced::Rectangle::new(iced::Point::new(120.0, 80.0), iced::Size::new(240.0, 36.0));
        let viewport = iced::Rectangle::with_size(iced::Size::new(1200.0, 900.0));
        for canvas_size in [iced::Size::new(800.0, 450.0), iced::Size::new(320.0, 180.0)] {
            let canvas = iced::Rectangle::new(details.position(), canvas_size);
            for x in [0.0, canvas_size.width - 20.0] {
                for y in [0.0, canvas_size.height - 20.0] {
                    let selection =
                        iced::Rectangle::new(iced::Point::new(x, y), iced::Size::new(20.0, 20.0));
                    let translation =
                        selection_details_translation(selection, details, viewport, canvas_size);
                    let placed =
                        iced::Rectangle::new(details.position() + translation, details.size());
                    assert!(placed.x >= canvas.x && placed.y >= canvas.y);
                    assert!(placed.x + placed.width <= canvas.x + canvas.width);
                    assert!(placed.y + placed.height <= canvas.y + canvas.height);
                }
            }
        }
    }

    #[test]
    fn selection_details_follow_the_selection_above_when_there_is_room() {
        let details =
            iced::Rectangle::new(iced::Point::new(120.0, 80.0), iced::Size::new(240.0, 36.0));
        let selection = iced::Rectangle::new(
            iced::Point::new(100.0, 100.0),
            iced::Size::new(200.0, 100.0),
        );
        let canvas_size = iced::Size::new(800.0, 450.0);
        let viewport = iced::Rectangle::with_size(iced::Size::new(1200.0, 900.0));
        assert_eq!(
            selection_details_translation(selection, details, viewport, canvas_size),
            iced::Vector::new(100.0, 56.0),
        );
    }

    #[test]
    fn selection_details_respect_a_clipped_viewport() {
        let details =
            iced::Rectangle::new(iced::Point::new(120.0, 80.0), iced::Size::new(240.0, 36.0));
        let viewport = iced::Rectangle::new(
            iced::Point::new(150.0, 100.0),
            iced::Size::new(400.0, 200.0),
        );
        let canvas_size = iced::Size::new(800.0, 450.0);
        for position in [iced::Point::ORIGIN, iced::Point::new(780.0, 430.0)] {
            let selection = iced::Rectangle::new(position, iced::Size::new(20.0, 20.0));
            let translation =
                selection_details_translation(selection, details, viewport, canvas_size);
            let placed = iced::Rectangle::new(details.position() + translation, details.size());
            assert!(placed.x >= viewport.x && placed.y >= viewport.y);
            assert!(placed.x + placed.width <= viewport.x + viewport.width);
            assert!(placed.y + placed.height <= viewport.y + viewport.height);
        }
    }

    #[test]
    fn canvas_resize_keeps_the_crop_in_sync_with_the_overlay() {
        let mut config = Config::default();
        let selection =
            iced::Rectangle::new(iced::Point::new(80.0, 90.0), iced::Size::new(640.0, 270.0));
        let original_bounds =
            iced::Rectangle::new(iced::Point::new(20.0, 30.0), iced::Size::new(800.0, 450.0));
        let mut model = Model {
            screenshot_selection: Some(selection),
            canvas_dimensions: original_bounds,
            ..Model::default()
        };
        model.update(
            Message::Canvas(selection_canvas::Message::CanvasSize(iced::Rectangle::new(
                iced::Point::new(40.0, 50.0),
                iced::Size::new(400.0, 225.0),
            ))),
            &mut config,
        );
        assert_eq!(
            model.screenshot_selection,
            Some(iced::Rectangle::new(
                iced::Point::new(40.0, 45.0),
                iced::Size::new(320.0, 135.0)
            )),
        );
        model.update(
            Message::Canvas(selection_canvas::Message::CanvasSize(original_bounds)),
            &mut config,
        );
        assert_eq!(model.screenshot_selection, Some(selection));
    }
}
