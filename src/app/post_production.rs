use super::{
    Composition,
    detection::{self, DetectedTextTableConfig},
    format_duration,
};
use crate::config::{ChapterParams, Config, ExtractionTarget};
use crate::extraction::chapter_writer::{ChapterLocations, EmbedWriter};
use crate::extraction::chapters::{ChapterOptions, runs_to_chapters};
use crate::{app::ReportLike, fl, impl_report_residual};
use cosmic::{
    Apply,
    widget::{self, segmented_button::SingleSelectModel},
};
use iced::futures::SinkExt;
use iced::{Alignment, Length, Subscription, Task, Widget};
use rfd::AsyncFileDialog;
use std::{fmt::Write, path::PathBuf, sync::Arc, time::Duration};
use vse_ui::{
    self as cosmic,
    widget::{AnimatedDropdown, segmented_button::SingleSelectModelBuilder},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreviewMode {
    Original,
    Converted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExportFormat {
    Srt,
    Vtt,
    Ass,
    Txt,
    /// FFmpeg's metadata format, for `ffmpeg -i video -i chapters -map_chapters 1`.
    FfMetadata,
}

impl ExportFormat {
    const fn for_target(target: ExtractionTarget) -> &'static [Self] {
        match target {
            ExtractionTarget::Subtitles => &[Self::Srt, Self::Vtt, Self::Ass, Self::Txt],
            // TXT lists `HH:MM:SS title` lines, the format video sites read chapters from.
            ExtractionTarget::Chapters => &[Self::FfMetadata, Self::Txt],
        }
    }

    const fn extension(self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::Vtt => "vtt",
            Self::Ass => "ass",
            Self::Txt => "txt",
            Self::FfMetadata => "ffmetadata",
        }
    }

    fn label(self) -> String {
        match self {
            Self::FfMetadata => "FFMETADATA".into(),
            _ => self.extension().to_uppercase(),
        }
    }

    fn formats(target: ExtractionTarget) -> SingleSelectModel<Self> {
        let (first, rest) = Self::for_target(target)
            .split_first()
            .expect("every target has an export format");
        rest.iter()
            .fold(
                SingleSelectModelBuilder::new().insert(first.label(), *first),
                |model, format| model.insert(format.label(), *format),
            )
            .with_first_as_active()
            .build()
    }
}

/// Everything about the source a post-production snapshot is taken from.
pub(super) struct SyncSource<'a> {
    pub video_path: Option<&'a PathBuf>,
    pub video_duration: Duration,
    pub target: ExtractionTarget,
    pub chapter_params: ChapterParams,
    pub results: &'a detection::DetectedTexts,
}

const OPENCC_MODES: [(&str, &str); 6] = [
    ("s2t.json", "s2t — Simplified → Traditional"),
    ("t2s.json", "t2s — Traditional → Simplified"),
    ("s2tw.json", "s2tw — Simplified → Taiwan"),
    ("tw2s.json", "tw2s — Taiwan → Simplified"),
    ("s2hk.json", "s2hk — Simplified → Hong Kong"),
    ("hk2s.json", "hk2s — Hong Kong → Simplified"),
];

#[derive(Clone, Debug)]
struct ConversionStream {
    generation: u64,
    config: Option<&'static str>,
    preserve_line_breaks: bool,
    inputs: TransformationInputs,
}

/// DetectedText IDs are only unique within one track, so each input names its track.
type TransformationInputs = Arc<[(ExtractionTarget, detection::DetectedTextId, String)]>;

impl std::hash::Hash for ConversionStream {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.generation.hash(state);
    }
}

pub struct Model {
    /// Which track the preview and the format/export controls work on.
    tracks: SingleSelectModel<ExtractionTarget>,
    formats: SingleSelectModel<ExportFormat>,
    video_path: Option<PathBuf>,
    video_duration: Duration,
    chapter_params: ChapterParams,
    preview_modes: SingleSelectModel<PreviewMode>,
    filename: String,
    opencc_enabled: bool,
    opencc_mode: usize,
    preserve_line_breaks: bool,
    subtitles_preview: detection::Model,
    chapters_preview: detection::Model,
    conversion_generation: u64,
    transformation_inputs: TransformationInputs,
}

impl Default for Model {
    fn default() -> Self {
        let tracks = SingleSelectModelBuilder::from_array(
            ExtractionTarget::ALL.map(|target| (target_label(target), target)),
        )
        .with_first_as_active()
        .build();
        let formats = ExportFormat::formats(*tracks.active_data());

        let preview_modes = SingleSelectModelBuilder::new()
            .insert(fl!("original"), PreviewMode::Original)
            .insert(fl!("converted"), PreviewMode::Converted)
            .with_first_as_active()
            .build();

        Self {
            tracks,
            formats,
            video_path: None,
            video_duration: Duration::ZERO,
            chapter_params: ChapterParams::default(),
            preview_modes,
            filename: "subtitles".into(),
            opencc_enabled: false,
            opencc_mode: 0,
            preserve_line_breaks: true,
            subtitles_preview: detection::Model::new(ExtractionTarget::Subtitles),
            chapters_preview: detection::Model::new(ExtractionTarget::Chapters),
            conversion_generation: 0,
            transformation_inputs: Arc::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    SelectTrack(widget::segmented_button::Entity),
    SelectFormat(widget::segmented_button::Entity),
    SelectPreviewMode(widget::segmented_button::Entity),
    FilenameChanged(String),
    ToggleOpenCc(bool),
    SelectOpenCcMode(usize),
    TogglePreserveLineBreaks(bool),
    Export,
    EmbedIntoVideo,
    EmbedFinished(Result<Option<PathBuf>, String>),
    Preview(ExtractionTarget, detection::Message),
    Request,
    Transformed {
        generation: u64,
        target: ExtractionTarget,
        id: detection::DetectedTextId,
        text: Option<String>,
    },
    TransformationError {
        generation: u64,
        error: String,
    },
    ExportSaved(Result<Option<PathBuf>, String>),
}

pub enum Event {
    Run(Task<Message>),
    Toast(String),
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

impl Model {
    /// Synchronize the export snapshot after a source change or page entry.
    ///
    /// The application calls this with the latest subtitle results and the change
    /// returned by their mutation. `changed` is an invalidation hint, not a patch:
    /// use `Full` for initial entry or a complete rebuild. Partial changes retain
    /// unaffected transformed text; a full rebuild invalidates every cue.
    ///
    /// Snapshot application may promote a partial change to `Full`. Conversion
    /// inputs are selected from that applied scope, then `Message::Request`
    /// advances the generation used by the conversion subscription. This method
    /// does not itself run the conversion task; the subscription processes the
    /// inputs and `update` rejects replies from older generations.
    pub(super) fn sync(
        &mut self,
        source: SyncSource<'_>,
        changed: detection::ResultsChanged,
        config: &Config,
    ) {
        let SyncSource {
            video_path,
            video_duration,
            target,
            chapter_params,
            results,
        } = source;
        if let Some(stem) = video_path.and_then(|path| path.file_stem()) {
            self.filename = stem.to_string_lossy().into_owned();
        }
        self.video_path = video_path.cloned();
        self.video_duration = video_duration;
        if target.is_chapters() {
            self.chapter_params = chapter_params;
        }
        let changed = self
            .preview_mut(target)
            .sync_result_from_source(results, changed);
        // A new request restarts the conversion stream, so the other track's
        // pending cues must be requested again too.
        self.transformation_inputs = match changed {
            detection::ResultsChanged::Full => self.transformation_inputs_for(
                ExtractionTarget::ALL.map(|track| (track, track != target)),
            ),
            detection::ResultsChanged::Targeted(_) | detection::ResultsChanged::Append(_) => {
                self.missing_transformation_inputs()
            }
        };
        let _ = self.update(Message::Request, config);
    }

    const fn preview(&self, target: ExtractionTarget) -> &detection::Model {
        match target {
            ExtractionTarget::Subtitles => &self.subtitles_preview,
            ExtractionTarget::Chapters => &self.chapters_preview,
        }
    }

    const fn preview_mut(&mut self, target: ExtractionTarget) -> &mut detection::Model {
        match target {
            ExtractionTarget::Subtitles => &mut self.subtitles_preview,
            ExtractionTarget::Chapters => &mut self.chapters_preview,
        }
    }

    fn shown_track(&self) -> ExtractionTarget {
        *self.tracks.active_data()
    }

    /// DetectedTexts to convert from each listed track: all of them, or only the ones
    /// without converted text when the flag is set.
    fn transformation_inputs_for(
        &self,
        tracks: impl IntoIterator<Item = (ExtractionTarget, bool)>,
    ) -> TransformationInputs {
        tracks
            .into_iter()
            .flat_map(|(target, only_missing)| {
                self.preview(target)
                    .results()
                    .iter()
                    .filter(move |result| !only_missing || result.needs_transformation())
                    .map(move |result| (target, result.id(), result.original_text().to_owned()))
            })
            .collect::<Vec<_>>()
            .into()
    }

    fn all_transformation_inputs(&self) -> TransformationInputs {
        self.transformation_inputs_for(ExtractionTarget::ALL.map(|target| (target, false)))
    }

    fn missing_transformation_inputs(&self) -> TransformationInputs {
        self.transformation_inputs_for(ExtractionTarget::ALL.map(|target| (target, true)))
    }

    fn clear_transformed_text(&mut self) {
        self.subtitles_preview.clear_transformed_text();
        self.chapters_preview.clear_transformed_text();
    }

    pub fn refresh_language(&mut self) {
        let tracks = self.tracks.entries().keys().copied().collect::<Vec<_>>();
        for (id, target) in tracks.into_iter().zip(ExtractionTarget::ALL) {
            self.tracks.text_set(id, target_label(target));
        }

        let preview_modes = self
            .preview_modes
            .entries()
            .keys()
            .copied()
            .collect::<Vec<_>>();
        if let Some(id) = preview_modes.first() {
            self.preview_modes.text_set(*id, fl!("original"));
        }
        if let Some(id) = preview_modes.get(1) {
            self.preview_modes.text_set(*id, fl!("converted"));
        }
    }

    fn selected_format(&self) -> ExportFormat {
        *self.formats.active_data()
    }

    fn timestamp(duration: Duration, separator: char) -> String {
        let total_seconds = duration.as_secs();
        format!(
            "{:02}:{:02}:{:02}{separator}{:03}",
            total_seconds / 3_600,
            (total_seconds % 3_600) / 60,
            total_seconds % 60,
            duration.subsec_millis()
        )
    }

    /// Chapters as they will be exported, with any conversion applied to their titles.
    fn chapters(&self) -> ChapterLocations {
        let runs = self
            .chapters_preview
            .results()
            .iter()
            .map(|result| {
                let mut run = result.subtitle.clone();
                run.set_text(result.text_for_display(true).to_owned());
                run
            })
            .collect::<Vec<_>>();
        runs_to_chapters(
            &runs,
            self.video_duration,
            &ChapterOptions {
                min_length: self.chapter_params.min_chapter_length(),
                leading_title: fl!("intro-chapter"),
            },
        )
    }

    fn export_text(&self) -> eyre::Result<String> {
        let results = self.preview(self.shown_track()).results();
        Ok(match self.selected_format() {
            ExportFormat::FfMetadata => self.chapters().into_ffmetadata()?,
            ExportFormat::Txt if self.shown_track().is_chapters() => self
                .chapters()
                .0
                .iter()
                .map(|chapter| format!("{} {}", format_duration(chapter.start), chapter.name))
                .collect::<Vec<_>>()
                .join("\n"),
            ExportFormat::Srt => detection::to_srt(results),
            ExportFormat::Vtt => {
                let mut output = String::from("WEBVTT\n\n");
                for result in results.iter() {
                    writeln!(
                        output,
                        "{} --> {}\n{}\n",
                        Self::timestamp(result.subtitle.start_timestamp, '.'),
                        Self::timestamp(result.subtitle.end_timestamp, '.'),
                        result.text_for_display(true)
                    )
                    .ok();
                }
                output
            }
            ExportFormat::Ass => {
                let mut output = String::from(
                    "[Script Info]\nScriptType: v4.00+\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Default,Arial,24,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,0,2,10,10,10,1\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
                );
                for result in results.iter() {
                    let start = Self::timestamp(result.subtitle.start_timestamp, '.');
                    let end = Self::timestamp(result.subtitle.end_timestamp, '.');
                    writeln!(
                        output,
                        "Dialogue: 0,{}, {},Default,,0,0,0,,{}",
                        start.trim_start_matches('0'),
                        end.trim_start_matches('0'),
                        result.text_for_display(true).replace('\n', "\\N")
                    )
                    .ok();
                }
                output
            }
            ExportFormat::Txt => results
                .iter()
                .map(|result| result.text_for_display(true))
                .collect::<Vec<_>>()
                .join("\n\n"),
        })
    }

    fn update(&mut self, message: Message, config: &Config) -> Event {
        match message {
            Message::SelectTrack(id) => {
                self.tracks.activate(id);
                self.formats = ExportFormat::formats(self.shown_track());
                Event::Run(Task::none())
            }
            Message::SelectFormat(id) => {
                self.formats.activate(id);
                Event::Run(Task::none())
            }
            Message::SelectPreviewMode(id) => {
                self.preview_modes.activate(id);
                Event::Run(Task::none())
            }
            Message::FilenameChanged(filename) => {
                self.filename = filename;
                Event::Run(Task::none())
            }
            Message::ToggleOpenCc(enabled) => {
                self.opencc_enabled = enabled;
                self.clear_transformed_text();
                self.transformation_inputs = self.all_transformation_inputs();
                self.update(Message::Request, config)
            }
            Message::SelectOpenCcMode(mode) => {
                self.opencc_mode = mode.min(OPENCC_MODES.len() - 1);
                self.clear_transformed_text();
                self.transformation_inputs = self.all_transformation_inputs();
                self.update(Message::Request, config)
            }
            Message::TogglePreserveLineBreaks(preserve) => {
                self.preserve_line_breaks = preserve;
                self.clear_transformed_text();
                self.transformation_inputs = self.all_transformation_inputs();
                self.update(Message::Request, config)
            }
            Message::Preview(target, message) => {
                match self.preview_mut(target).update(message, config) {
                    detection::Event::Run(task) => {
                        Event::Run(task.map(move |message| Message::Preview(target, message)))
                    }
                    detection::Event::Error(error) => Event::Error(error),
                    detection::Event::GoToPostProduction | detection::Event::None => {
                        Event::Run(Task::none())
                    }
                    detection::Event::SyncWithPostProduction(_) => Event::Run(Task::none()),
                }
            }
            Message::Request => {
                self.conversion_generation = self.conversion_generation.wrapping_add(1);
                Event::Run(Task::none())
            }
            Message::Transformed {
                generation,
                target,
                id,
                text,
            } => {
                if generation == self.conversion_generation {
                    self.preview_mut(target).set_transformed_text(id, text);
                }
                Event::Run(Task::none())
            }
            Message::TransformationError { generation, error } => {
                if generation == self.conversion_generation {
                    Event::Error(eyre::eyre!(error))
                } else {
                    Event::Run(Task::none())
                }
            }
            Message::Export => {
                let contents = self.export_text()?;
                let extension = self.selected_format().extension();
                let filename = format!("{}.{}", self.filename.trim(), extension);
                let filter_name = if self.shown_track().is_chapters() {
                    fl!("chapter-file")
                } else {
                    fl!("subtitle-file")
                };
                Event::Run(Task::perform(
                    async move {
                        let file = AsyncFileDialog::new()
                            .add_filter(filter_name, &[extension])
                            .set_file_name(&filename)
                            .set_directory("./")
                            .save_file()
                            .await;

                        file.map(|file| {
                            std::fs::write(file.path(), contents)
                                .map(|()| file.path().to_path_buf())
                                .map_err(|error| error.to_string())
                        })
                        .transpose()
                    },
                    Message::ExportSaved,
                ))
            }
            Message::ExportSaved(result) => match result {
                Ok(Some(path)) => Event::Toast(fl!(
                    "successfully-saved-subtitles",
                    path = path.display().to_string()
                )),
                Ok(None) => Event::Run(Task::none()),
                Err(error) => {
                    Event::Error(eyre::eyre!("writing the subtitle file failed: {error}"))
                }
            },
            Message::EmbedIntoVideo => {
                let input = self.video_path.clone()?;
                let embed = EmbedWriter {
                    chapters: (!self.chapters_preview.results().is_empty())
                        .then(|| self.chapters()),
                    subtitles: (!self.subtitles_preview.results().is_empty())
                        .then(|| detection::to_srt(self.subtitles_preview.results())),
                };
                let extension = input
                    .extension()
                    .map_or_else(|| "mkv".into(), |ext| ext.to_string_lossy().into_owned());
                let filename = format!("{}.embedded.{extension}", self.filename.trim());
                Event::Run(Task::perform(
                    async move {
                        let mut dialog = AsyncFileDialog::new()
                            .add_filter(fl!("video"), &[extension.as_str()])
                            .set_file_name(&filename);
                        if let Some(directory) = input.parent() {
                            dialog = dialog.set_directory(directory);
                        }
                        let Some(file) = dialog.save_file().await else {
                            return Ok(None);
                        };
                        let output = file.path().to_path_buf();
                        // FFmpeg would truncate its own input before reading it.
                        if output == input {
                            return Err(fl!("embed-same-file"));
                        }
                        smol::unblock(move || embed_into_video(&input, output, embed))
                            .await
                            .map(Some)
                    },
                    Message::EmbedFinished,
                ))
            }
            Message::EmbedFinished(result) => match result {
                Ok(Some(path)) => Event::Toast(fl!(
                    "embedded-into-video",
                    path = path.display().to_string()
                )),
                Ok(None) => Event::Run(Task::none()),
                Err(error) => Event::Error(eyre::eyre!("embedding into the video failed: {error}")),
            },
        }
    }

    /// `shown` is the source model of the track being previewed; the status describes it.
    fn video_summary<'a>(
        sources: &ViewArgs<'a>,
        shown: &'a detection::Model,
    ) -> impl Widget<Message> + 'a {
        let filename = sources
            .video_path
            .and_then(|path| path.file_name())
            .map_or_else(
                || fl!("no-video-loaded"),
                |name| name.to_string_lossy().into_owned(),
            );
        let details = format!(
            "{} · {}",
            fl!("subtitle-count", count = sources.subtitles.results().len()),
            fl!("chapter-count", count = sources.chapters.results().len()),
        );
        let chapters = shown.target().is_chapters();
        let (status, state) = match shown.state {
            detection::State::Empty => (
                fl!("not-completed"),
                fl!("subtitle-extraction-not-completed"),
            ),
            detection::State::SearchActive => {
                (fl!("processing"), fl!("subtitle-extraction-in-progress"))
            }
            detection::State::Finished => {
                let state = match (shown.results().is_empty(), chapters) {
                    (true, false) => fl!("no-subtitles-ready"),
                    (true, true) => fl!("no-chapters-ready"),
                    (false, false) => fl!("subtitles-ready-for-export"),
                    (false, true) => fl!("chapters-ready-for-export"),
                };
                (fl!("completed"), state)
            }
        };

        widget::row![
            widget::column![
                widget::text::body(filename)
                    .font(cosmic::font::semibold())
                    .width(Length::Fill)
                    .wrapping(iced::widget::text::Wrapping::None)
                    .ellipsis(iced::widget::text::Ellipsis::End),
                widget::text::caption(details)
            ]
            .width(Length::Fill)
            .spacing(cosmic::theme::spacing().space_xxs),
            widget::column![
                widget::text(status)
                    .style(cosmic::theme::text::accent)
                    .align_x(iced::widget::text::Alignment::Right)
                    .width(Length::Fill),
                widget::text::caption(state)
                    .align_x(iced::widget::text::Alignment::Right)
                    .width(Length::Fill)
            ]
            .width(Length::Fixed(220.0))
            .spacing(cosmic::theme::spacing().space_xxs)
        ]
        .align_y(Alignment::Center)
        .padding(cosmic::theme::spacing().space_m)
        .apply(widget::container)
        .style(cosmic::theme::container::card)
        .width(Length::Fill)
    }

    /// `export_disabled` is about the shown track; embedding uses both tracks.
    fn export_settings(
        &self,
        export_disabled: bool,
        embed_disabled: bool,
    ) -> impl Widget<Message> + '_ {
        let spacing = cosmic::theme::spacing();
        let track_section = widget::settings::section().title(fl!("track")).add(
            widget::segmented_control::horizontal(&self.tracks)
                .fill()
                .on_activate(Message::SelectTrack),
        );
        let format_tabs =
            widget::segmented_control::horizontal(&self.formats).on_activate(Message::SelectFormat);
        let filename = widget::text_input("subtitles", &self.filename)
            .on_input(Message::FilenameChanged)
            .width(Length::Fixed(220.0));
        let conversion_labels = OPENCC_MODES
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>();
        let conversion = widget::dropdown(
            conversion_labels,
            Some(self.opencc_mode),
            Message::SelectOpenCcMode,
        )
        .width(Length::Fixed(260.0))
        .apply(AnimatedDropdown::from);

        let format_section = widget::settings::section()
            .title(fl!("format"))
            .add(format_tabs);
        let file_section = widget::settings::section()
            .title(fl!("export-settings"))
            .add(widget::settings::item(fl!("filename"), filename));
        let conversion_section = widget::settings::section()
            .title(fl!("chinese-conversion"))
            .add(
                widget::settings::togglable(fl!("apply-opencc-conversion"))
                    .description(fl!("conversion-export-note"))
                    .toggler(self.opencc_enabled, Message::ToggleOpenCc),
            )
            .add(widget::settings::item(fl!("conversion-mode"), conversion))
            .add(
                widget::settings::togglable(fl!("preserve-line-breaks"))
                    .toggler(self.preserve_line_breaks, Message::TogglePreserveLineBreaks),
            );

        let export = widget::button(widget::text(format!(
            "{} .{}",
            fl!("export"),
            self.selected_format().extension().to_uppercase()
        )))
        .style(cosmic::theme::button::suggested)
        .on_press_maybe((!export_disabled).then_some(Message::Export))
        .width(Length::Shrink);
        let embed = widget::button(widget::text(fl!("embed-into-video-action")))
            .on_press_maybe((!embed_disabled).then_some(Message::EmbedIntoVideo))
            .width(Length::Shrink);
        let embed_section = widget::settings::section()
            .title(fl!("embed-into-video"))
            .add(widget::text::caption(fl!(
                "embed-into-video-description",
                subtitles = self.subtitles_preview.results().len(),
                chapters = self.chapters_preview.results().len()
            )))
            .add(embed);
        let sections = vec![
            track_section.boxed(),
            format_section.boxed(),
            file_section.boxed(),
            conversion_section.boxed(),
            export.boxed(),
            embed_section.boxed(),
        ];

        widget::settings::view_column(sections)
            .spacing(spacing.space_m)
            .height(Length::Fill)
            .width(Length::FillPortion(2))
    }

    fn export_view<'a>(&'a self, sources: &ViewArgs<'a>) -> impl Widget<Message> + 'a {
        let shown = self.shown_track();
        let show_converted = matches!(self.preview_modes.active_data(), PreviewMode::Converted);
        let preview_toggle = widget::segmented_control::horizontal(&self.preview_modes)
            .on_activate(Message::SelectPreviewMode);
        let preview_results = self.preview(shown);
        let preview = widget::column![
            widget::row![
                widget::text::title3(fl!("export-preview")),
                widget::Space::new().width(Length::Fill),
                preview_toggle
            ]
            .align_y(Alignment::Center),
            widget::text(fl!("showing-cues", count = preview_results.results().len())),
            preview_results
                .results_table(
                    DetectedTextTableConfig {
                        show_id_instead_of_preview: true,
                        timestamp_above_text: true,
                        read_only: true,
                        show_transformed: show_converted && self.opencc_enabled,
                        start_only: shown.is_chapters(),
                    },
                    false,
                )
                .map(move |message| Message::Preview(shown, message)),
        ]
        .spacing(cosmic::theme::spacing().space_s)
        .padding(cosmic::theme::spacing().space_m)
        .height(Length::Fill)
        .apply(widget::container)
        .style(cosmic::theme::container::card)
        .width(Length::FillPortion(3));

        let searching =
            sources.subtitles.state.is_search_active() || sources.chapters.state.is_search_active();
        let source_for = |target| match target {
            ExtractionTarget::Subtitles => sources.subtitles,
            ExtractionTarget::Chapters => sources.chapters,
        };
        let export_disabled =
            source_for(shown).state.is_search_active() || preview_results.results().is_empty();
        let embed_disabled = searching
            || self.video_path.is_none()
            || (self.subtitles_preview.results().is_empty()
                && self.chapters_preview.results().is_empty());

        widget::row![
            self.export_settings(export_disabled, embed_disabled),
            preview
        ]
        .spacing(cosmic::theme::spacing().space_s)
        .height(Length::Fill)
    }

    fn view<'a>(&'a self, sources: ViewArgs<'a>) -> impl Widget<Message> + 'a {
        let spacing = cosmic::theme::spacing();
        let shown = match self.shown_track() {
            ExtractionTarget::Subtitles => sources.subtitles,
            ExtractionTarget::Chapters => sources.chapters,
        };
        let summary = Self::video_summary(&sources, shown);
        let content = self.export_view(&sources);

        widget::column![summary, content]
            .spacing(spacing.space_s)
            .height(Length::Fill)
    }

    fn subscription(&self) -> Subscription<Message> {
        if self.transformation_inputs.is_empty() {
            return Subscription::none();
        }
        Subscription::run_with(
            ConversionStream {
                generation: self.conversion_generation,
                config: self
                    .opencc_enabled
                    .then_some(OPENCC_MODES[self.opencc_mode].0),
                preserve_line_breaks: self.preserve_line_breaks,
                inputs: Arc::clone(&self.transformation_inputs),
            },
            conversion_stream,
        )
    }
}

pub(super) struct ViewArgs<'a> {
    pub subtitles: &'a detection::Model,
    pub chapters: &'a detection::Model,
    pub video_path: Option<&'a PathBuf>,
}

impl Composition for Model {
    type Message = Message;

    type Event = Event;

    type ViewContext<'a> = ViewArgs<'a>;

    type UpdateContext<'a> = &'a Config;

    type SubscriptionContext<'a> = ();

    fn view<'a>(&'a self, context: Self::ViewContext<'a>) -> impl Widget<Self::Message> + 'a {
        Self::view(self, context)
    }

    fn update(&mut self, message: Self::Message, context: Self::UpdateContext<'_>) -> Self::Event {
        Self::update(self, message, context)
    }

    fn subscription(&self, (): Self::SubscriptionContext<'_>) -> Subscription<Self::Message> {
        Self::subscription(self)
    }
}

fn conversion_stream(
    source: &ConversionStream,
) -> impl futures::Stream<Item = Message> + Send + use<> {
    let source = source.clone();
    iced::stream::channel(32, async move |mut output| {
        let generation = source.generation;
        let mut _output = output.clone();
        let (sender, receiver) = smol::channel::bounded(32);
        let worker = smol::unblock(move || {
            let converter = source.config.map(opencc::OpenCC::new);
            for (target, id, original) in source.inputs.iter() {
                let text = if converter.is_none() && source.preserve_line_breaks {
                    None
                } else {
                    let text = converter
                        .as_ref()
                        .map_or_else(|| original.clone(), |converter| converter.convert(original));
                    Some(if source.preserve_line_breaks {
                        text
                    } else {
                        text.lines().collect::<Vec<_>>().join(" ")
                    })
                };
                if let Err(e) = sender.send_blocking((*target, *id, text)) {
                    _output
                        .try_send(Message::TransformationError {
                            generation,
                            error: format!("failis to send error due to {e}"),
                        })
                        .ok();
                    break;
                }
            }
        });

        while let Ok((target, id, text)) = receiver.recv().await {
            if output
                .send(Message::Transformed {
                    generation,
                    target,
                    id,
                    text,
                })
                .await
                .is_err()
            {
                return;
            }
        }
        worker.cancel().await;
    })
}

fn target_label(target: ExtractionTarget) -> String {
    match target {
        ExtractionTarget::Subtitles => fl!("target-subtitles"),
        ExtractionTarget::Chapters => fl!("chapters"),
    }
}

/// Remux `input` into `output` with what `embed` adds, copying every existing stream.
fn embed_into_video(
    input: &std::path::Path,
    output: PathBuf,
    embed: EmbedWriter,
) -> Result<PathBuf, String> {
    let mut child = embed
        .into_sidecar(input, &output)
        .map_err(|error| error.to_string())?
        .spawn()
        .map_err(|error| fl!("ffmpeg-start-failed", error = error.to_string()))?;
    let errors = child
        .iter()
        .map_err(|error| fl!("ffmpeg-read-failed", error = error.to_string()))?
        .filter_errors()
        .collect::<Vec<_>>();
    let status = child
        .wait()
        .map_err(|error| fl!("ffmpeg-read-failed", error = error.to_string()))?;

    if status.success() {
        Ok(output)
    } else {
        Err(errors.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extraction::Subtitle;
    use iced::futures::StreamExt;
    use image::RgbaImage;
    use smol::block_on;

    fn source_with_text(text: &str) -> detection::Model {
        let mut source = detection::Model::default();
        source.update(
            detection::Message::EventFound {
                subtitle: Subtitle::new(Duration::ZERO, Duration::from_secs(1), text.to_owned()),
                replace_previous: false,
                preview: RgbaImage::new(1, 1),
            },
            &Config::default(),
        );
        source
    }

    fn subtitles_source(results: &detection::DetectedTexts) -> SyncSource<'_> {
        SyncSource {
            video_path: None,
            video_duration: Duration::ZERO,
            target: ExtractionTarget::Subtitles,
            chapter_params: ChapterParams::default(),
            results,
        }
    }

    fn source_with_runs(target: ExtractionTarget, runs: &[(u64, u64, &str)]) -> detection::Model {
        let mut source = detection::Model::new(target);
        for (start, end, text) in runs {
            let _ = source.update(
                detection::Message::EventFound {
                    subtitle: Subtitle::new(
                        Duration::from_secs(*start),
                        Duration::from_secs(*end),
                        (*text).to_owned(),
                    ),
                    replace_previous: false,
                    preview: RgbaImage::new(1, 1),
                },
                &Config::default(),
            );
        }
        source
    }

    fn select<T: PartialEq + 'static>(
        model: &SingleSelectModel<T>,
        value: &T,
    ) -> widget::segmented_button::Entity {
        model
            .entries()
            .iter()
            .find_map(|(id, entry)| (entry.data() == value).then_some(*id))
            .unwrap()
    }

    #[test]
    fn subtitle_and_chapter_tracks_are_kept_side_by_side() {
        let config = Config::default();
        let subtitles = source_with_runs(ExtractionTarget::Subtitles, &[(1, 2, "line")]);
        let chapters = source_with_runs(ExtractionTarget::Chapters, &[(0, 60, "Song")]);
        let mut model = Model::default();
        model.sync(
            subtitles_source(subtitles.results()),
            detection::ResultsChanged::Full,
            &config,
        );
        model.sync(
            SyncSource {
                target: ExtractionTarget::Chapters,
                ..subtitles_source(chapters.results())
            },
            detection::ResultsChanged::Full,
            &config,
        );

        assert_eq!(model.selected_format(), ExportFormat::Srt);
        assert_eq!(
            model.export_text().unwrap(),
            "1\n00:00:01,000 --> 00:00:02,000\nline\n\n"
        );

        let chapters_track = select(&model.tracks, &ExtractionTarget::Chapters);
        model.update(Message::SelectTrack(chapters_track), &config);
        assert_eq!(model.selected_format(), ExportFormat::FfMetadata);
        assert_eq!(model.subtitles_preview.results().len(), 1);
        assert_eq!(model.chapters_preview.results().len(), 1);
        // Both tracks are queued for conversion, each tagged with its own track.
        assert_eq!(
            model
                .transformation_inputs
                .iter()
                .map(|(target, _, text)| (*target, text.as_str()))
                .collect::<Vec<_>>(),
            [
                (ExtractionTarget::Subtitles, "line"),
                (ExtractionTarget::Chapters, "Song")
            ]
        );
    }

    #[test]
    fn chapter_results_export_as_chapters() {
        let config = Config::default();
        let source = source_with_runs(
            ExtractionTarget::Chapters,
            &[(2, 170, "One"), (180, 290, "Two")],
        );
        let mut model = Model::default();
        model.sync(
            SyncSource {
                video_duration: Duration::from_secs(300),
                target: ExtractionTarget::Chapters,
                ..subtitles_source(source.results())
            },
            detection::ResultsChanged::Full,
            &config,
        );
        let chapters_track = select(&model.tracks, &ExtractionTarget::Chapters);
        model.update(Message::SelectTrack(chapters_track), &config);

        assert_eq!(model.selected_format(), ExportFormat::FfMetadata);
        assert_eq!(
            model.export_text().unwrap(),
            ";FFMETADATA1\n\
             [CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=180000\ntitle=One\n\
             [CHAPTER]\nTIMEBASE=1/1000\nSTART=180000\nEND=300000\ntitle=Two\n"
        );

        let txt = select(&model.formats, &ExportFormat::Txt);
        model.update(Message::SelectFormat(txt), &config);
        assert_eq!(model.export_text().unwrap(), "00:00:00 One\n00:03:00 Two");
    }

    #[test]
    fn conversion_stream_emits_each_transformed_subtitle() {
        let id = detection::DetectedTextId(7);
        let source = ConversionStream {
            generation: 1,
            config: Some("s2t.json"),
            preserve_line_breaks: false,
            inputs: vec![(ExtractionTarget::Subtitles, id, "汉字\n测试".to_owned())].into(),
        };

        let messages = conversion_stream(&source)
            .collect::<Vec<_>>()
            .apply(block_on);
        assert!(matches!(
            messages.as_slice(),
            [Message::Transformed {
                generation: 1,
                target: ExtractionTarget::Subtitles,
                id: transformed_id,
                text: Some(text),
            }] if *transformed_id == id && text == "漢字 測試"
        ));
    }

    #[test]
    fn request_restarts_the_stream_and_stale_items_are_ignored() {
        let source = source_with_text("汉字");
        let mut model = Model::default();
        model.opencc_enabled = true;
        let config = Config::default();
        let id = source.results()[0].id();
        model.sync(
            subtitles_source(source.results()),
            detection::ResultsChanged::Full,
            &config,
        );
        let generation = model.conversion_generation;

        model.update(
            Message::Transformed {
                generation: generation.wrapping_sub(1),
                target: ExtractionTarget::Subtitles,
                id,
                text: Some("stale".to_owned()),
            },
            &config,
        );
        assert_eq!(
            model.subtitles_preview.results()[0].text_for_display(true),
            "汉字"
        );

        model.update(
            Message::Transformed {
                generation,
                target: ExtractionTarget::Subtitles,
                id,
                text: Some("漢字".to_owned()),
            },
            &config,
        );
        assert_eq!(
            model.subtitles_preview.results()[0].text_for_display(true),
            "漢字"
        );

        model.update(Message::Request, &config);
        assert_eq!(model.conversion_generation, generation.wrapping_add(1));
        assert_eq!(
            model.subtitles_preview.results()[0].text_for_display(true),
            "漢字"
        );
    }

    #[test]
    fn append_sync_preserves_existing_transformed_text() {
        let mut source = source_with_text("first");
        let config = Config::default();
        let first_id = source.results()[0].id();
        let mut model = Model::default();
        model.sync(
            subtitles_source(source.results()),
            detection::ResultsChanged::Full,
            &config,
        );
        model
            .subtitles_preview
            .set_transformed_text(first_id, Some("transformed first".to_owned()));

        let event = source.update(
            detection::Message::EventFound {
                subtitle: Subtitle::new(
                    Duration::from_secs(2),
                    Duration::from_secs(3),
                    "second".to_owned(),
                ),
                replace_previous: false,
                preview: RgbaImage::new(1, 1),
            },
            &config,
        );
        let detection::Event::SyncWithPostProduction(changed) = event else {
            panic!("append should request synchronization");
        };
        model.sync(subtitles_source(source.results()), changed, &config);

        assert_eq!(
            model.subtitles_preview.results()[0].text_for_display(true),
            "transformed first"
        );
        assert_eq!(
            model.subtitles_preview.results()[1].text_for_display(true),
            "second"
        );
        assert_eq!(model.transformation_inputs.len(), 1);
        assert_eq!(
            model.transformation_inputs[0].0,
            ExtractionTarget::Subtitles
        );
        assert_eq!(model.transformation_inputs[0].1, source.results()[1].id());
    }
}
