use super::{
    Composition, format_duration,
    subtitle::{self, SubtitleTableConfig},
};
use crate::config::{ChapterParams, Config, ExtractionTarget};
use crate::extraction::chapter_writer::{ChapterLocations, ChapterWriter};
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
    pub results: &'a subtitle::SubtitleResults,
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
    inputs: Arc<[(subtitle::SubtitleId, String)]>,
}

impl std::hash::Hash for ConversionStream {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.generation.hash(state);
    }
}

pub struct Model {
    formats: SingleSelectModel<ExportFormat>,
    video_path: Option<PathBuf>,
    video_duration: Duration,
    target: ExtractionTarget,
    chapter_params: ChapterParams,
    preview_modes: SingleSelectModel<PreviewMode>,
    filename: String,
    opencc_enabled: bool,
    opencc_mode: usize,
    preserve_line_breaks: bool,
    export_preview: subtitle::Model,
    conversion_generation: u64,
    transformation_inputs: Arc<[(subtitle::SubtitleId, String)]>,
}

impl Default for Model {
    fn default() -> Self {
        let target = ExtractionTarget::default();
        let formats = ExportFormat::formats(target);

        let preview_modes = SingleSelectModelBuilder::new()
            .insert(fl!("original"), PreviewMode::Original)
            .insert(fl!("converted"), PreviewMode::Converted)
            .with_first_as_active()
            .build();

        Self {
            formats,
            video_path: None,
            video_duration: Duration::ZERO,
            target,
            chapter_params: ChapterParams::default(),
            preview_modes,
            filename: "subtitles".into(),
            opencc_enabled: false,
            opencc_mode: 0,
            preserve_line_breaks: true,
            export_preview: subtitle::Model::default(),
            conversion_generation: 0,
            transformation_inputs: Arc::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    SelectFormat(widget::segmented_button::Entity),
    SelectPreviewMode(widget::segmented_button::Entity),
    FilenameChanged(String),
    ToggleOpenCc(bool),
    SelectOpenCcMode(usize),
    TogglePreserveLineBreaks(bool),
    Export,
    EmbedChapters,
    EmbedFinished(Result<Option<PathBuf>, String>),
    Subtitle(subtitle::Message),
    Request,
    Transformed {
        generation: u64,
        id: subtitle::SubtitleId,
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
        changed: subtitle::ResultsChanged,
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
        self.chapter_params = chapter_params;
        if target != self.target {
            self.target = target;
            self.formats = ExportFormat::formats(target);
        }
        let changed = self
            .export_preview
            .sync_result_from_source(results, changed);
        self.transformation_inputs = match changed {
            subtitle::ResultsChanged::Full => self.all_transformation_inputs(),
            subtitle::ResultsChanged::Targeted(_) | subtitle::ResultsChanged::Append(_) => {
                self.missing_transformation_inputs()
            }
        };
        let _ = self.update(Message::Request, config);
    }

    fn all_transformation_inputs(&self) -> Arc<[(subtitle::SubtitleId, String)]> {
        self.export_preview
            .results()
            .iter()
            .map(|result| (result.id(), result.original_text().to_owned()))
            .collect::<Vec<_>>()
            .into()
    }

    fn missing_transformation_inputs(&self) -> Arc<[(subtitle::SubtitleId, String)]> {
        self.export_preview
            .results()
            .iter()
            .filter(|result| result.needs_transformation())
            .map(|result| (result.id(), result.original_text().to_owned()))
            .collect::<Vec<_>>()
            .into()
    }

    pub fn refresh_language(&mut self) {
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
            .export_preview
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

    fn export_text(&self, results: &subtitle::SubtitleResults) -> eyre::Result<String> {
        Ok(match self.selected_format() {
            ExportFormat::FfMetadata => self.chapters().into_ffmetadata()?,
            ExportFormat::Txt if self.target.is_chapters() => self
                .chapters()
                .0
                .iter()
                .map(|chapter| format!("{} {}", format_duration(chapter.start), chapter.name))
                .collect::<Vec<_>>()
                .join("\n"),
            ExportFormat::Srt => subtitle::to_srt(results),
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
                self.export_preview.clear_transformed_text();
                self.transformation_inputs = self.all_transformation_inputs();
                self.update(Message::Request, config)
            }
            Message::SelectOpenCcMode(mode) => {
                self.opencc_mode = mode.min(OPENCC_MODES.len() - 1);
                self.export_preview.clear_transformed_text();
                self.transformation_inputs = self.all_transformation_inputs();
                self.update(Message::Request, config)
            }
            Message::TogglePreserveLineBreaks(preserve) => {
                self.preserve_line_breaks = preserve;
                self.export_preview.clear_transformed_text();
                self.transformation_inputs = self.all_transformation_inputs();
                self.update(Message::Request, config)
            }
            Message::Subtitle(message) => match self.export_preview.update(message, config) {
                subtitle::Event::Run(task) => Event::Run(task.map(Message::Subtitle)),
                subtitle::Event::Error(error) => Event::Error(error),
                subtitle::Event::GoToPostProduction | subtitle::Event::None => {
                    Event::Run(Task::none())
                }
                subtitle::Event::SyncWithPostProduction(_) => Event::Run(Task::none()),
            },
            Message::Request => {
                self.conversion_generation = self.conversion_generation.wrapping_add(1);
                Event::Run(Task::none())
            }
            Message::Transformed {
                generation,
                id,
                text,
            } => {
                if generation == self.conversion_generation {
                    self.export_preview.set_transformed_text(id, text);
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
                let contents = self.export_text(self.export_preview.results())?;
                let extension = self.selected_format().extension();
                let filename = format!("{}.{}", self.filename.trim(), extension);
                let filter_name = if self.target.is_chapters() {
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
            Message::EmbedChapters => {
                let input = self.video_path.clone()?;
                let chapters = self.chapters();
                let extension = input
                    .extension()
                    .map_or_else(|| "mkv".into(), |ext| ext.to_string_lossy().into_owned());
                let filename = format!("{}.chapters.{extension}", self.filename.trim());
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
                            return Err(fl!("embed-chapters-same-file"));
                        }
                        smol::unblock(move || embed_chapters(&input, output, chapters))
                            .await
                            .map(Some)
                    },
                    Message::EmbedFinished,
                ))
            }
            Message::EmbedFinished(result) => match result {
                Ok(Some(path)) => {
                    Event::Toast(fl!("chapters-embedded", path = path.display().to_string()))
                }
                Ok(None) => Event::Run(Task::none()),
                Err(error) => Event::Error(eyre::eyre!("embedding chapters failed: {error}")),
            },
        }
    }

    fn video_summary<'a>(
        subtitles: &'a subtitle::Model,
        video_path: Option<&'a PathBuf>,
    ) -> impl Widget<Message> + 'a {
        let filename = video_path.and_then(|path| path.file_name()).map_or_else(
            || fl!("no-video-loaded"),
            |name| name.to_string_lossy().into_owned(),
        );
        let chapters = subtitles.target().is_chapters();
        let count = subtitles.results().len();
        let details = if chapters {
            fl!("chapter-count", count = count)
        } else {
            fl!("subtitle-count", count = count)
        };
        let (status, state) = match subtitles.state {
            subtitle::State::Empty => (
                fl!("not-completed"),
                fl!("subtitle-extraction-not-completed"),
            ),
            subtitle::State::SearchActive => {
                (fl!("processing"), fl!("subtitle-extraction-in-progress"))
            }
            subtitle::State::Finished => {
                let state = match (subtitles.results().is_empty(), chapters) {
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

    fn export_settings(&self, disabled: bool) -> impl Widget<Message> + '_ {
        let spacing = cosmic::theme::spacing();
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
        .on_press_maybe((!disabled).then_some(Message::Export))
        .width(Length::Shrink);
        let mut sections = vec![
            format_section.boxed(),
            file_section.boxed(),
            conversion_section.boxed(),
            export.boxed(),
        ];
        if self.target.is_chapters() {
            let embed = widget::button(widget::text(fl!("embed-chapters-into-video")))
                .on_press_maybe(
                    (!disabled && self.video_path.is_some()).then_some(Message::EmbedChapters),
                )
                .width(Length::Shrink);
            sections.push(
                widget::settings::section()
                    .title(fl!("embed-chapters"))
                    .add(widget::text::caption(fl!("embed-chapters-description")))
                    .add(embed)
                    .boxed(),
            );
        }

        widget::settings::view_column(sections)
            .spacing(spacing.space_m)
            .height(Length::Fill)
            .width(Length::FillPortion(2))
    }

    fn export_view<'a>(
        &'a self,
        subtitles: &'a subtitle::Model,
        search_active: bool,
    ) -> impl Widget<Message> + 'a {
        let show_converted = matches!(self.preview_modes.active_data(), PreviewMode::Converted);
        let preview_toggle = widget::segmented_control::horizontal(&self.preview_modes)
            .on_activate(Message::SelectPreviewMode);
        let preview = widget::column![
            widget::row![
                widget::text::title3(fl!("export-preview")),
                widget::Space::new().width(Length::Fill),
                preview_toggle
            ]
            .align_y(Alignment::Center),
            widget::text(fl!("showing-cues", count = subtitles.results().len())),
            self.export_preview
                .results_table(
                    SubtitleTableConfig {
                        show_id_instead_of_preview: true,
                        timestamp_above_text: true,
                        read_only: true,
                        show_transformed: show_converted && self.opencc_enabled,
                        start_only: self.target.is_chapters(),
                    },
                    false,
                )
                .map(Message::Subtitle),
        ]
        .spacing(cosmic::theme::spacing().space_s)
        .padding(cosmic::theme::spacing().space_m)
        .height(Length::Fill)
        .apply(widget::container)
        .style(cosmic::theme::container::card)
        .width(Length::FillPortion(3));

        widget::row![
            self.export_settings(search_active || self.export_preview.results().is_empty()),
            preview
        ]
        .spacing(cosmic::theme::spacing().space_s)
        .height(Length::Fill)
    }

    fn view<'a>(
        &'a self,
        subtitles: &'a subtitle::Model,
        video_path: Option<&'a PathBuf>,
    ) -> impl Widget<Message> + 'a {
        let spacing = cosmic::theme::spacing();
        let content = self.export_view(subtitles, subtitles.state.is_search_active());

        widget::column![Self::video_summary(subtitles, video_path), content]
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
    pub subtitles: &'a subtitle::Model,
    pub video_path: Option<&'a PathBuf>,
}

impl Composition for Model {
    type Message = Message;

    type Event = Event;

    type ViewContext<'a> = ViewArgs<'a>;

    type UpdateContext<'a> = &'a Config;

    type SubscriptionContext<'a> = ();

    fn view<'a>(&'a self, context: Self::ViewContext<'a>) -> impl Widget<Self::Message> + 'a {
        Self::view(self, context.subtitles, context.video_path)
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
            for (id, original) in source.inputs.iter() {
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
                if let Err(e) = sender.send_blocking((*id, text)) {
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

        while let Ok((id, text)) = receiver.recv().await {
            if output
                .send(Message::Transformed {
                    generation,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extraction::Subtitle;
    use iced::futures::StreamExt;
    use image::RgbaImage;
    use smol::block_on;

    fn source_with_text(text: &str) -> subtitle::Model {
        let mut source = subtitle::Model::default();
        source.update(
            subtitle::Message::EventFound {
                subtitle: Subtitle::new(Duration::ZERO, Duration::from_secs(1), text.to_owned()),
                replace_previous: false,
                preview: RgbaImage::new(1, 1),
            },
            &Config::default(),
        );
        source
    }

    fn subtitles_source(results: &subtitle::SubtitleResults) -> SyncSource<'_> {
        SyncSource {
            video_path: None,
            video_duration: Duration::ZERO,
            target: ExtractionTarget::Subtitles,
            chapter_params: ChapterParams::default(),
            results,
        }
    }

    #[test]
    fn chapter_results_export_as_chapters() {
        let config = Config::default();
        let mut source = subtitle::Model::default();
        for (start, end, text) in [(2, 170, "One"), (180, 290, "Two")] {
            let _ = source.update(
                subtitle::Message::EventFound {
                    subtitle: Subtitle::new(
                        Duration::from_secs(start),
                        Duration::from_secs(end),
                        text.to_owned(),
                    ),
                    replace_previous: false,
                    preview: RgbaImage::new(1, 1),
                },
                &config,
            );
        }
        let mut model = Model::default();
        model.sync(
            SyncSource {
                video_duration: Duration::from_secs(300),
                target: ExtractionTarget::Chapters,
                ..subtitles_source(source.results())
            },
            subtitle::ResultsChanged::Full,
            &config,
        );

        assert_eq!(model.selected_format(), ExportFormat::FfMetadata);
        assert_eq!(
            model.export_text(model.export_preview.results()).unwrap(),
            ";FFMETADATA1\n\
             [CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=180000\ntitle=One\n\
             [CHAPTER]\nTIMEBASE=1/1000\nSTART=180000\nEND=300000\ntitle=Two\n"
        );

        let txt = model
            .formats
            .entries()
            .iter()
            .find_map(|(id, entry)| (*entry.data() == ExportFormat::Txt).then_some(*id))
            .unwrap();
        model.update(Message::SelectFormat(txt), &config);
        assert_eq!(
            model.export_text(model.export_preview.results()).unwrap(),
            "00:00:00 One\n00:03:00 Two"
        );
    }

    #[test]
    fn conversion_stream_emits_each_transformed_subtitle() {
        let id = subtitle::SubtitleId(7);
        let source = ConversionStream {
            generation: 1,
            config: Some("s2t.json"),
            preserve_line_breaks: false,
            inputs: vec![(id, "汉字\n测试".to_owned())].into(),
        };

        let messages = conversion_stream(&source)
            .collect::<Vec<_>>()
            .apply(block_on);
        assert!(matches!(
            messages.as_slice(),
            [Message::Transformed {
                generation: 1,
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
            subtitle::ResultsChanged::Full,
            &config,
        );
        let generation = model.conversion_generation;

        model.update(
            Message::Transformed {
                generation: generation.wrapping_sub(1),
                id,
                text: Some("stale".to_owned()),
            },
            &config,
        );
        assert_eq!(
            model.export_preview.results()[0].text_for_display(true),
            "汉字"
        );

        model.update(
            Message::Transformed {
                generation,
                id,
                text: Some("漢字".to_owned()),
            },
            &config,
        );
        assert_eq!(
            model.export_preview.results()[0].text_for_display(true),
            "漢字"
        );

        model.update(Message::Request, &config);
        assert_eq!(model.conversion_generation, generation.wrapping_add(1));
        assert_eq!(
            model.export_preview.results()[0].text_for_display(true),
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
            subtitle::ResultsChanged::Full,
            &config,
        );
        model
            .export_preview
            .set_transformed_text(first_id, Some("transformed first".to_owned()));

        let event = source.update(
            subtitle::Message::EventFound {
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
        let subtitle::Event::SyncWithPostProduction(changed) = event else {
            panic!("append should request synchronization");
        };
        model.sync(subtitles_source(source.results()), changed, &config);

        assert_eq!(
            model.export_preview.results()[0].text_for_display(true),
            "transformed first"
        );
        assert_eq!(
            model.export_preview.results()[1].text_for_display(true),
            "second"
        );
        assert_eq!(model.transformation_inputs.len(), 1);
        assert_eq!(model.transformation_inputs[0].0, source.results()[1].id());
    }
}

/// Remux `input` into `output` with `chapters`, copying every stream as-is.
fn embed_chapters(
    input: &std::path::Path,
    output: PathBuf,
    chapters: ChapterLocations,
) -> Result<PathBuf, String> {
    let mut child = ChapterWriter::new(chapters)
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
