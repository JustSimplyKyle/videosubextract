use crate::config::{ProcessingResolution, SubtitleDetector};
use crate::extraction::{self, OcrHandle, Request as ExtractionRequest, Subtitle};
use crate::native_video_sub_finder::NativeSearchParams;
use crate::video_player::CropRect;
use cosmic::theme;
use cosmic::widget::text_editor;
use iced::futures::StreamExt;
use image::RgbaImage;

use super::*;
use std::time::Duration;

const JUMP_TO_END_DELAY: Duration = Duration::from_secs(3);

const RESULT_ROW_HEIGHT: f32 = 120.0;
const RESULT_PREVIEW_WIDTH: f32 = 180.0;
const RESULT_TIMING_WIDTH: f32 = 132.0;
const RESULT_ROW_OVERSCAN: usize = 3;

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
    pub current_timestamp: Duration,
    pub done: bool,
    pub progress_bar: ProgressBar,
    scrollbar_jump_status: ScrollbarJumpStatus,
    next_result_id: u64,
    result_scroll_offset: f32,
    result_viewport_height: f32,
    zoomed_result_id: Option<u64>,
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
        timestamp: Duration,

        #[debug("{}x{}", preview.width(), preview.height())]
        preview: RgbaImage,
    },
    EventFound {
        subtitle: Subtitle,
        replace_previous: bool,
        #[debug("{}x{}", preview.width(), preview.height())]
        preview: RgbaImage,
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
    ZoomIntoSubtitle {
        id: u64,
    },
    CloseSubtitlePreview,
    None,
}

pub enum Event {
    GoToPostProduction,
    Run(Task<Message>),
    Error(eyre::Report),
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum VirtualRowKey {
    TopSpacer,
    Row { id: usize },
    BottomSpacer,
}

struct SubtitleTable<'a> {
    scroll_offset: f32,
    viewport_height: f32,
    results: &'a [SubtitleResult],
}

impl<'a> SubtitleTable<'a> {
    fn row_pitch() -> f32 {
        RESULT_ROW_HEIGHT
    }
    fn visible_result_range(&self, result_count: usize) -> std::ops::Range<usize> {
        let first_visible =
            ((self.scroll_offset / Self::row_pitch()).floor() as usize).min(result_count);

        let visible_end = (((self.scroll_offset + self.viewport_height) / Self::row_pitch()).ceil()
            as usize)
            .max(first_visible.saturating_add(1))
            .min(result_count);

        first_visible.saturating_sub(RESULT_ROW_OVERSCAN)
            ..visible_end
                .saturating_add(RESULT_ROW_OVERSCAN)
                .min(result_count)
    }
    fn spacer(amount: usize) -> Element<'static, Message> {
        let height = amount as f32 * Self::row_pitch();

        widget::Space::new().height(Length::Fixed(height)).into()
    }

    fn wrap_row(item: Element<'a, Message>) -> Element<'a, Message> {
        widget::column![item, widget::divider::horizontal::light()]
            .height(Length::Fixed(Self::row_pitch()))
            .into()
    }

    fn active_subtitles(
        &self,
        active_range: std::ops::Range<usize>,
    ) -> impl Iterator<Item = (VirtualRowKey, Element<'a, Message>)> {
        self.results[active_range.clone()]
            .iter()
            .enumerate()
            .map(move |(relative_id, result)| {
                let id = active_range.start + relative_id;
                (
                    VirtualRowKey::Row { id },
                    self.subtitle_row(id, result).apply(Self::wrap_row),
                )
            })
    }
    fn timestamp(result: &SubtitleResult) -> Element<'_, Message> {
        fn precise_timestamp(timestamp: Duration) -> String {
            let total_seconds = timestamp.as_secs();
            let hours = total_seconds / 3_600;
            let minutes = (total_seconds % 3_600) / 60;
            let seconds = total_seconds % 60;
            let milliseconds = timestamp.subsec_millis();

            format!("{hours:02}:{minutes:02}:{seconds:02}.{milliseconds:03}")
        }

        widget::column![
            widget::text::monotext(precise_timestamp(result.subtitle.start_timestamp)),
            widget::text::caption("↓"),
            widget::text::monotext(precise_timestamp(result.subtitle.end_timestamp)),
        ]
        .align_x(Alignment::Center)
        .width(Length::Fixed(RESULT_TIMING_WIDTH))
        .into()
    }
    fn subtitle_row(&self, id: usize, result: &'a SubtitleResult) -> Element<'a, Message> {
        let toolbar = Self::toolbar(id);
        let spacing = cosmic::theme::spacing();

        widget::row!(
            widget::image(result.preview.clone())
                .content_fit(iced::ContentFit::Contain)
                .width(Length::Fixed(RESULT_PREVIEW_WIDTH))
                .height(Length::Fill)
                .apply(widget::mouse_area)
                .on_press(Message::ZoomIntoSubtitle { id: result.id })
                .interaction(iced::mouse::Interaction::Pointer),
            Self::timestamp(result),
            Self::text_editor(id, &result.editor_content),
            toolbar,
        )
        .spacing(spacing.space_m)
        .padding([spacing.space_s, spacing.space_m])
        .height(Length::Fill)
        .align_y(Alignment::Center)
        .into()
    }
    fn toolbar(id: usize) -> Element<'static, Message> {
        widget::row::with_capacity(2)
            .push_maybe((id != 0).then_some(Self::merge_with_previous(id)))
            .push(Self::delete(id))
            .spacing(cosmic::theme::spacing().space_s)
            .align_y(Alignment::Center)
            .into()
    }
    fn text_editor(id: usize, content: &'a widget::text_editor::Content) -> Element<'a, Message> {
        widget::text_editor::text_editor(content)
            .on_action(move |action| Message::SubtitleContentEdit { id, action })
            .height(Length::Fill)
            .min_height(48.0)
            .style(|x, y| {
                use iced::widget::text_editor::Catalog;
                let mut style = x.style(&theme::iced::TextEditor::default(), y);
                style.border.width = 2.0;
                style
            })
            .apply(Element::from)
    }
    fn delete(id: usize) -> Element<'static, Message> {
        widget::button::icon(widget::icon::from_name("edit-delete-symbolic"))
            .on_press(Message::Delete(id))
            .class(cosmic::theme::Button::Destructive)
            .into()
    }
    fn merge_with_previous(id: usize) -> Element<'static, Message> {
        widget::button::icon(widget::icon::from_name("go-up-symbolic"))
            .on_press(Message::MergeWithPrevious(id))
            .class(cosmic::theme::Button::Icon)
            .into()
    }
    fn view(self) -> Element<'a, Message> {
        let visible_range = self.visible_result_range(self.results.len());
        let active_subtitles = self.active_subtitles(visible_range.clone());

        let top_spacer = Self::spacer(visible_range.start);
        let bottom_spacer = Self::spacer(self.results.len().saturating_sub(visible_range.end));

        // we need a keyed column here because without it, state such as "is text_editor selected" will be lost in scrolling since iced treats every "first row" as the same.
        iced::widget::keyed::column::Column::with_capacity(visible_range.len() + 2)
            .push(VirtualRowKey::TopSpacer, top_spacer)
            .extend(active_subtitles)
            .push(VirtualRowKey::BottomSpacer, bottom_spacer)
            .width(Length::Fill)
            .into()
    }
}

struct SubtitleView<'a> {
    model: &'a Model,
    video_duration: Duration,
}

impl<'a> SubtitleView<'a> {
    fn format_eta(duration: Duration) -> String {
        let seconds = duration.as_secs();

        if seconds < 60 {
            fl!("eta-seconds", seconds = seconds)
        } else {
            // Round the seconds to the nearest minute.
            let minutes = ((seconds + 30) / 60).max(1);

            fl!("eta-minutes", minutes = minutes)
        }
    }

    fn status(&self) -> Element<'a, Message> {
        self.model
            .progress_bar
            .set_length(self.video_duration.as_millis() as u64);

        if self.model.done {
            widget::text(fl!(
                "complete-subtitles-found",
                count = self.model.results.len()
            ))
            .class(cosmic::theme::Text::Accent)
            .into()
        } else if self.model.search_active {
            let status_text = widget::text(fl!(
                "elapsed-status",
                elapsed = self.model.progress_bar.elapsed().apply(format_duration),
                igt = self.model.current_timestamp.apply(format_duration),
                eta = self.model.progress_bar.eta().apply(Self::format_eta)
            ))
            .class(cosmic::theme::Text::Accent);

            let progress_bar = widget::progress_bar::determinate_linear(
                self.model.current_timestamp.as_secs_f32() / self.video_duration.as_secs_f32(),
            )
            .width(Length::Fill);

            widget::row!(status_text, progress_bar)
                .spacing(cosmic::theme::spacing().space_s)
                .align_y(Alignment::Center)
                .width(Length::Fill)
                .into()
        } else {
            widget::text(fl!("no-active-search"))
                .class(cosmic::theme::Text::Accent)
                .into()
        }
    }

    fn controls(&self) -> Element<'a, Message> {
        let to_post_prod = widget::button::text(fl!("post-production"))
            .class(cosmic::theme::Button::Suggested)
            .on_press_maybe((!self.model.search_active).then_some(Message::GoToPostProduction));
        let undo_edit = widget::button::icon(icon::from_name("edit-undo-symbolic"))
            .on_press_maybe((!self.model.edit_history.is_empty()).then_some(Message::UndoEdit));

        widget::row![self.status(), undo_edit, to_post_prod]
            .spacing(cosmic::theme::spacing().space_s)
            .align_y(Alignment::Center)
            .into()
    }

    fn preview_card(title: String, handle: &'a widget::image::Handle) -> Element<'a, Message> {
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
        .into()
    }

    fn header(&self) -> Option<Element<'a, Message>> {
        self.model.preview.as_ref().map(|handle| {
            widget::Row::new()
                .spacing(cosmic::theme::spacing().space_s)
                .push(Self::preview_card(fl!("view"), handle))
                .push_maybe(
                    self.model
                        .results
                        .last()
                        .map(|result| Self::preview_card(fl!("current"), &result.preview)),
                )
                .into()
        })
    }

    fn scrolled(viewport: iced::widget::scrollable::Viewport) -> Message {
        let content_fits = viewport.content_bounds().height <= viewport.bounds().height + 1.0;
        let at_end = content_fits || viewport.relative_offset().y >= 0.999;
        Message::Scrolled {
            at_end,
            offset: viewport.absolute_offset().y,
            viewport_height: viewport.bounds().height,
        }
    }

    fn results(&self) -> Element<'a, Message> {
        let scrollable_id = iced::id::Id::new("scrollable");
        let jump_to_end = (self.model.scrollbar_jump_status == ScrollbarJumpStatus::DisplayButton)
            .then_some(
                widget::button::text(fl!("jump-to-latest"))
                    .class(cosmic::theme::Button::Suggested)
                    .on_press(Message::JumpToEnd {
                        id: scrollable_id.clone(),
                    })
                    .apply(iced::widget::bottom_right)
                    .padding(cosmic::theme::spacing().space_m),
            );

        let results = SubtitleTable {
            scroll_offset: self.model.result_scroll_offset,
            viewport_height: self.model.result_viewport_height,
            results: &self.model.results,
        }
        .view()
        .apply(widget::container)
        .padding(iced::Padding::ZERO.right(20))
        .height(Length::Fill)
        .apply(widget::scrollable)
        .on_scroll(Self::scrolled)
        .id(scrollable_id)
        .apply(Element::from);

        iced::widget::stack![results, jump_to_end]
            .apply(widget::container)
            .class(theme::Container::List)
            .height(Length::Fill)
            .into()
    }

    fn zoomed_preview(result: &'a SubtitleResult) -> Element<'a, Message> {
        widget::dialog()
            .title(fl!("subtitle-preview"))
            .control(
                widget::image(result.preview.clone())
                    .expand(true)
                    .content_fit(iced::ContentFit::Contain),
            )
            .primary_action(
                widget::button::icon(icon::from_name("window-close-symbolic"))
                    .class(theme::Button::Icon)
                    .on_press(Message::CloseSubtitlePreview),
            )
            .width(Length::Fill)
            .max_width(1000.0)
            .apply(widget::container)
            .center(Length::Fill)
            .style(|_| widget::container::background(iced::Color::from_rgba(0., 0., 0., 0.45)))
            .into()
    }

    fn view(&self) -> Element<'a, Message> {
        let content: Element<'a, Message> =
            widget::column![self.header(), self.controls(), self.results()]
                .spacing(cosmic::theme::spacing().space_s)
                .into();

        let zoomed_result = self
            .model
            .zoomed_result_id
            .and_then(|id| self.model.results.iter().find(|result| result.id == id));

        if let Some(result) = zoomed_result {
            iced::widget::stack![content, Self::zoomed_preview(result)].into()
        } else {
            content
        }
    }
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
        self.current_timestamp = Duration::ZERO;
        self.done = false;
        self.edit_history.clear();
        self.progress_bar.set_elapsed(Duration::ZERO);
        self.scrollbar_jump_status = ScrollbarJumpStatus::NoShow;
        self.zoomed_result_id = None;
        self.next_result_id = 0;
        self.result_scroll_offset = 0.0;
        self.result_viewport_height = 0.0;
    }

    pub fn update(&mut self, message: Message, config: &Config) -> Event {
        self.set_ocr_model(config.ocr_model.clone());
        match message {
            Message::Progress { timestamp, preview } => {
                self.progress_bar.set_position(timestamp.as_millis() as u64);
                self.current_timestamp = timestamp;
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
            Message::ZoomIntoSubtitle { id } => {
                self.zoomed_result_id = Some(id);
                Event::None
            }
            Message::CloseSubtitlePreview => {
                self.zoomed_result_id = None;
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

    pub fn view(&self, video_duration: Duration) -> Element<'_, Message> {
        SubtitleView {
            model: self,
            video_duration,
        }
        .view()
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
            } if x == "z" && !repeat => Message::UndoEdit,
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

    extraction::stream(request).map(|event| match event {
        extraction::Event::Progress {
            timestamp,
            preview: Some(preview),
        } => Message::Progress { timestamp, preview },
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
}
