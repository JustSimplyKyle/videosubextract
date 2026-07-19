use crate::config::SubtitleDetector;
use crate::native_video_sub_finder::{
    NativeSearchParams, NativeSubtitleEvent, find_subtitles_with,
};
use crate::ocr::OcrProvider;
use crate::subfinder::{Params as RustSearchParams, SubtitleSearch};
use cosmic::theme;
use iced::futures::{SinkExt, StreamExt};
use iced::widget::text_editor;

use super::*;
use std::sync::{Arc, RwLock};
use std::time::Duration;

const JUMP_TO_END_DELAY: Duration = Duration::from_secs(3);
const OCR_PARALELLISM: usize = 4;

#[derive(Debug, Clone)]
pub struct SubtitleResult {
    pub start_timestamp: Duration,
    pub end_timestamp: Duration,
    pub text: String,
    pub preview: widget::image::Handle,
    editor_content: text_editor::Content,
}

impl SubtitleResult {
    fn new(
        start_timestamp: Duration,
        end_timestamp: Duration,
        text: String,
        preview: widget::image::Handle,
    ) -> Self {
        let editor_content = text_editor::Content::with_text(&text);

        Self {
            start_timestamp,
            end_timestamp,
            text,
            preview,
            editor_content,
        }
    }

    pub(crate) fn set_text(&mut self, text: String) {
        self.editor_content = text_editor::Content::with_text(&text);
        self.text = text;
    }
}

#[derive(Default)]
pub struct Model {
    pub search_active: bool,
    pub search_gen: usize,
    pub search_path: Option<std::path::PathBuf>,
    pub search_selection: Option<iced::Rectangle>,
    search_ocr: Option<RuntimeOcrModel>,
    search_detector: SubtitleDetector,
    native_search_params: NativeSearchParams,
    pub results: Vec<SubtitleResult>,
    pub preview: Option<widget::image::Handle>,
    pub current_frame: usize,
    pub done: bool,
    pub progress_bar: ProgressBar,
    scrollbar_jump_status: ScrollbarJumpStatus,
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

#[derive(Debug, Clone)]
pub enum Message {
    Progress {
        frame: usize,
        preview: RgbaImage,
    },
    EventFound {
        start_timestamp: Duration,
        end_timestamp: Duration,
        text: String,
        preview: RgbaImage,
    },
    Delete(usize),
    MergeWithPrevious(usize),
    UndoEdit,
    Scrolled {
        at_end: bool,
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
    None,
}

impl Model {
    pub fn start_search(
        &mut self,
        path: std::path::PathBuf,
        selection: Option<iced::Rectangle>,
        ocr: OcrModel,
        detector: SubtitleDetector,
        native_search_params: NativeSearchParams,
    ) {
        self.search_active = true;
        self.search_gen += 1;
        self.search_path = Some(path);
        self.search_selection = selection;
        self.search_ocr = Some(RuntimeOcrModel::new(ocr));
        self.search_detector = detector;
        self.native_search_params = native_search_params;
        self.results.clear();
        self.preview = None;
        self.current_frame = 0;
        self.done = false;
        self.edit_history.clear();
        self.progress_bar.set_elapsed(Duration::ZERO);
        self.scrollbar_jump_status = ScrollbarJumpStatus::NoShow;
    }

    pub fn update(&mut self, message: Message, config: &Config) -> Event {
        self.set_ocr_model(config.ocr_model);
        match message {
            Message::Progress { frame, preview } => {
                self.progress_bar.set_position(self.current_frame as u64);
                self.current_frame = frame;
                self.preview = Some(widget::image::Handle::from_rgba(
                    preview.width(),
                    preview.height(),
                    preview.into_raw(),
                ));
                Event::None
            }
            Message::EventFound {
                start_timestamp,
                end_timestamp,
                text,
                preview,
            } => {
                let preview = widget::image::Handle::from_rgba(
                    preview.width(),
                    preview.height(),
                    preview.into_raw(),
                );
                if config.post_ocr_processing
                    && let Some(prev) = self.results.last_mut()
                    && (start_timestamp - prev.end_timestamp) < Duration::from_millis(5000)
                    && prev.text.trim() == text.trim()
                {
                    *prev = SubtitleResult::new(prev.start_timestamp, end_timestamp, text, preview);
                    return Event::None;
                }
                if text.trim().is_empty() {
                    return Event::None;
                }
                self.results.push(SubtitleResult::new(
                    start_timestamp,
                    end_timestamp,
                    text,
                    preview,
                ));
                Event::None
            }
            Message::SearchDone => {
                self.search_active = false;
                self.done = true;
                self.preview = None;
                Event::None
            }
            Message::SearchError(e) => {
                eprintln!("subtitle search error: {e}");
                self.search_active = false;
                Event::None
            }
            Message::GoToPostProduction => Event::GoToPostProduction,
            Message::Scrolled { at_end } => {
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
                        &mut self.results[x - 1].end_timestamp,
                        result.end_timestamp,
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
                                previous.end_timestamp = previous_end_timestamp;
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
                    result.text = result.editor_content.text();
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

        const TOOLBAR_SIZE: f32 = 48.0;

        let grid = widget::responsive(move |size| {
            let horizontal_padding = 80.0; // [0, 40] on both sides
            let column_gaps = f32::from(spacing.space_s) * 2.0;

            let available_width =
                (size.width - horizontal_padding - column_gaps - TOOLBAR_SIZE).max(0.0);

            let text_width = (available_width * 0.35).max(160.0).min(available_width);

            let image_width = (available_width - text_width).max(0.0);

            self.results
                .iter()
                .enumerate()
                .fold(widget::grid(), |grid, (id, result)| {
                    let t_start = result.start_timestamp.as_secs_f64();
                    let t_end = result.end_timestamp.as_secs_f64();

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
                        .width(image_width)
                        .height(Length::Shrink);

                    let timeline = widget::text(format!("{t_start:.1}s – {t_end:.1}s"));

                    let ocr = widget::text_editor(&result.editor_content)
                        .on_action(move |action| Message::SubtitleContentEdit { id, action })
                        .height(Length::Shrink)
                        .min_height(48.0)
                        .class(cosmic::theme::iced::TextEditor::Custom(Box::new(|x, y| {
                            use iced::widget::text_editor;
                            let mut style = text_editor::Catalog::style(
                                x,
                                &theme::iced::TextEditor::Default,
                                y,
                            );
                            style.border.width = 2.0;
                            style
                        })))
                        .apply(Element::from);

                    grid.push(toolbar)
                        .push(image)
                        .push(
                            widget::column![timeline, ocr]
                                .spacing(space_s / 2)
                                .width(text_width)
                                .align_x(Alignment::Start),
                        )
                        .insert_row()
                })
                .row_spacing(spacing.space_m)
                .column_spacing(spacing.space_s)
                .padding([0, 40].into())
                .row_alignment(Alignment::Center)
                .into()
        });

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

        let results = grid
            .apply(widget::container)
            .padding(iced::Padding::ZERO.right(60))
            .height(Length::Fill)
            .apply(widget::scrollable)
            .on_scroll(|viewport| {
                let content_fits =
                    viewport.content_bounds().height <= viewport.bounds().height + 1.0;
                let at_end = content_fits || viewport.relative_offset().y >= 0.999;
                Message::Scrolled { at_end }
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
    use std::fmt::Write;

    results
        .iter()
        .enumerate()
        .fold(String::new(), |mut out, (i, r)| {
            let start = frame_to_srt_timestamp(r.start_timestamp);
            let end = frame_to_srt_timestamp(r.end_timestamp);
            let _ = write!(
                out,
                "{}\n{} --> {}\n{}\n\n",
                i + 1,
                start,
                end,
                r.text.trim()
            );
            out
        })
}

fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

fn frame_to_srt_timestamp(frame: Duration) -> String {
    let total_ms = frame.as_millis();
    let ms = total_ms % 1000;
    let s = (total_ms / 1000) % 60;
    let m = (total_ms / 60_000) % 60;
    let h = total_ms / 3_600_000;
    format!("{h:02}:{m:02}:{s:02},{ms:03}")
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
    ocr: RuntimeOcrModel,
    detector: SubtitleDetector,
    native_search_params: NativeSearchParams,
}

#[derive(Clone)]
struct RuntimeOcrModel(Arc<RwLock<OcrModel>>);

impl RuntimeOcrModel {
    fn new(model: OcrModel) -> Self {
        Self(Arc::new(RwLock::new(model)))
    }

    fn read(&self) -> OcrModel {
        *self.0.read().unwrap_or_else(|error| error.into_inner())
    }

    fn set(&self, model: OcrModel) {
        *self.0.write().unwrap_or_else(|error| error.into_inner()) = model;
    }
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
    }
}

struct ProgressIter<I> {
    inner: I,
    tx: tokio::sync::mpsc::Sender<Message>,
    count: usize,
    interval: usize,
}

impl<I: Iterator<Item = VideoFrame>> Iterator for ProgressIter<I> {
    type Item = VideoFrame;

    fn next(&mut self) -> Option<Self::Item> {
        let mat = self.inner.next()?;
        self.count += 1;
        if self.count.is_multiple_of(self.interval)
            && let Ok(rgba) = super::mat_to_image_handle(&mat.mat)
        {
            let _ = self.tx.try_send(Message::Progress {
                frame: self.count,
                preview: rgba,
            });
        }
        Some(mat)
    }
}

fn subtitle_search_stream(
    search: &SubtitleSearchSubscription,
) -> impl futures::Stream<Item = Message> + Send + use<> {
    let path = search.key.path.clone();
    let selection = search.selection;
    let ocr = search.ocr.clone();
    let detector = search.detector;
    let native_search_params = search.native_search_params;
    let preview_interval = 100;

    iced::stream::channel(
        OCR_PARALELLISM,
        async move |mut tx: futures::channel::mpsc::Sender<Message>| {
            let (btx1, mut brx) = tokio::sync::mpsc::channel::<Message>(OCR_PARALELLISM);
            let btx2 = btx1.clone();
            let (sender, receiver) =
                tokio::sync::mpsc::channel::<NativeSubtitleEvent>(OCR_PARALELLISM);
            let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();

            tokio::task::spawn_blocking(move || {
                let input = match ffmpeg_the_third::format::input(&path) {
                    Ok(x) => x,
                    Err(e) => {
                        let _ = btx1.blocking_send(Message::SearchError(e.to_string()));
                        return;
                    }
                };
                let (_, iter) = match create_video_player::<false>(input, selection) {
                    Ok(x) => x,
                    Err(e) => {
                        let _ = btx1.blocking_send(Message::SearchError(e.to_string()));
                        return;
                    }
                };
                let frame_iter = ProgressIter {
                    inner: iter.filter_map(Result::ok),
                    tx: btx1.clone(),
                    count: 0,
                    interval: preview_interval,
                };

                let search_result = match detector {
                    SubtitleDetector::OriginalCpp => {
                        find_subtitles_with(frame_iter, &native_search_params, |event| {
                            sender
                                .blocking_send(event)
                                .map_err(|_| eyre::eyre!("subtitle OCR receiver closed"))
                        })
                    }
                    SubtitleDetector::RustRewrite => {
                        for event in SubtitleSearch::new(frame_iter, RustSearchParams::default()) {
                            let event = NativeSubtitleEvent {
                                start_timestamp: event.start_timestamp,
                                end_timestamp: event.end_timestamp,
                                ocr_image: event.sample_bgr,
                            };
                            if sender.blocking_send(event).is_err() {
                                return;
                            }
                        }
                        Ok(())
                    }
                };

                if let Err(error) = search_result {
                    let _ = btx1.blocking_send(Message::SearchError(error.to_string()));
                    return;
                }

                let _ = completion_tx.send(());
            });

            tokio::task::spawn(async move {
                let events = iced::futures::stream::unfold(receiver, |mut receiver| async move {
                    receiver.recv().await.map(|event| (event, receiver))
                });
                let jobs = events.map(move |event| {
                    let ocr = ocr.clone();
                    tokio::task::spawn_blocking(move || {
                        let preview = super::mat_to_image_handle(&event.ocr_image).ok()?;

                        let text = preview
                            .clone()
                            .apply(DynamicImage::ImageRgba8)
                            .apply(|img| ocr.read().recognize_text(&img))
                            .unwrap_or_default();

                        Some(Message::EventFound {
                            start_timestamp: event.start_timestamp,
                            end_timestamp: event.end_timestamp,
                            text,
                            preview,
                        })
                    })
                });
                let mut results = Box::pin(jobs.buffered(OCR_PARALELLISM));

                while let Some(result) = results.next().await {
                    match result {
                        Ok(Some(message)) => {
                            if btx2.send(message).await.is_err() {
                                return;
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            let _ = btx2.send(Message::SearchError(error.to_string())).await;
                            return;
                        }
                    }
                }

                if completion_rx.await.is_ok() {
                    let _ = btx2.send(Message::SearchDone).await;
                }
            });

            while let Some(msg) = brx.recv().await {
                if tx.send(msg).await.is_err() {
                    break;
                }
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detection(start: u64, end: u64, text: &str) -> Message {
        Message::EventFound {
            start_timestamp: Duration::from_secs(start),
            end_timestamp: Duration::from_secs(end),
            text: text.to_owned(),
            preview: RgbaImage::new(1, 1),
        }
    }

    #[test]
    fn post_ocr_processing_merges_adjacent_duplicate_text() {
        let mut model = Model::default();
        let config = Config::default();

        model.update(detection(0, 1, "same text"), &config);
        model.update(detection(2, 3, "same text"), &config);

        assert_eq!(model.results.len(), 1);
        assert_eq!(model.results[0].end_timestamp, Duration::from_secs(3));
    }

    #[test]
    fn post_ocr_processing_can_be_disabled() {
        let mut model = Model::default();
        let config = Config {
            post_ocr_processing: false,
            ..Config::default()
        };

        model.update(detection(0, 1, "same text"), &config);
        model.update(detection(2, 3, "same text"), &config);

        assert_eq!(model.results.len(), 2);
    }
}
