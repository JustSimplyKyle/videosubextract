use super::subtitle::{self, SubtitleTableConfig};
use crate::{config::Config, fl};
use cosmic::{
    Apply, Element,
    widget::{self, segmented_button::SingleSelectModel},
};
use iced::futures::SinkExt;
use iced::{Alignment, Length, Subscription, Task};
use rfd::AsyncFileDialog;
use std::{fmt::Write, path::PathBuf, sync::Arc, time::Duration};

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
}

impl ExportFormat {
    const ALL: [Self; 4] = [Self::Srt, Self::Vtt, Self::Ass, Self::Txt];

    const fn extension(self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::Vtt => "vtt",
            Self::Ass => "ass",
            Self::Txt => "txt",
        }
    }
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
    formats: SingleSelectModel,
    preview_modes: SingleSelectModel,
    filename: String,
    opencc_enabled: bool,
    opencc_mode: usize,
    preserve_line_breaks: bool,
    preview: subtitle::Model,
    conversion_generation: u64,
    transformation_inputs: Arc<[(subtitle::SubtitleId, String)]>,
}

impl Default for Model {
    fn default() -> Self {
        let mut formats = SingleSelectModel::default();
        let mut first = None;
        for format in ExportFormat::ALL {
            let id = formats
                .insert()
                .text(format.extension().to_uppercase())
                .data::<ExportFormat>(format)
                .id();
            first.get_or_insert(id);
        }
        let first = first.expect("there is at least one export format");
        formats.activate(first);

        let mut preview_modes = SingleSelectModel::default();
        let original = preview_modes
            .insert()
            .text(fl!("original"))
            .data::<PreviewMode>(PreviewMode::Original)
            .id();
        preview_modes
            .insert()
            .text(fl!("converted"))
            .data::<PreviewMode>(PreviewMode::Converted);
        preview_modes.activate(original);

        Self {
            formats,
            preview_modes,
            filename: "subtitles".into(),
            opencc_enabled: false,
            opencc_mode: 0,
            preserve_line_breaks: true,
            preview: subtitle::Model::default(),
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
}

impl Model {
    pub fn sync(
        &mut self,
        path: Option<&PathBuf>,
        results: &subtitle::SubtitleResults,
        changed: subtitle::ResultsChanged,
        config: &Config,
    ) {
        if let Some(stem) = path.and_then(|path| path.file_stem()) {
            self.filename = stem.to_string_lossy().into_owned();
        }
        let changed = self.preview.sync_read_only_results(results, changed);
        self.transformation_inputs = match changed {
            subtitle::ResultsChanged::Full => self.all_transformation_inputs(),
            subtitle::ResultsChanged::Targeted(_) | subtitle::ResultsChanged::Append(_) => {
                self.missing_transformation_inputs()
            }
        };
        let _ = self.update(Message::Request, config);
    }

    fn all_transformation_inputs(&self) -> Arc<[(subtitle::SubtitleId, String)]> {
        self.preview
            .results()
            .iter()
            .map(|result| (result.id(), result.original_text().to_owned()))
            .collect::<Vec<_>>()
            .into()
    }

    fn missing_transformation_inputs(&self) -> Arc<[(subtitle::SubtitleId, String)]> {
        self.preview
            .results()
            .iter()
            .filter(|result| result.needs_transformation())
            .map(|result| (result.id(), result.original_text().to_owned()))
            .collect::<Vec<_>>()
            .into()
    }

    pub fn refresh_language(&mut self) {
        let preview_modes = self.preview_modes.iter().collect::<Vec<_>>();
        if let Some(id) = preview_modes.first() {
            self.preview_modes.text_set(*id, fl!("original"));
        }
        if let Some(id) = preview_modes.get(1) {
            self.preview_modes.text_set(*id, fl!("converted"));
        }
    }

    fn selected_format(&self) -> ExportFormat {
        self.formats
            .active_data::<ExportFormat>()
            .copied()
            .unwrap_or(ExportFormat::Srt)
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

    fn export_text(&self, results: &subtitle::SubtitleResults) -> String {
        match self.selected_format() {
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
        }
    }

    pub fn update(&mut self, message: Message, config: &Config) -> Event {
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
                self.preview.clear_transformed_text();
                self.transformation_inputs = self.all_transformation_inputs();
                self.update(Message::Request, config)
            }
            Message::SelectOpenCcMode(mode) => {
                self.opencc_mode = mode.min(OPENCC_MODES.len() - 1);
                self.preview.clear_transformed_text();
                self.transformation_inputs = self.all_transformation_inputs();
                self.update(Message::Request, config)
            }
            Message::TogglePreserveLineBreaks(preserve) => {
                self.preserve_line_breaks = preserve;
                self.preview.clear_transformed_text();
                self.transformation_inputs = self.all_transformation_inputs();
                self.update(Message::Request, config)
            }
            Message::Subtitle(message) => match self.preview.update(message, config) {
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
                    self.preview.set_transformed_text(id, text);
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
                let contents = self.export_text(self.preview.results());
                let extension = self.selected_format().extension();
                let filename = format!("{}.{}", self.filename.trim(), extension);
                Event::Run(Task::perform(
                    async move {
                        let file = AsyncFileDialog::new()
                            .add_filter(fl!("subtitle-file"), &[extension])
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
        }
    }

    fn video_summary<'a>(
        subtitles: &'a subtitle::Model,
        video_path: Option<&'a PathBuf>,
    ) -> Element<'a, Message> {
        let filename = video_path.and_then(|path| path.file_name()).map_or_else(
            || fl!("no-video-loaded"),
            |name| name.to_string_lossy().into_owned(),
        );
        let details = fl!("subtitle-count", count = subtitles.results().len());
        let (status, state) = if subtitles.search_active {
            (fl!("processing"), fl!("subtitle-extraction-in-progress"))
        } else if subtitles.done {
            let state = if subtitles.results().is_empty() {
                fl!("no-subtitles-ready")
            } else {
                fl!("subtitles-ready-for-export")
            };
            (fl!("completed"), state)
        } else {
            (
                fl!("not-completed"),
                fl!("subtitle-extraction-not-completed"),
            )
        };

        widget::row![
            widget::column![
                widget::text::body(filename)
                    .font(cosmic::font::semibold())
                    .width(Length::Fill)
                    .wrapping(iced::widget::text::Wrapping::None)
                    .ellipsize(iced::widget::text::Ellipsize::End(
                        iced::advanced::text::EllipsizeHeightLimit::Lines(1)
                    )),
                widget::text::caption(details)
            ]
            .width(Length::Fill)
            .spacing(cosmic::theme::spacing().space_xxs),
            widget::column![
                widget::text(status)
                    .class(cosmic::theme::Text::Accent)
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
        .class(cosmic::theme::Container::Card)
        .width(Length::Fill)
        .into()
    }

    fn export_settings(&self, disabled: bool) -> Element<'_, Message> {
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
        .width(Length::Fixed(260.0));

        let format_section = widget::settings::section()
            .title(fl!("format"))
            .add(format_tabs);
        let file_section = widget::settings::section()
            .title(fl!("export-settings"))
            .add(widget::settings::item(fl!("filename"), filename));
        let conversion_section = widget::settings::section()
            .title(fl!("chinese-conversion"))
            .add(
                widget::settings::item::builder(fl!("apply-opencc-conversion"))
                    .description(fl!("conversion-export-note"))
                    .toggler(self.opencc_enabled, Message::ToggleOpenCc),
            )
            .add(widget::settings::item(fl!("conversion-mode"), conversion))
            .add(
                widget::settings::item::builder(fl!("preserve-line-breaks"))
                    .checkbox(self.preserve_line_breaks, Message::TogglePreserveLineBreaks),
            );

        widget::settings::view_column(vec![
            format_section.into(),
            file_section.into(),
            conversion_section.into(),
            widget::button::text(format!(
                "{} .{}",
                fl!("export"),
                self.selected_format().extension().to_uppercase()
            ))
            .class(cosmic::theme::Button::Suggested)
            .on_press_maybe((!disabled).then_some(Message::Export))
            .width(Length::Fill)
            .into(),
        ])
        .spacing(spacing.space_m)
        .height(Length::Fill)
        .width(Length::FillPortion(2))
        .into()
    }

    fn export_view<'a>(
        &'a self,
        subtitles: &'a subtitle::Model,
        search_active: bool,
    ) -> Element<'a, Message> {
        let show_converted = matches!(
            self.preview_modes.active_data::<PreviewMode>(),
            Some(PreviewMode::Converted)
        );
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
            self.preview
                .results_table(
                    SubtitleTableConfig {
                        show_id_instead_of_preview: true,
                        timestamp_above_text: true,
                        read_only: true,
                        show_transformed: show_converted && self.opencc_enabled,
                    },
                    false,
                )
                .map(Message::Subtitle),
        ]
        .spacing(cosmic::theme::spacing().space_s)
        .padding(cosmic::theme::spacing().space_m)
        .height(Length::Fill)
        .apply(widget::container)
        .class(cosmic::theme::Container::Card)
        .width(Length::FillPortion(3));

        widget::row![
            self.export_settings(search_active || self.preview.results().is_empty()),
            preview
        ]
        .spacing(cosmic::theme::spacing().space_s)
        .height(Length::Fill)
        .into()
    }

    pub fn view<'a>(
        &'a self,
        subtitles: &'a subtitle::Model,
        video_path: Option<&'a PathBuf>,
    ) -> Element<'a, Message> {
        let spacing = cosmic::theme::spacing();
        let content = self.export_view(subtitles, subtitles.search_active);

        widget::column![Self::video_summary(subtitles, video_path), content]
            .spacing(spacing.space_s)
            .height(Length::Fill)
            .into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
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

fn conversion_stream(
    source: &ConversionStream,
) -> impl futures::Stream<Item = Message> + Send + use<> {
    let source = source.clone();
    iced::stream::channel(32, async move |mut output| {
        let generation = source.generation;
        let (sender, mut receiver) = tokio::sync::mpsc::channel(32);
        let worker = tokio::task::spawn_blocking(move || {
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
                if sender.blocking_send((*id, text)).is_err() {
                    break;
                }
            }
        });

        while let Some((id, text)) = receiver.recv().await {
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
        if let Err(error) = worker.await {
            output
                .send(Message::TransformationError {
                    generation,
                    error: format!("OpenCC conversion task failed: {error}"),
                })
                .await
                .ok();
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extraction::Subtitle;
    use iced::futures::StreamExt;
    use image::RgbaImage;

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

    #[tokio::test]
    async fn conversion_stream_emits_each_transformed_subtitle() {
        let id = subtitle::SubtitleId(7);
        let source = ConversionStream {
            generation: 1,
            config: Some("s2t.json"),
            preserve_line_breaks: false,
            inputs: vec![(id, "汉字\n测试".to_owned())].into(),
        };

        let messages = conversion_stream(&source).collect::<Vec<_>>().await;
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
            None,
            source.results(),
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
        assert_eq!(model.preview.results()[0].text_for_display(true), "汉字");

        model.update(
            Message::Transformed {
                generation,
                id,
                text: Some("漢字".to_owned()),
            },
            &config,
        );
        assert_eq!(model.preview.results()[0].text_for_display(true), "漢字");

        model.update(Message::Request, &config);
        assert_eq!(model.conversion_generation, generation.wrapping_add(1));
        assert_eq!(model.preview.results()[0].text_for_display(true), "漢字");
    }

    #[test]
    fn append_sync_preserves_existing_transformed_text() {
        let mut source = source_with_text("first");
        let config = Config::default();
        let first_id = source.results()[0].id();
        let mut model = Model::default();
        model.sync(
            None,
            source.results(),
            subtitle::ResultsChanged::Full,
            &config,
        );
        model
            .preview
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
        model.sync(None, source.results(), changed, &config);

        assert_eq!(
            model.preview.results()[0].text_for_display(true),
            "transformed first"
        );
        assert_eq!(model.preview.results()[1].text_for_display(true), "second");
        assert_eq!(model.transformation_inputs.len(), 1);
        assert_eq!(model.transformation_inputs[0].0, source.results()[1].id());
    }
}
