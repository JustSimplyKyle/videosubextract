use cosmic::{
    Apply, Element,
    widget::{self, segmented_button::SingleSelectModel},
};
// use super::*;
use ffmpeg_sidecar::command::FfmpegCommand;
use iced::{Length, Task};
use rfd::AsyncFileDialog;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tab {
    Srt,
    Merge,
    OpenCc,
}

pub struct Model {
    pub tabs: SingleSelectModel,
    pub feedback: Option<String>,
}

impl Default for Model {
    fn default() -> Self {
        let mut tabs = SingleSelectModel::default();
        let srt_tab = tabs
            .insert()
            .text("Convert to SRT")
            .data::<Tab>(Tab::Srt)
            .id();
        tabs.insert().text("Merge Video").data::<Tab>(Tab::Merge);
        tabs.insert()
            .text("OpenCC Translate")
            .data::<Tab>(Tab::OpenCc);
        tabs.activate(srt_tab);

        Self {
            tabs,
            feedback: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    SelectTab(widget::segmented_button::Entity),
    MergeWithVideo,
    OpenCcS2T,
    OpenCcT2S,
    ConvertToSrt,
    MuxFinished(Result<String, String>),
    SrtSaved(Result<Option<std::path::PathBuf>, String>),
}

pub enum Event {
    Run(Task<Message>),
    Error(eyre::Report),
}

impl Model {
    pub fn update(
        &mut self,
        message: Message,
        results: &mut [super::subtitle::SubtitleResult],
        video_path: Option<&std::path::PathBuf>,
    ) -> Event {
        use std::fmt::Write;

        match message {
            Message::SelectTab(id) => {
                self.tabs.activate(id);
                self.feedback = None;
                Event::Run(Task::none())
            }
            Message::MergeWithVideo => {
                let srt = super::subtitle::to_srt(results);
                let temp_srt = std::env::temp_dir().join("temp_subs.srt");

                if let Err(error) = std::fs::write(&temp_srt, srt) {
                    self.feedback =
                        Some("Failed to create temporary subtitle file for merge.".into());
                    std::fs::remove_file(&temp_srt).ok();
                    return Event::Error(
                        eyre::eyre!(error).wrap_err("creating temporary subtitle file for merge"),
                    );
                }

                let Some(video) = video_path.cloned() else {
                    self.feedback = Some("No video loaded to merge with.".into());
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

                        let mut child = ffmpeg
                            .spawn()
                            .map_err(|error| format!("Failed to start FFmpeg: {error}"))?;

                        let mut log = String::new();
                        let events = child
                            .iter()
                            .map_err(|error| format!("Failed to read FFmpeg output: {error}"))?;

                        for event in events {
                            writeln!(log, "{event:?}").ok();
                        }

                        Ok(log)
                    },
                    Message::MuxFinished,
                ))
            }
            Message::OpenCcS2T => {
                let cc = opencc::OpenCC::new("s2t.json");
                for res in results.iter_mut() {
                    res.set_text(cc.convert(&res.text));
                }
                self.feedback = Some("Subtitles Converted to Traditional Chinese (S2T).".into());
                Event::Run(Task::none())
            }
            Message::OpenCcT2S => {
                let cc = opencc::OpenCC::new("t2s.json");
                for res in results.iter_mut() {
                    res.set_text(cc.convert(&res.text));
                }
                self.feedback = Some("Subtitles Converted to Simplified Chinese (T2S).".into());
                Event::Run(Task::none())
            }
            Message::ConvertToSrt => {
                self.feedback = None;
                let srt = super::subtitle::to_srt(results);
                Event::Run(Task::perform(
                    async move {
                        let file = AsyncFileDialog::new()
                            .add_filter("Subtitle", &["srt"])
                            .set_file_name("output.srt")
                            .set_directory("./")
                            .save_file()
                            .await;

                        file.map(|f| {
                            std::fs::write(f.path(), srt)
                                .map(|()| f.path().to_path_buf())
                                .map_err(|error| error.to_string())
                        })
                        .transpose()
                    },
                    Message::SrtSaved,
                ))
            }
            Message::SrtSaved(result) => match result {
                Ok(Some(path)) => {
                    self.feedback = Some(format!("Successfully saved SRT to {}", path.display()));
                    Event::Run(Task::none())
                }
                Ok(None) => {
                    self.feedback = Some("File save cancelled.".into());
                    Event::Run(Task::none())
                }
                Err(error) => {
                    self.feedback = Some("Failed to save the SRT file.".into());
                    Event::Error(eyre::eyre!("writing the SRT file failed: {error}"))
                }
            },
            Message::MuxFinished(result) => match result {
                Ok(log) if log.is_empty() => {
                    self.feedback = Some("Subtitles embedded successfully.".into());
                    Event::Run(Task::none())
                }
                Ok(log) => {
                    self.feedback = Some(log);
                    Event::Run(Task::none())
                }
                Err(error) => {
                    self.feedback = Some(error.clone());
                    Event::Error(eyre::eyre!(error))
                }
            },
        }
    }

    pub fn view(&self, search_active: bool) -> Element<'_, Message> {
        let space_s = cosmic::theme::spacing().space_s;

        let tabs = widget::tab_bar::horizontal(&self.tabs).on_activate(Message::SelectTab);

        let tab_content: Element<_> = match self.tabs.active_data::<Tab>() {
            Some(Tab::Srt) => {
                let btn = widget::button::text("Save as SRT")
                    .class(cosmic::theme::Button::Suggested)
                    .on_press_maybe((!search_active).then_some(Message::ConvertToSrt));
                widget::column![btn].into()
            }
            Some(Tab::Merge) => {
                let btn = widget::button::text("Merge Subtitles with Video")
                    .class(cosmic::theme::Button::Suggested)
                    .on_press_maybe((!search_active).then_some(Message::MergeWithVideo));
                widget::column![btn].into()
            }
            Some(Tab::OpenCc) => {
                let btn_s2t = widget::button::text("Simplified to Traditional")
                    .class(cosmic::theme::Button::Suggested)
                    .on_press_maybe((!search_active).then_some(Message::OpenCcS2T));
                let btn_t2s = widget::button::text("Traditional to Simplified")
                    .class(cosmic::theme::Button::Suggested)
                    .on_press_maybe((!search_active).then_some(Message::OpenCcT2S));
                widget::row![btn_s2t, btn_t2s].spacing(space_s).into()
            }
            None => widget::text("Select a tab").into(),
        };

        let mut col = widget::column![tabs, tab_content].spacing(space_s);

        if let Some(feedback) = &self.feedback {
            let feedback = widget::text(feedback)
                .selectable()
                .apply(widget::container)
                .width(Length::Fill)
                .height(Length::Shrink)
                .class(cosmic::theme::Container::Card)
                .padding(40)
                .apply(widget::scrollable);
            col = col.push(feedback);
        }

        col.into()
    }
}
