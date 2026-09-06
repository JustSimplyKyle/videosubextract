use super::subtitle::{self, SubtitleTableConfig};
use crate::{config::Config, fl};
use cosmic::{
    Apply, Element,
    widget::{self, segmented_button::SingleSelectModel},
};
use ffmpeg_sidecar::command::FfmpegCommand;
use iced::{Alignment, Length, Task};
use rfd::AsyncFileDialog;
use std::{fmt::Write, path::PathBuf, time::Duration};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tab {
    Export,
    Burn,
}

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

    fn extension(self) -> &'static str {
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

pub struct Model {
    pub tabs: SingleSelectModel,
    formats: SingleSelectModel,
    preview_modes: SingleSelectModel,
    filename: String,
    opencc_enabled: bool,
    opencc_mode: usize,
    preserve_line_breaks: bool,
    pub feedback: Option<String>,
}

impl Default for Model {
    fn default() -> Self {
        let mut tabs = SingleSelectModel::default();
        let export_tab = tabs
            .insert()
            .text(fl!("export-subtitles"))
            .data::<Tab>(Tab::Export)
            .id();
        tabs.insert()
            .text(fl!("burn-into-video"))
            .data::<Tab>(Tab::Burn);
        tabs.activate(export_tab);

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
            tabs,
            formats,
            preview_modes,
            filename: "subtitles".into(),
            opencc_enabled: false,
            opencc_mode: 0,
            preserve_line_breaks: true,
            feedback: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    SelectTab(widget::segmented_button::Entity),
    SelectFormat(widget::segmented_button::Entity),
    SelectPreviewMode(widget::segmented_button::Entity),
    FilenameChanged(String),
    ToggleOpenCc(bool),
    SelectOpenCcMode(usize),
    TogglePreserveLineBreaks(bool),
    Export,
    MergeWithVideo,
    Subtitle(subtitle::Message),
    MuxFinished(Result<String, String>),
    ExportSaved(Result<Option<PathBuf>, String>),
}

pub enum Event {
    Run(Task<Message>),
    Toast(String),
    Error(eyre::Report),
}

impl Model {
    pub fn set_video_path(&mut self, path: Option<&PathBuf>) {
        if let Some(stem) = path.and_then(|path| path.file_stem()) {
            self.filename = stem.to_string_lossy().into_owned();
        }
    }

    pub fn refresh_language(&mut self) {
        let tabs: Vec<_> = self.tabs.iter().collect();
        if let Some(id) = tabs.first() {
            self.tabs.text_set(*id, fl!("export-subtitles"));
        }
        if let Some(id) = tabs.get(1) {
            self.tabs.text_set(*id, fl!("burn-into-video"));
        }
        let preview_modes: Vec<_> = self.preview_modes.iter().collect();
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

    fn converted_results(
        &self,
        results: &[subtitle::SubtitleResult],
    ) -> Vec<subtitle::SubtitleResult> {
        let converter = self
            .opencc_enabled
            .then(|| opencc::OpenCC::new(OPENCC_MODES[self.opencc_mode].0));

        results
            .iter()
            .cloned()
            .map(|mut result| {
                let mut text = converter.as_ref().map_or_else(
                    || result.subtitle.text.clone(),
                    |cc| cc.convert(&result.subtitle.text),
                );
                if !self.preserve_line_breaks {
                    text = text.lines().collect::<Vec<_>>().join(" ");
                }
                result.set_text(text);
                result
            })
            .collect()
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

    fn export_text(&self, results: &[subtitle::SubtitleResult]) -> String {
        let results = self.converted_results(results);
        match self.selected_format() {
            ExportFormat::Srt => subtitle::to_srt(&results),
            ExportFormat::Vtt => {
                let mut output = String::from("WEBVTT\n\n");
                for result in &results {
                    writeln!(
                        output,
                        "{} --> {}\n{}\n",
                        Self::timestamp(result.subtitle.start_timestamp, '.'),
                        Self::timestamp(result.subtitle.end_timestamp, '.'),
                        result.subtitle.text
                    )
                    .ok();
                }
                output
            }
            ExportFormat::Ass => {
                let mut output = String::from(
                    "[Script Info]\nScriptType: v4.00+\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Default,Arial,24,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,0,2,10,10,10,1\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
                );
                for result in &results {
                    let start = Self::timestamp(result.subtitle.start_timestamp, '.');
                    let end = Self::timestamp(result.subtitle.end_timestamp, '.');
                    writeln!(
                        output,
                        "Dialogue: 0,{}, {},Default,,0,0,0,,{}",
                        start.trim_start_matches('0'),
                        end.trim_start_matches('0'),
                        result.subtitle.text.replace('\n', "\\N")
                    )
                    .ok();
                }
                output
            }
            ExportFormat::Txt => results
                .iter()
                .map(|result| result.subtitle.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n"),
        }
    }

    pub fn update(
        &mut self,
        message: Message,
        subtitles: &mut subtitle::Model,
        config: &Config,
        video_path: Option<&PathBuf>,
    ) -> Event {
        match message {
            Message::SelectTab(id) => {
                self.tabs.activate(id);
                self.feedback = None;
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
                Event::Run(Task::none())
            }
            Message::SelectOpenCcMode(mode) => {
                self.opencc_mode = mode.min(OPENCC_MODES.len() - 1);
                Event::Run(Task::none())
            }
            Message::TogglePreserveLineBreaks(preserve) => {
                self.preserve_line_breaks = preserve;
                Event::Run(Task::none())
            }
            Message::Subtitle(message) => match subtitles.update(message, config) {
                subtitle::Event::Run(task) => Event::Run(task.map(Message::Subtitle)),
                subtitle::Event::Error(error) => Event::Error(error),
                subtitle::Event::GoToPostProduction | subtitle::Event::None => {
                    Event::Run(Task::none())
                }
            },
            Message::Export => {
                self.feedback = None;
                let contents = self.export_text(&subtitles.results);
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
            Message::MergeWithVideo => {
                let srt = subtitle::to_srt(&self.converted_results(&subtitles.results));
                let temp_srt = std::env::temp_dir().join("videosubextract-temp-subs.srt");
                if let Err(error) = std::fs::write(&temp_srt, srt) {
                    self.feedback = Some(fl!("failed-create-temp-subtitle"));
                    return Event::Error(
                        eyre::eyre!(error).wrap_err("creating temporary subtitle file for merge"),
                    );
                }

                let Some(video) = video_path.cloned() else {
                    self.feedback = Some(fl!("no-video-loaded"));
                    std::fs::remove_file(temp_srt).ok();
                    return Event::Run(Task::none());
                };
                let stem = video.file_stem().unwrap_or_default().to_string_lossy();
                let output = video.with_file_name(format!("{stem}_merged.mkv"));

                Event::Run(Task::perform(
                    async move {
                        let mut command = FfmpegCommand::new();
                        let ffmpeg = command
                            .overwrite()
                            .input(video.to_string_lossy())
                            .format("srt")
                            .input(temp_srt.to_string_lossy())
                            .codec_audio("copy")
                            .codec_video("copy")
                            .codec_subtitle("srt")
                            .output(output.to_string_lossy());
                        let mut child = ffmpeg.spawn().map_err(|error| {
                            fl!("ffmpeg-start-failed", error = error.to_string())
                        })?;
                        let mut log = String::new();
                        let events = child.iter().map_err(|error| {
                            fl!("ffmpeg-read-failed", error = error.to_string())
                        })?;
                        for event in events {
                            writeln!(log, "{event:?}").ok();
                        }
                        std::fs::remove_file(temp_srt).ok();
                        Ok(log)
                    },
                    Message::MuxFinished,
                ))
            }
            Message::ExportSaved(result) => match result {
                Ok(Some(path)) => {
                    self.feedback = None;
                    Event::Toast(fl!(
                        "successfully-saved-subtitles",
                        path = path.display().to_string()
                    ))
                }
                Ok(None) => {
                    self.feedback = Some(fl!("file-save-cancelled"));
                    Event::Run(Task::none())
                }
                Err(error) => {
                    self.feedback = Some(fl!("failed-save-subtitles"));
                    Event::Error(eyre::eyre!("writing the subtitle file failed: {error}"))
                }
            },
            Message::MuxFinished(result) => match result {
                Ok(_) => {
                    self.feedback = None;
                    Event::Toast(fl!("subtitles-embedded"))
                }
                Err(error) => {
                    self.feedback = Some(error.clone());
                    Event::Error(eyre::eyre!(error))
                }
            },
        }
    }

    fn video_summary<'a>(
        &'a self,
        subtitles: &'a subtitle::Model,
        video_path: Option<&'a PathBuf>,
    ) -> Element<'a, Message> {
        let filename = video_path.and_then(|path| path.file_name()).map_or_else(
            || fl!("no-video-loaded"),
            |name| name.to_string_lossy().into_owned(),
        );
        let details = fl!("subtitle-count", count = subtitles.results.len());
        let (status, state) = if subtitles.search_active {
            (fl!("processing"), fl!("subtitle-extraction-in-progress"))
        } else if subtitles.done {
            let state = if subtitles.results.is_empty() {
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
            widget::text(fl!("showing-cues", count = subtitles.results.len())),
            subtitles
                .results_table(
                    SubtitleTableConfig {
                        show_id_instead_of_preview: true,
                        timestamp_above_text: true,
                        read_only: true,
                        opencc_config: (show_converted && self.opencc_enabled)
                            .then_some(OPENCC_MODES[self.opencc_mode].0),
                        preserve_line_breaks: !show_converted || self.preserve_line_breaks,
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
            self.export_settings(search_active || subtitles.results.is_empty()),
            preview
        ]
        .spacing(cosmic::theme::spacing().space_s)
        .height(Length::Fill)
        .into()
    }

    fn burn_view(&self, disabled: bool) -> Element<'_, Message> {
        widget::column![
            widget::text::title3(fl!("burn-into-video")),
            widget::text(fl!("burn-description")),
            widget::button::text(fl!("merge-subtitles-with-video"))
                .class(cosmic::theme::Button::Suggested)
                .on_press_maybe((!disabled).then_some(Message::MergeWithVideo)),
        ]
        .spacing(cosmic::theme::spacing().space_s)
        .padding(cosmic::theme::spacing().space_m)
        .apply(widget::container)
        .class(cosmic::theme::Container::Card)
        .width(Length::Fill)
        .into()
    }

    pub fn view<'a>(
        &'a self,
        subtitles: &'a subtitle::Model,
        video_path: Option<&'a PathBuf>,
    ) -> Element<'a, Message> {
        let spacing = cosmic::theme::spacing();
        let tabs = widget::tab_bar::horizontal(&self.tabs).on_activate(Message::SelectTab);
        let disabled = subtitles.search_active || subtitles.results.is_empty();
        let content = match self.tabs.active_data::<Tab>() {
            Some(Tab::Export) => self.export_view(subtitles, subtitles.search_active),
            Some(Tab::Burn) => self.burn_view(disabled),
            None => widget::text(fl!("select-tab")).into(),
        };

        widget::column![
            self.video_summary(subtitles, video_path),
            tabs,
            content,
            self.feedback.as_ref().map(|feedback| {
                widget::text(feedback)
                    .selectable()
                    .apply(widget::container)
                    .class(cosmic::theme::Container::Card)
                    .padding(spacing.space_s)
                    .width(Length::Fill)
            }),
        ]
        .spacing(spacing.space_s)
        .height(Length::Fill)
        .into()
    }
}
