use eyre::Context;
use iced::alignment::Vertical;
use rfd::AsyncFileDialog;
use std::{path::PathBuf, time::Duration};

use crate::{
    app::{
        Composition, ReportLike, format_duration, selection_canvas, video_player_widget,
        video_player_widget::{LoadingMessage, Player, PlayerMessage},
    },
    extraction::srt_parser::Subtitles,
    fl, impl_report_residual,
};

use iced::{Length, Subscription, Task, Widget, alignment::Horizontal};
use vse_ui::{
    self as cosmic, Apply, icon_button,
    widget::{self, AutoWidget, IntoWidget, icon},
};

// Several different modes can be used to find chapters in a video
// method a.
//   monitoring a specific section of a video, mark the changes in text as chapter markers
//   useful for simple record-like simple videos
// method b.
//   does the regular subtitle search first/or a user provided srt file, however mark places where there's an unusually large pause(user-defined), typically indicating a rest between songs, and chooses the first subtitle that outputs
//   useful for concert recordings
// method b.1
//     adds a regex detection for possible chapter markers, common for dual-language subtitle tracks where you want to filter out the places where the singer talks with the audience
//     provides a default filter that ignores track markers without japenese
#[derive(Default)]
pub struct Model {
    video_player: video_player_widget::Model,
    current_subtitles: Option<Subtitles>,
}

#[derive(Debug, Clone)]
pub enum Message {
    VideoPlayer(video_player_widget::Message),
    Canvas(selection_canvas::Message),
    PickSrt,
    LoadSubtitlesFromSrt(Option<PathBuf>),
}

pub enum Event {
    Run(Task<Message>),
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
    pub fn update(&mut self, message: Message) -> Event {
        match message {
            Message::VideoPlayer(message) => match self.video_player.update(message, ()) {
                video_player_widget::Event::Run(task) => Event::Run(task.map(Message::VideoPlayer)),
                video_player_widget::Event::Error(error) => Event::Error(error),
                video_player_widget::Event::None => Event::None,
            },
            Message::Canvas(message) => match message {
                selection_canvas::Message::CanvasSize(_rectangle) => Event::None,
                selection_canvas::Message::ScreenshotRegion(_rectangle) => Event::None,
            },
            Message::PickSrt => Task::perform(
                async move {
                    let dialog = AsyncFileDialog::new().add_filter(fl!("video"), &["srt"]);
                    let file = dialog.pick_file().await;
                    file.map(|file| file.path().to_path_buf())
                },
                Message::LoadSubtitlesFromSrt,
            )
            .apply(Event::Run),
            Message::LoadSubtitlesFromSrt(path) => {
                let s = std::fs::read(path?)
                    .wrap_err("couldn't read from path")
                    .and_then(|x| x.apply(String::from_utf8).wrap_err("read srt isn't utf8"))?;
                self.current_subtitles =
                    Some(Subtitles::new(&s).wrap_err("couldn't parse subtitles from srt")?);
                Event::None
            }
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        self.video_player.subscription(()).map(Message::VideoPlayer)
    }

    pub fn view(&self) -> ChapterFinderView<'_> {
        AutoWidget::from_builder(ChapterFinderViewBuilder { model: self })
    }
}

const fn playback_map(message: PlayerMessage) -> Message {
    Message::VideoPlayer(video_player_widget::Message::Playback(message))
}

pub type ChapterFinderView<'a> = AutoWidget<ChapterFinderViewBuilder<'a>>;

#[derive(Clone, Copy)]
pub struct ChapterFinderViewBuilder<'a> {
    model: &'a Model,
}

impl<'a> ChapterFinderViewBuilder<'a> {
    fn canvas() -> impl Widget<Message> + 'a {
        widget::canvas(selection_canvas::SelectionProgram::default())
            .width(Length::Fill)
            .height(Length::Fill)
            .map(Message::Canvas)
    }

    fn load_video(self) -> impl Widget<Message> + 'a {
        let has_video = self.model.video_player.is_some();
        widget::button(widget::text(if has_video {
            fl!("change-video")
        } else {
            fl!("load-video")
        }))
        .on_press(Message::VideoPlayer(video_player_widget::Message::Loading(
            LoadingMessage::PickVideo,
        )))
        .apply(|button| {
            if has_video {
                button
            } else {
                button.style(cosmic::theme::button::suggested)
            }
        })
    }

    fn video_preview(player: &'a Player) -> impl Widget<Message> + 'a {
        let image = player.image(0.).map(playback_map);
        widget::stack![image, Self::canvas()]
    }

    fn video_controls(player: &'a Player) -> impl Widget<Message> + 'a {
        let seek_duration = Duration::from_secs(5);
        let backward = icon_button!("media-seek-backward-symbolic")
            .on_press(playback_map(PlayerMessage::SeekBackward(seek_duration)))
            .style(cosmic::theme::button::nav_toggle);
        let forward = icon_button!("media-seek-forward-symbolic")
            .on_press(playback_map(PlayerMessage::SeekForward(seek_duration)))
            .style(cosmic::theme::button::nav_toggle);
        let current_time = widget::text(format_duration(player.current_time()))
            .width(Length::Fill)
            .align_x(Horizontal::Right);

        widget::row![backward, forward, current_time]
            .spacing(cosmic::theme::spacing().space_s)
            .align_y(Vertical::Center)
    }
}

impl<'a> IntoWidget for ChapterFinderViewBuilder<'a> {
    type Message = Message;
    type Theme = iced::Theme;
    type Renderer = iced::Renderer;
    type Widget = impl Widget<Message> + 'a;

    fn into_widget(self) -> Self::Widget {
        let player = self.model.video_player.as_ref();
        let preview = player.map(Self::video_preview);
        let slider = player.map(|player| player.slider().map(playback_map));
        let controls = player.map(Self::video_controls);

        widget::column![
            preview,
            slider,
            widget::row![self.load_video(), controls]
                .spacing(cosmic::theme::spacing().space_s)
                .align_y(Vertical::Center),
        ]
        .spacing(cosmic::theme::spacing().space_s)
    }
}
