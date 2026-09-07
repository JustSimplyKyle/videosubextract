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

const RESULT_ROW_HEIGHT: f32 = 160.0;
const RESULT_PREVIEW_WIDTH: f32 = 320.0;
const RESULT_TIMING_WIDTH: f32 = 132.0;
const RESULT_ROW_OVERSCAN: usize = 3;

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct SubtitleId(pub(crate) u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum ResultsChanged {
    Full,
    Targeted(SubtitleId),
    Append(SubtitleId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SubtitleIndex(usize);

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SubtitleTableConfig {
    /// Use a compact, one-based cue number instead of the extracted frame preview.
    pub show_id_instead_of_preview: bool,
    /// Put the cue's time range above its text to preserve horizontal space.
    pub timestamp_above_text: bool,
    /// Render subtitle text without editing controls.
    pub read_only: bool,
    /// Show cached transformed text in read-only mode.
    pub show_transformed: bool,
}

#[derive(Debug, Clone)]
pub struct SubtitleResult {
    id: SubtitleId,
    pub subtitle: Subtitle,
    pub preview: widget::image::Handle,
    display: SubtitleDisplay,
}

#[derive(Debug, Clone)]
enum SubtitleDisplay {
    Editor(text_editor::Content),
    DisplayOnly {
        original: String,
        transformed: Option<String>,
    },
}

impl SubtitleResult {
    fn new_with_editor(id: SubtitleId, subtitle: Subtitle, preview: widget::image::Handle) -> Self {
        let editor_content = text_editor::Content::with_text(subtitle.text());

        Self {
            id,
            subtitle,
            preview,
            display: SubtitleDisplay::Editor(editor_content),
        }
    }

    fn new_with_readonly(
        id: SubtitleId,
        subtitle: Subtitle,
        preview: widget::image::Handle,
    ) -> Self {
        let original = subtitle.text().to_string();
        Self {
            id,
            subtitle,
            preview,
            display: SubtitleDisplay::DisplayOnly {
                original,
                transformed: None,
            },
        }
    }

    fn read_only_copy(&self) -> Self {
        Self::new_with_readonly(self.id, self.subtitle.clone(), self.preview.clone())
    }

    pub(crate) const fn id(&self) -> SubtitleId {
        self.id
    }

    pub(crate) fn text_for_display(&self, show_transformed: bool) -> &str {
        match &self.display {
            SubtitleDisplay::Editor(_) => self.subtitle.text(),
            SubtitleDisplay::DisplayOnly {
                original,
                transformed,
            } => {
                if show_transformed {
                    transformed.as_deref().unwrap_or(original)
                } else {
                    original
                }
            }
        }
    }

    pub(crate) fn original_text(&self) -> &str {
        match &self.display {
            SubtitleDisplay::Editor(_) => self.subtitle.text(),
            SubtitleDisplay::DisplayOnly { original, .. } => original,
        }
    }

    pub(crate) fn needs_transformation(&self) -> bool {
        matches!(
            &self.display,
            SubtitleDisplay::DisplayOnly {
                transformed: None,
                ..
            }
        )
    }

    fn set_transformed_text(&mut self, text: Option<String>) {
        if let SubtitleDisplay::DisplayOnly { transformed, .. } = &mut self.display {
            *transformed = text;
        }
    }
}

#[derive(Debug, Clone, Default)]
struct SubtitleResults(Vec<SubtitleResult>);

impl std::ops::Deref for SubtitleResults {
    type Target = [SubtitleResult];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl SubtitleResults {
    fn with_mut<R>(
        &mut self,
        f: impl FnOnce(&mut Vec<SubtitleResult>) -> R,
    ) -> (R, ResultsChanged) {
        (f(&mut self.0), ResultsChanged::Full)
    }

    fn with_mut_entry<R>(
        &mut self,
        id: SubtitleId,
        f: impl FnOnce(&mut SubtitleResult) -> R,
    ) -> Option<(R, ResultsChanged)> {
        self.0
            .iter_mut()
            .find(|result| result.id == id)
            .map(|result| (f(result), ResultsChanged::Targeted(id)))
    }

    fn push(&mut self, result: SubtitleResult) -> ResultsChanged {
        let id = result.id;
        self.0.push(result);
        ResultsChanged::Append(id)
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
    results: SubtitleResults,
    pub preview: Option<widget::image::Handle>,
    pub current_timestamp: Duration,
    pub done: bool,
    pub progress_bar: ProgressBar,
    scrollbar_jump_status: ScrollbarJumpStatus,
    next_result_id: u64,
    result_scroll_offset: f32,
    result_viewport_height: f32,
    zoomed_result_id: Option<SubtitleId>,
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
    Delete(SubtitleId),
    MergeWithPrevious(SubtitleId),
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
        id: SubtitleId,
        action: text_editor::Action,
    },
    ZoomIntoSubtitle {
        id: SubtitleId,
    },
    CloseSubtitlePreview,
    None,
}

pub enum Event {
    GoToPostProduction,
    SyncWithPostProduction(ResultsChanged),
    Run(Task<Message>),
    Error(eyre::Report),
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum VirtualRowKey {
    TopSpacer,
    Row { id: SubtitleId },
    BottomSpacer,
}

struct SubtitleTable<'a> {
    scroll_offset: f32,
    viewport_height: f32,
    results: &'a [SubtitleResult],
    config: SubtitleTableConfig,
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
        self.results[active_range.clone()].iter().enumerate().map(
            move |(relative_index, result)| {
                let index = SubtitleIndex(active_range.start + relative_index);
                (
                    VirtualRowKey::Row { id: result.id },
                    self.subtitle_row(index, result).apply(Self::wrap_row),
                )
            },
        )
    }

    fn timestamp(result: &SubtitleResult, render_atop: bool) -> Element<'_, Message> {
        fn precise_timestamp(timestamp: Duration) -> String {
            let total_seconds = timestamp.as_secs();
            let hours = total_seconds / 3_600;
            let minutes = (total_seconds % 3_600) / 60;
            let seconds = total_seconds % 60;
            let milliseconds = timestamp.subsec_millis();

            format!("{hours:02}:{minutes:02}:{seconds:02}.{milliseconds:03}")
        }
        if render_atop {
            widget::text::monotext(format!(
                "{} → {}",
                precise_timestamp(result.subtitle.start_timestamp),
                precise_timestamp(result.subtitle.end_timestamp)
            ))
            .into()
        } else {
            widget::column![
                widget::text::monotext(precise_timestamp(result.subtitle.start_timestamp)),
                widget::text::caption("↓"),
                widget::text::monotext(precise_timestamp(result.subtitle.end_timestamp)),
            ]
            .align_x(Alignment::Center)
            .width(Length::Fixed(RESULT_TIMING_WIDTH))
            .into()
        }
    }
    fn subtitle_row(
        &self,
        index: SubtitleIndex,
        result: &'a SubtitleResult,
    ) -> Element<'a, Message> {
        let spacing = cosmic::theme::spacing();

        let leading: Element<'a, Message> = if self.config.show_id_instead_of_preview {
            widget::text::title3((index.0 + 1).to_string())
                .apply(widget::container)
                .width(Length::Fixed(24.0))
                .center_y(Length::Fill)
                .class(theme::Container::Card)
                .into()
        } else {
            widget::image(result.preview.clone())
                .content_fit(iced::ContentFit::Contain)
                .width(Length::Fixed(RESULT_PREVIEW_WIDTH))
                .height(Length::Fill)
                .apply(widget::mouse_area)
                .on_press(Message::ZoomIntoSubtitle { id: result.id })
                .interaction(iced::mouse::Interaction::Pointer)
                .into()
        };

        let subtitle_text: Element<'a, Message> = if self.config.read_only {
            widget::text(result.text_for_display(self.config.show_transformed))
                .selectable()
                .width(Length::Fill)
                .height(Length::Fill)
                .align_y(iced::alignment::Vertical::Center)
                .apply(widget::scrollable)
                .width(Length::Fill)
                .height(Length::Fill)
                .apply(widget::container)
                .class(theme::Container::Secondary)
                .padding([spacing.space_s, spacing.space_m])
                .center_y(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            let SubtitleDisplay::Editor(content) = &result.display else {
                return widget::text(result.subtitle.text()).into();
            };
            Self::text_editor(result.id, content)
        };

        let timing_and_text: Element<'a, Message> = if self.config.timestamp_above_text {
            widget::column![Self::timestamp(result, true), subtitle_text]
                .spacing(spacing.space_xxs)
                .height(Length::Fill)
                .width(Length::Fill)
                .into()
        } else {
            widget::row![Self::timestamp(result, false), subtitle_text]
                .spacing(spacing.space_m)
                .height(Length::Fill)
                .width(Length::Fill)
                .align_y(Alignment::Center)
                .into()
        };

        widget::row!(leading, timing_and_text)
            .push_maybe((!self.config.read_only).then(|| Self::toolbar(index, result.id)))
            .spacing(spacing.space_m)
            .padding([spacing.space_s, spacing.space_m])
            .height(Length::Fill)
            .align_y(Alignment::Center)
            .into()
    }
    fn toolbar(index: SubtitleIndex, id: SubtitleId) -> Element<'static, Message> {
        widget::row::with_capacity(2)
            .push_maybe((index.0 != 0).then_some(Self::merge_with_previous(id)))
            .push(Self::delete(id))
            .spacing(cosmic::theme::spacing().space_s)
            .align_y(Alignment::Center)
            .into()
    }
    fn text_editor(
        id: SubtitleId,
        content: &'a widget::text_editor::Content,
    ) -> Element<'a, Message> {
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
    fn delete(id: SubtitleId) -> Element<'static, Message> {
        widget::button::icon(widget::icon::from_name("edit-delete-symbolic"))
            .on_press(Message::Delete(id))
            .class(cosmic::theme::Button::Destructive)
            .into()
    }
    fn merge_with_previous(id: SubtitleId) -> Element<'static, Message> {
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
        let config = SubtitleTableConfig {
            timestamp_above_text: true,
            ..Default::default()
        };
        self.model.results_table(config, true)
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
    pub fn results(&self) -> &[SubtitleResult] {
        &self.results
    }

    fn result_index(&self, id: SubtitleId) -> Option<SubtitleIndex> {
        self.results
            .iter()
            .position(|result| result.id == id)
            .map(SubtitleIndex)
    }

    pub(crate) fn results_table(
        &self,
        config: SubtitleTableConfig,
        show_jump_to_end: bool,
    ) -> Element<'_, Message> {
        let scrollable_id = iced::id::Id::new("scrollable");
        let jump_to_end = (show_jump_to_end
            && self.scrollbar_jump_status == ScrollbarJumpStatus::DisplayButton)
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
            scroll_offset: self.result_scroll_offset,
            viewport_height: self.result_viewport_height,
            results: &self.results,
            config,
        }
        .view()
        .apply(widget::container)
        .padding(iced::Padding::ZERO.right(20))
        .height(Length::Fill)
        .apply(widget::scrollable)
        .on_scroll(SubtitleView::scrolled)
        .id(scrollable_id)
        .apply(Element::from);

        iced::widget::stack![results, jump_to_end]
            .apply(widget::container)
            .class(theme::Container::List)
            .height(Length::Fill)
            .into()
    }
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
        let _ = self.results.with_mut(Vec::clear);
        self.preview = None;
        self.current_timestamp = Duration::ZERO;
        self.done = false;
        self.edit_history.clear();
        self.progress_bar.reset();
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
                if replace_previous && let Some(previous_id) = self.results.last().map(|x| x.id) {
                    let (_, changed) = self.results.with_mut(|results| {
                        let previous = results.last_mut().unwrap();
                        *previous = SubtitleResult::new_with_editor(previous_id, subtitle, preview);
                    });
                    return Event::SyncWithPostProduction(changed);
                }
                let id = SubtitleId(self.next_result_id);
                self.next_result_id = self.next_result_id.wrapping_add(1);
                let changed = self
                    .results
                    .push(SubtitleResult::new_with_editor(id, subtitle, preview));
                Event::SyncWithPostProduction(changed)
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
            Message::Delete(id) => {
                if let Some(SubtitleIndex(index)) = self.result_index(id) {
                    let (result, changed) = self.results.with_mut(|results| results.remove(index));
                    self.edit_history
                        .push(SubtitleEdit::Delete { index, result });
                    return Event::SyncWithPostProduction(changed);
                }
                Event::None
            }
            Message::MergeWithPrevious(id) => {
                if let Some(SubtitleIndex(index)) = self.result_index(id)
                    && index > 0
                {
                    let ((result, previous_end_timestamp), changed) =
                        self.results.with_mut(|results| {
                            let result = results.remove(index);
                            let previous_end_timestamp = std::mem::replace(
                                &mut results[index - 1].subtitle.end_timestamp,
                                result.subtitle.end_timestamp,
                            );
                            (result, previous_end_timestamp)
                        });
                    self.edit_history.push(SubtitleEdit::MergeWithPrevious {
                        index,
                        previous_end_timestamp,
                        result,
                    });
                    return Event::SyncWithPostProduction(changed);
                }
                Event::None
            }
            Message::UndoEdit => {
                if let Some(edit) = self.edit_history.pop() {
                    let (_, changed) = self.results.with_mut(|results| match edit {
                        SubtitleEdit::Delete { index, result } => {
                            results.insert(index.min(results.len()), result);
                        }
                        SubtitleEdit::MergeWithPrevious {
                            index,
                            previous_end_timestamp,
                            result,
                        } => {
                            if let Some(previous) = index
                                .checked_sub(1)
                                .and_then(|index| results.get_mut(index))
                            {
                                previous.subtitle.end_timestamp = previous_end_timestamp;
                                results.insert(index.min(results.len()), result);
                            }
                        }
                    });
                    return Event::SyncWithPostProduction(changed);
                }
                Event::None
            }
            Message::SubtitleContentEdit { id, action } => {
                if let Some(((), changed)) = self.results.with_mut_entry(id, |result| {
                    if let SubtitleDisplay::Editor(content) = &mut result.display {
                        content.perform(action);
                        result.subtitle.set_text(content.text());
                    }
                }) {
                    return Event::SyncWithPostProduction(changed);
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

    pub(crate) fn sync_read_only_results(
        &mut self,
        source: &[SubtitleResult],
        changed: ResultsChanged,
    ) -> ResultsChanged {
        let applied = match changed {
            ResultsChanged::Full => {
                self.results =
                    SubtitleResults(source.iter().map(SubtitleResult::read_only_copy).collect());
                ResultsChanged::Full
            }
            ResultsChanged::Targeted(id) => {
                let source_result = source.iter().find(|result| result.id == id);
                let target = self.results.0.iter_mut().find(|result| result.id == id);
                match (source_result, target) {
                    (Some(source_result), Some(target)) => {
                        *target = source_result.read_only_copy();
                        ResultsChanged::Targeted(id)
                    }
                    _ => {
                        self.results = SubtitleResults(
                            source.iter().map(SubtitleResult::read_only_copy).collect(),
                        );
                        ResultsChanged::Full
                    }
                }
            }
            ResultsChanged::Append(id) => {
                let source_result = source.iter().find(|result| result.id == id);
                if self.results.len() + 1 == source.len()
                    && let Some(source_result) = source_result
                    && !self.results.iter().any(|result| result.id == id)
                {
                    self.results.0.push(source_result.read_only_copy());
                    ResultsChanged::Append(id)
                } else {
                    self.results = SubtitleResults(
                        source.iter().map(SubtitleResult::read_only_copy).collect(),
                    );
                    ResultsChanged::Full
                }
            }
        };
        self.result_scroll_offset = 0.0;
        self.result_viewport_height = 0.0;
        self.scrollbar_jump_status = ScrollbarJumpStatus::NoShow;
        applied
    }

    pub(crate) fn set_transformed_text(&mut self, id: SubtitleId, text: Option<String>) -> bool {
        let Some(result) = self.results.0.iter_mut().find(|result| result.id == id) else {
            return false;
        };
        result.set_transformed_text(text);
        true
    }

    pub(crate) fn clear_transformed_text(&mut self) {
        for result in &mut self.results.0 {
            result.set_transformed_text(None);
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
            .map(|result| {
                let mut subtitle = result.subtitle.clone();
                subtitle.set_text(result.text_for_display(true).to_owned());
                subtitle
            })
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
            subtitle: Subtitle::new(
                Duration::from_secs(start),
                Duration::from_secs(end),
                text.to_owned(),
            ),
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

    fn model_with_three_results() -> (Model, Config, [SubtitleId; 3]) {
        let mut model = Model::default();
        let config = Config::default();

        model.update(detection(0, 1, "first", false), &config);
        model.update(detection(2, 3, "second", false), &config);
        model.update(detection(4, 5, "third", false), &config);

        let ids = [
            model.results[0].id,
            model.results[1].id,
            model.results[2].id,
        ];
        (model, config, ids)
    }

    #[test]
    fn deletion_resolves_stable_id_after_indices_shift() {
        let (mut model, config, [first_id, second_id, third_id]) = model_with_three_results();

        model.update(Message::Delete(first_id), &config);
        model.update(Message::Delete(third_id), &config);

        assert_eq!(model.results.len(), 1);
        assert_eq!(model.results[0].id, second_id);
        assert_eq!(model.results[0].subtitle.text(), "second");
    }

    #[test]
    fn edit_targets_stable_id_after_indices_shift() {
        let (mut model, config, [first_id, second_id, third_id]) = model_with_three_results();

        model.update(Message::Delete(first_id), &config);
        model.update(
            Message::SubtitleContentEdit {
                id: second_id,
                action: text_editor::Action::Edit(text_editor::Edit::Insert('!')),
            },
            &config,
        );

        let second = model
            .results
            .iter()
            .find(|result| result.id == second_id)
            .expect("the edited subtitle remains present");
        let third = model
            .results
            .iter()
            .find(|result| result.id == third_id)
            .expect("the other subtitle remains present");
        assert!(second.subtitle.text().contains('!'));
        assert_eq!(third.subtitle.text(), "third");
    }

    #[test]
    fn stale_edit_event_does_not_edit_replacement_at_same_index() {
        let (mut model, config, [_first_id, second_id, third_id]) = model_with_three_results();

        model.update(Message::Delete(second_id), &config);
        model.update(
            Message::SubtitleContentEdit {
                id: second_id,
                action: text_editor::Action::Edit(text_editor::Edit::Insert('!')),
            },
            &config,
        );

        let third = model
            .results
            .iter()
            .find(|result| result.id == third_id)
            .expect("the following subtitle remains present");
        assert_eq!(third.subtitle.text(), "third");
        assert!(!model.results.iter().any(|result| result.id == second_id));
    }

    #[test]
    fn merge_resolves_stable_id_after_indices_shift() {
        let (mut model, config, [first_id, second_id, third_id]) = model_with_three_results();

        model.update(Message::Delete(first_id), &config);
        model.update(Message::MergeWithPrevious(third_id), &config);

        assert_eq!(model.results.len(), 1);
        assert_eq!(model.results[0].id, second_id);
        assert_eq!(
            model.results[0].subtitle.end_timestamp,
            Duration::from_secs(5)
        );
    }

    #[test]
    fn undo_merge_restores_stable_identity_and_order() {
        let (mut model, config, [first_id, second_id, third_id]) = model_with_three_results();

        model.update(Message::MergeWithPrevious(third_id), &config);
        model.update(Message::UndoEdit, &config);

        assert_eq!(
            model
                .results
                .iter()
                .map(|result| result.id)
                .collect::<Vec<_>>(),
            vec![first_id, second_id, third_id]
        );
        assert_eq!(
            model.results[1].subtitle.end_timestamp,
            Duration::from_secs(3)
        );
    }
}
