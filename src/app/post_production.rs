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
    SrtSaved(Option<std::path::PathBuf>),
}

impl Model {
    pub fn update(
        &mut self,
        message: Message,
        results: &mut [super::subtitle::SubtitleResult],
        video_path: Option<&std::path::PathBuf>,
    ) -> Task<Message> {
        use std::fmt::Write;

        match message {
            Message::SelectTab(id) => {
                self.tabs.activate(id);
                self.feedback = None;
                Task::none()
            }
            Message::MergeWithVideo => {
                let srt = super::subtitle::to_srt(results);
                let temp_srt = std::env::temp_dir().join("temp_subs.srt");

                if std::fs::write(&temp_srt, srt).is_err() {
                    self.feedback =
                        Some("Failed to create temporary subtitle file for merge.".into());
                    std::fs::remove_file(temp_srt).ok();
                    return Task::none();
                }

                let Some(video) = video_path.cloned() else {
                    self.feedback = Some("No video loaded to merge with.".into());
                    std::fs::remove_file(temp_srt).ok();
                    return Task::none();
                };

                let stem = video.file_stem().unwrap_or_default().to_string_lossy();
                let output = video.with_file_name(format!("{stem}_merged.mkv"));

                Task::perform(
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
                            let _ = writeln!(log, "{event:?}");
                        }

                        Ok(log)
                    },
                    Message::MuxFinished,
                )
            }
            Message::OpenCcS2T => {
                let cc = opencc::OpenCC::new("s2t.json");
                for res in results.iter_mut() {
                    res.text = cc.convert(&res.text);
                }
                self.feedback = Some("Subtitles Converted to Traditional Chinese (S2T).".into());
                Task::none()
            }
            Message::OpenCcT2S => {
                let cc = opencc::OpenCC::new("t2s.json");
                for res in results.iter_mut() {
                    res.text = cc.convert(&res.text);
                }
                self.feedback = Some("Subtitles Converted to Simplified Chinese (T2S).".into());
                Task::none()
            }
            Message::ConvertToSrt => {
                self.feedback = None;
                let srt = super::subtitle::to_srt(results);
                Task::perform(
                    async move {
                        let file = AsyncFileDialog::new()
                            .add_filter("Subtitle", &["srt"])
                            .set_file_name("output.srt")
                            .set_directory("./")
                            .save_file()
                            .await;

                        file.map(|f| {
                            if let Err(e) = std::fs::write(f.path(), srt) {
                                eprintln!("error writing to srt file {e}");
                            }
                            f.path().to_path_buf()
                        })
                    },
                    Message::SrtSaved,
                )
            }
            Message::SrtSaved(path_opt) => {
                if let Some(p) = path_opt {
                    self.feedback = Some(format!("Successfully saved SRT to {}", p.display()));
                } else {
                    self.feedback = Some("File save cancelled.".into());
                }
                Task::none()
            }
            Message::MuxFinished(result) => {
                self.feedback = Some(match result {
                    Ok(log) if log.is_empty() => "Subtitles embedded successfully.".into(),
                    Ok(log) => log,
                    Err(error) => error,
                });

                Task::none()
            }
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
