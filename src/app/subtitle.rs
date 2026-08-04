use crate::config::{ProcessingResolution, SubtitleDetector};
use crate::extraction::{self, OcrHandle, Request as ExtractionRequest, Subtitle};
use crate::native_video_sub_finder::NativeSearchParams;
use crate::video_player::CropRect;
use cosmic::theme;
use iced::futures::StreamExt;
use iced::widget::text_editor;
use image::RgbaImage;

use super::*;
use std::time::Duration;

const JUMP_TO_END_DELAY: Duration = Duration::from_secs(3);

const TOOLBAR_SIZE: f32 = 48.0;
const RESULT_ROW_HEIGHT: f32 = 120.0;
const RESULT_ROW_OVERSCAN: usize = 3;
const INITIAL_VISIBLE_RESULT_ROWS: usize = 8;

#[derive(Debug, Clone)]
pub struct SubtitleResult {
    id: u64,
    pub subtitle: Subtitle,
    pub preview: widget::image::Handle,
    editor_content: text_editor::Content,
}

impl SubtitleResult {
    fn new(id: u64, subtitle: Subtitle, preview: widget::image::Handle) -> Self {
        let editor_content = text_editor::Content::with_text(&subtitle.text);

        Self {
            id,
            subtitle,
            preview,
            editor_content,
        }
    }

    pub(crate) fn set_text(&mut self, text: String) {
        self.editor_content = text_editor::Content::with_text(&text);
        self.subtitle.text = text;
    }
}

#[derive(Default)]
pub struct Model {
    pub search_active: bool,
    pub search_gen: usize,
    pub search_path: Option<std::path::PathBuf>,
    pub search_selection: Option<iced::Rectangle>,
    search_ocr: Option<OcrHandle>,
    search_detector: SubtitleDetector,
    native_search_params: NativeSearchParams,
    post_ocr_processing: bool,
    processing_resolution: ProcessingResolution,
    pub results: Vec<SubtitleResult>,
    pub preview: Option<widget::image::Handle>,
    pub current_frame: usize,
    pub done: bool,
    pub progress_bar: ProgressBar,
    scrollbar_jump_status: ScrollbarJumpStatus,
    next_result_id: u64,
    result_scroll_offset: f32,
    result_viewport_height: f32,
    edit_history: Vec<SubtitleEdit>,
}

#[derive(Debug, Clone)]
enum SubtitleEdit {
    Delete {
        index: usize,
        result: SubtitleResult,
    },
    MergeWithPrevious {
        index: usize,
        previous_end_timestamp: Duration,
        result: SubtitleResult,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ScrollbarJumpStatus {
    #[default]
    NoShow,
    TimeoutRunning,
    DisplayButton,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VirtualRowKey {
    TopSpacer,
    Result(u64),
    BottomSpacer,
}

pub struct ProgressBar(indicatif::ProgressBar);

impl std::ops::Deref for ProgressBar {
    type Target = indicatif::ProgressBar;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl Default for ProgressBar {
    fn default() -> Self {
        Self(indicatif::ProgressBar::hidden())
    }
}

#[derive(derive_more::Debug, Clone)]
pub enum Message {
    Progress {
        frame: usize,

        #[debug("{}x{}", preview.width(), preview.height())]
        preview: RgbaImage,
    },
    EventFound {
        subtitle: Subtitle,
        replace_previous: bool,
        #[debug("{}x{}", preview.width(), preview.height())]
        preview: RgbaImage,
    },
    SearchStarted {
        total_frames: Option<usize>,
    },
    Delete(usize),
    MergeWithPrevious(usize),
    UndoEdit,
    Scrolled {
        at_end: bool,
        offset: f32,
        viewport_height: f32,
    },
    JumpToEnd {
        id: iced::id::Id,
    },
    ShowJumpToEnd,
    SearchDone,
    SearchError(String),
    GoToPostProduction,
    SubtitleContentEdit {
        id: usize,
        action: text_editor::Action,
    },
    None,
}

pub enum Event {
    GoToPostProduction,
    Run(Task<Message>),
    Error(eyre::Report),
    None,
}

impl Model {
    pub fn start_search(
        &mut self,
        path: std::path::PathBuf,
        selection: Option<iced::Rectangle>,
        config: &Config,
    ) {
        self.search_active = true;
        self.search_gen += 1;
        self.search_path = Some(path);
        self.search_selection = selection;
        self.search_ocr = Some(OcrHandle::new(config.ocr_model.clone()));
        self.search_detector = config.subtitle_detector;
        self.native_search_params = config.native_search_params;
        self.post_ocr_processing = config.post_ocr_processing;
        self.processing_resolution = config.processing_resolution;
        self.results.clear();
        self.preview = None;
        self.current_frame = 0;
        self.done = false;
        self.edit_history.clear();
        self.progress_bar.set_elapsed(Duration::ZERO);
        self.scrollbar_jump_status = ScrollbarJumpStatus::NoShow;
        self.next_result_id = 0;
        self.result_scroll_offset = 0.0;
        self.result_viewport_height = 0.0;
    }

    pub fn update(&mut self, message: Message, config: &Config) -> Event {
        self.set_ocr_model(config.ocr_model.clone());
        match message {
            Message::Progress { frame, preview } => {
                self.progress_bar.set_position(frame as u64);
                self.current_frame = frame;
                self.preview = Some(widget::image::Handle::from_rgba(
                    preview.width(),
                    preview.height(),
                    preview.into_raw(),
                ));
                Event::None
            }
            Message::EventFound {
                subtitle,
                replace_previous,
                preview,
            } => {
                let preview = widget::image::Handle::from_rgba(
                    preview.width(),
                    preview.height(),
                    preview.into_raw(),
                );
                if replace_previous && let Some(previous) = self.results.last_mut() {
                    *previous = SubtitleResult::new(previous.id, subtitle, preview);
                    return Event::None;
                }
                let id = self.next_result_id;
                self.next_result_id = self.next_result_id.wrapping_add(1);
                self.results
                    .push(SubtitleResult::new(id, subtitle, preview));
                Event::None
            }
            Message::SearchStarted { total_frames } => {
                if let Some(total_frames) = total_frames {
                    self.progress_bar.set_length(total_frames as u64);
                }
                Event::None
            }
            Message::SearchDone => {
                self.search_active = false;
                self.done = true;
                self.preview = None;
                Event::None
            }
            Message::SearchError(e) => {
                self.search_active = false;
                Event::Error(eyre::eyre!("subtitle search failed: {e}"))
            }
            Message::GoToPostProduction => Event::GoToPostProduction,
            Message::Scrolled {
                at_end,
                offset,
                viewport_height,
            } => {
                self.result_scroll_offset = offset;
                self.result_viewport_height = viewport_height;
                if at_end {
                    self.scrollbar_jump_status = ScrollbarJumpStatus::NoShow;
                    Event::None
                } else if matches!(self.scrollbar_jump_status, ScrollbarJumpStatus::NoShow) {
                    self.scrollbar_jump_status = ScrollbarJumpStatus::TimeoutRunning;

                    Event::Run(Task::perform(tokio::time::sleep(JUMP_TO_END_DELAY), |()| {
                        Message::ShowJumpToEnd
                    }))
                } else {
                    Event::None
                }
            }
            Message::JumpToEnd { id } => {
                self.scrollbar_jump_status = ScrollbarJumpStatus::NoShow;
                Event::Run(iced::widget::operation::snap_to_end(id))
            }
            Message::ShowJumpToEnd => {
                if self.scrollbar_jump_status == ScrollbarJumpStatus::TimeoutRunning {
                    self.scrollbar_jump_status = ScrollbarJumpStatus::DisplayButton;
                }
                Event::None
            }
            Message::Delete(x) => {
                if x < self.results.len() {
                    let result = self.results.remove(x);
                    self.edit_history
                        .push(SubtitleEdit::Delete { index: x, result });
                }
                Event::None
            }
            Message::MergeWithPrevious(x) => {
                if x > 0 && x < self.results.len() {
                    let result = self.results.remove(x);
                    let previous_end_timestamp = std::mem::replace(
                        &mut self.results[x - 1].subtitle.end_timestamp,
                        result.subtitle.end_timestamp,
                    );
                    self.edit_history.push(SubtitleEdit::MergeWithPrevious {
                        index: x,
                        previous_end_timestamp,
                        result,
                    });
                }
                Event::None
            }
            Message::UndoEdit => {
                if let Some(edit) = self.edit_history.pop() {
                    match edit {
                        SubtitleEdit::Delete { index, result } => {
                            self.results.insert(index.min(self.results.len()), result);
                        }
                        SubtitleEdit::MergeWithPrevious {
                            index,
                            previous_end_timestamp,
                            result,
                        } => {
                            if let Some(previous) = index
                                .checked_sub(1)
                                .and_then(|index| self.results.get_mut(index))
                            {
                                previous.subtitle.end_timestamp = previous_end_timestamp;
                                self.results.insert(index.min(self.results.len()), result);
                            }
                        }
                    }
                }
                Event::None
            }
            Message::SubtitleContentEdit { id, action } => {
                if let Some(result) = self.results.get_mut(id) {
                    result.editor_content.perform(action);
                    result.subtitle.text = result.editor_content.text();
                }
                Event::None
            }
            Message::None => Event::None,
        }
    }

    pub fn set_ocr_model(&self, ocr: OcrModel) {
        if let Some(search_ocr) = &self.search_ocr {
            search_ocr.set(ocr);
        }
    }

    pub fn view(&self, total_frames: Option<usize>, fps: f64) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        let space_s = cosmic::theme::spacing().space_s;
        if let Some(len) = total_frames {
            self.progress_bar.set_length(len as u64);
        }

        let status = if self.done {
            widget::text(format!(
                "Complete — {} subtitle(s) found",
                self.results.len()
            ))
            .class(cosmic::theme::Text::Accent)
            .apply(Element::from)
        } else if self.search_active {
            let status_text = widget::text(format!(
                "## Elapsed {} · IGT {} · ETA {}",
                self.progress_bar.elapsed().apply(format_duration),
                (self.current_frame as u64 / fps as u64)
                    .apply(Duration::from_secs)
                    .apply(format_duration),
                self.progress_bar.eta().apply(format_duration)
            ))
            .class(cosmic::theme::Text::Accent);

            let progress_bar = widget::progress_bar::determinate_linear(
                self.current_frame as f32 / total_frames.unwrap_or(1) as f32,
            )
            .width(Length::Fill);

            widget::row!(status_text, progress_bar)
                .spacing(space_s)
                .align_y(Alignment::Center)
                .width(Length::Fill)
                .apply(Element::from)
        } else {
            widget::text(
                "No active search. Load a video and select a subtitle region on Page Prepare.",
            )
            .class(cosmic::theme::Text::Accent)
            .apply(Element::from)
        };

        let to_post_prod = widget::button::text("Post Production")
            .class(cosmic::theme::Button::Suggested)
            .on_press_maybe((!self.search_active).then_some(Message::GoToPostProduction));

        let undo_edit = widget::button::icon(icon::from_name("edit-undo-symbolic"))
            .on_press_maybe((!self.edit_history.is_empty()).then_some(Message::UndoEdit));
        let row_spacing = f32::from(spacing.space_m);
        let row_pitch = RESULT_ROW_HEIGHT + row_spacing;

        let visible_rows = visible_result_range(
            self.result_scroll_offset,
            self.result_viewport_height,
            row_pitch,
            self.results.len(),
        );
        let top_spacer_height = visible_rows.start as f32 * row_pitch;
        let bottom_spacer_height =
            self.results.len().saturating_sub(visible_rows.end) as f32 * row_pitch;

        let mut virtual_rows = Vec::with_capacity(visible_rows.len() + 2);

        if top_spacer_height > 0.0 {
            virtual_rows.push((
                VirtualRowKey::TopSpacer,
                widget::Space::new()
                    .height(Length::Fixed(top_spacer_height))
                    .into(),
            ));
        }

        virtual_rows.extend(self.results[visible_rows.clone()].iter().enumerate().map(
            |(relative_id, result)| {
                let id = visible_rows.start + relative_id;
                let t_start = result.subtitle.start_timestamp.as_secs_f64();
                let t_end = result.subtitle.end_timestamp.as_secs_f64();

                let toolbar = widget::column![
                    widget::button::icon(widget::icon::from_name("edit-delete-symbolic"))
                        .on_press(Message::Delete(id))
                        .class(cosmic::theme::Button::Destructive),
                ]
                .push_maybe(
                    (id != 0).then_some(
                        widget::button::icon(widget::icon::from_name("go-up-symbolic"))
                            .on_press(Message::MergeWithPrevious(id))
                            .class(cosmic::theme::Button::Icon),
                    ),
                )
                .width(TOOLBAR_SIZE)
                .spacing(space_s)
                .align_x(Alignment::Center);

                let image = widget::image(result.preview.clone())
                    .content_fit(iced::ContentFit::Contain)
                    .width(Length::FillPortion(65))
                    .height(Length::Fill);

                let timeline = widget::text(format!("{t_start:.1}s – {t_end:.1}s"));

                let ocr = widget::text_editor(&result.editor_content)
                    .on_action(move |action| Message::SubtitleContentEdit { id, action })
                    .height(Length::Fill)
                    .min_height(48.0)
                    .class(cosmic::theme::iced::TextEditor::Custom(Box::new(|x, y| {
                        use iced::widget::text_editor::Catalog;
                        let mut style = x.style(&theme::iced::TextEditor::default(), y);
                        style.border.width = 2.0;
                        style
                    })))
                    .apply(Element::from);

                let row = widget::row!(
                    toolbar,
                    image,
                    widget::column![timeline, ocr]
                        .spacing(space_s / 2)
                        .width(Length::FillPortion(35))
                        .height(Length::Fill)
                        .align_x(Alignment::Start),
                )
                .spacing(space_s)
                .padding([0, 40])
                .height(Length::Fixed(RESULT_ROW_HEIGHT))
                .align_y(Alignment::Center);

                let row = widget::container(row)
                    .height(Length::Fixed(row_pitch))
                    .padding(iced::Padding::ZERO.bottom(row_spacing));

                (VirtualRowKey::Result(result.id), row.into())
            },
        ));

        if bottom_spacer_height > 0.0 {
            virtual_rows.push((
                VirtualRowKey::BottomSpacer,
                widget::Space::new()
                    .height(Length::Fixed(bottom_spacer_height))
                    .into(),
            ));
        }

        let result_rows = iced::widget::keyed_column(virtual_rows).width(Length::Fill);

        let mut col = widget::column!(
            widget::row!(status, undo_edit, to_post_prod)
                .spacing(space_s)
                .align_y(Alignment::Center)
        )
        .spacing(space_s);

        let view_card = |title, handle| {
            widget::column!(
                widget::text(title),
                widget::image(handle)
                    .width(Length::Fill)
                    .height(Length::Fixed(120.))
                    .content_fit(iced::ContentFit::Contain)
            )
            .align_x(Alignment::Center)
            .apply(widget::container)
            .class(cosmic::theme::Container::Card)
            .padding(20)
        };

        if let Some(handle) = &self.preview {
            let preview = widget::Row::new()
                .spacing(space_s)
                .push(view_card("View", handle))
                .push_maybe(
                    self.results
                        .last()
                        .map(|x| view_card("Current", &x.preview)),
                );

            col = col.push(preview);
        }

        let scrollable_id = iced::id::Id::new("scrollable");
        let scrollable_id_clone = scrollable_id.clone();

        let results = result_rows
            .apply(widget::container)
            .padding(iced::Padding::ZERO.right(60))
            .height(Length::Fill)
            .apply(widget::scrollable)
            .on_scroll(|viewport| {
                let content_fits =
                    viewport.content_bounds().height <= viewport.bounds().height + 1.0;
                let at_end = content_fits || viewport.relative_offset().y >= 0.999;
                Message::Scrolled {
                    at_end,
                    offset: viewport.absolute_offset().y,
                    viewport_height: viewport.bounds().height,
                }
            })
            .id(scrollable_id_clone)
            .apply(Element::from);

        let stack = iced::widget::Stack::new().push(results);

        let stack = if self.scrollbar_jump_status == ScrollbarJumpStatus::DisplayButton {
            let jump_to_end = widget::button::text("Jump to latest ↓")
                .class(cosmic::theme::Button::Suggested)
                .on_press(Message::JumpToEnd { id: scrollable_id });

            stack.push(iced::widget::bottom_right(jump_to_end).padding(spacing.space_m))
        } else {
            stack
        };

        col.push(stack).into()
    }

    pub fn subscription(&self, video_frame_rate: f64) -> Subscription<Message> {
        let mut subscriptions = vec![];
        if self.search_active
            && let Some(path) = &self.search_path
            && let Some(ocr) = &self.search_ocr
        {
            let search = SubtitleSearchSubscription {
                key: SubtitleSearchKey {
                    generation: self.search_gen,
                    path: path.clone(),
                },
                selection: self.search_selection,
                frame_rate: video_frame_rate,
                ocr: ocr.clone(),
                detector: self.search_detector,
                native_search_params: self.native_search_params,
                post_ocr_processing: self.post_ocr_processing,
                processing_resolution: self.processing_resolution,
            };
            subscriptions.push(Subscription::run_with(search, subtitle_search_stream));
        }

        subscriptions.push(iced::keyboard::listen().map(|x| match x {
            iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Character(x),
                modifiers: iced::keyboard::Modifiers::CTRL,
                repeat,
                ..
            } if x == "z" && repeat => Message::UndoEdit,
            _ => Message::None,
        }));
        Subscription::batch(subscriptions)
    }
}

pub fn to_srt(results: &[SubtitleResult]) -> String {
    extraction::to_srt(
        &results
            .iter()
            .map(|result| result.subtitle.clone())
            .collect::<Vec<_>>(),
    )
}

fn visible_result_range(
    scroll_offset: f32,
    viewport_height: f32,
    row_pitch: f32,
    result_count: usize,
) -> std::ops::Range<usize> {
    if result_count == 0 || !row_pitch.is_finite() || row_pitch <= 0.0 {
        return 0..0;
    }

    if !viewport_height.is_finite() || viewport_height <= 0.0 {
        return 0..INITIAL_VISIBLE_RESULT_ROWS.min(result_count);
    }

    let scroll_offset = if scroll_offset.is_finite() {
        scroll_offset.max(0.0)
    } else {
        0.0
    };
    let first_visible = ((scroll_offset / row_pitch).floor() as usize).min(result_count);
    let visible_end = (((scroll_offset + viewport_height) / row_pitch).ceil() as usize)
        .max(first_visible.saturating_add(1))
        .min(result_count);

    first_visible.saturating_sub(RESULT_ROW_OVERSCAN)
        ..visible_end
            .saturating_add(RESULT_ROW_OVERSCAN)
            .min(result_count)
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct SubtitleSearchKey {
    generation: usize,
    path: std::path::PathBuf,
}

struct SubtitleSearchSubscription {
    key: SubtitleSearchKey,
    selection: Option<iced::Rectangle>,
    frame_rate: f64,
    ocr: OcrHandle,
    detector: SubtitleDetector,
    native_search_params: NativeSearchParams,
    post_ocr_processing: bool,
    processing_resolution: ProcessingResolution,
}

impl std::hash::Hash for SubtitleSearchSubscription {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key.hash(state);
        self.selection
            .map(|selection| {
                (
                    selection.x.to_bits(),
                    selection.y.to_bits(),
                    selection.width.to_bits(),
                    selection.height.to_bits(),
                )
            })
            .hash(state);
        self.frame_rate.to_bits().hash(state);
        self.post_ocr_processing.hash(state);
        self.processing_resolution.hash(state);
    }
}

fn subtitle_search_stream(
    search: &SubtitleSearchSubscription,
) -> impl futures::Stream<Item = Message> + Send + use<> {
    let request = ExtractionRequest {
        input: search.key.path.clone(),
        crop: search.selection.map(|selection| CropRect {
            x: selection.x,
            y: selection.y,
            width: selection.width,
            height: selection.height,
        }),
        ocr: search.ocr.clone(),
        detector: search.detector,
        native_search_params: search.native_search_params,
        post_ocr_processing: search.post_ocr_processing,
        processing_resolution: search.processing_resolution,
        progress_interval: 100,
        include_progress_preview: true,
    };

    extraction::stream(request.clone()).map(|event| match event {
        extraction::Event::Started { total_frames } => Message::SearchStarted { total_frames },
        extraction::Event::Progress {
            frame,
            preview: Some(preview),
        } => Message::Progress { frame, preview },
        extraction::Event::Progress { .. } => Message::None,
        extraction::Event::SubtitleFound {
            subtitle,
            preview,
            replace_previous,
        } => Message::EventFound {
            subtitle,
            preview,
            replace_previous,
        },
        extraction::Event::Finished => Message::SearchDone,
        extraction::Event::Error(error) => Message::SearchError(error),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detection(start: u64, end: u64, text: &str, replace_previous: bool) -> Message {
        Message::EventFound {
            subtitle: Subtitle {
                start_timestamp: Duration::from_secs(start),
                end_timestamp: Duration::from_secs(end),
                text: text.to_owned(),
            },
            replace_previous,
            preview: RgbaImage::new(1, 1),
        }
    }

    #[test]
    fn post_ocr_processing_merges_adjacent_duplicate_text() {
        let mut model = Model::default();
        let config = Config::default();

        model.update(detection(0, 1, "same text", false), &config);
        model.update(detection(0, 3, "same text", true), &config);

        assert_eq!(model.results.len(), 1);
        assert_eq!(
            model.results[0].subtitle.end_timestamp,
            Duration::from_secs(3)
        );
    }

    #[test]
    fn post_ocr_processing_can_be_disabled() {
        let mut model = Model::default();
        let config = Config {
            post_ocr_processing: false,
            ..Config::default()
        };

        model.update(detection(0, 1, "same text", false), &config);
        model.update(detection(2, 3, "same text", false), &config);

        assert_eq!(model.results.len(), 2);
    }

    #[test]
    fn visible_range_only_contains_viewport_and_overscan() {
        let row_pitch = 176.0;

        assert_eq!(
            visible_result_range(5.0 * row_pitch, 2.0 * row_pitch, row_pitch, 20),
            2..10
        );
        assert_eq!(
            visible_result_range(0.0, 2.0 * row_pitch, row_pitch, 20),
            0..5
        );
        assert_eq!(
            visible_result_range(18.0 * row_pitch, 2.0 * row_pitch, row_pitch, 20),
            15..20
        );
    }

    #[test]
    fn visible_range_has_an_initial_window_before_viewport_is_known() {
        assert_eq!(
            visible_result_range(0.0, 0.0, RESULT_ROW_HEIGHT, 20),
            0..INITIAL_VISIBLE_RESULT_ROWS
        );
        assert_eq!(visible_result_range(0.0, 0.0, RESULT_ROW_HEIGHT, 2), 0..2);
        assert_eq!(visible_result_range(0.0, 0.0, RESULT_ROW_HEIGHT, 0), 0..0);
    }
}
