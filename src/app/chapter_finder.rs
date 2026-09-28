use crate::apply_traits::ApplyConditional;

use super::*;
use cosmic::iced::widget::Stack;
use iced::alignment::Horizontal;
use vse_ui::{self as cosmic, Apply};

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
    canvas_generation: u32,
}

#[derive(Debug, Clone)]
pub enum Message {
    VideoPlayer(video_player_widget::Message),
    Canvas(selection_canvas::Message),
}

pub enum Event {
    Run(Task<Message>),
    Error(eyre::Report),
    None,
}

impl Model {
    pub fn update(&mut self, message: Message) -> Event {
        match message {
            Message::VideoPlayer(message) => match self.video_player.update(message) {
                video_player_widget::Event::Run(task) => Event::Run(task.map(Message::VideoPlayer)),
                video_player_widget::Event::Error(error) => Event::Error(error),
                video_player_widget::Event::None => Event::None,
            },
            Message::Canvas(message) => match message {
                selection_canvas::Message::CanvasSize(rectangle) => Event::None,
                selection_canvas::Message::ScreenshotRegion(rectangle) => Event::None,
            },
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        self.video_player.subscription().map(Message::VideoPlayer)
    }

    fn canvas(&self) -> Element<'_, Message> {
        widget::canvas(selection_canvas::SelectionProgram {
            reset_generation: self.canvas_generation,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .apply(Element::from)
        .map(Message::Canvas)
    }

    fn video_preview(&self) -> Element<'_, Message> {
        use video_player_widget::Message as VideoMessage;

        let video = &self.video_player;

        let image = widget::image(video.frame_handle())
            .content_fit(iced::ContentFit::Contain)
            .width(Length::Fill)
            .height(Length::Shrink);

        let image = Stack::new().push(image).push(self.canvas());

        let load_video = widget::button(widget::text(if video.path().is_none() {
            fl!("load-video")
        } else {
            fl!("change-video")
        }))
        .on_press(VideoMessage::PickVideo.apply(Message::VideoPlayer));
        let load_video = if video.path().is_none() {
            load_video.style(cosmic::theme::button::suggested)
        } else {
            load_video
        };
        let backward = widget::button(icon::from_name("media-seek-backward-symbolic"))
            .on_press(
                VideoMessage::SeekBackward(Duration::from_secs(5)).apply(Message::VideoPlayer),
            )
            .style(cosmic::theme::button::nav_toggle);
        let forward = widget::button(icon::from_name("media-seek-forward-symbolic"))
            .on_press(VideoMessage::SeekForward(Duration::from_secs(5)).apply(Message::VideoPlayer))
            .style(cosmic::theme::button::nav_toggle);
        let slider = video.controller().as_ref().map(|controller| {
            widget::slider(
                0.0..=controller.inner.info.video_time.as_secs_f64(),
                video.current_time().as_secs_f64(),
                |seconds| {
                    VideoMessage::SeekAbsolute(Duration::from_secs_f64(seconds))
                        .apply(Message::VideoPlayer)
                },
            )
        });
        let current_time = widget::text(format_duration(video.current_time()))
            .width(Length::Fill)
            .align_x(Horizontal::Right);

        widget::column![
            image,
            slider,
            widget::row![load_video, backward, forward, current_time]
                .spacing(cosmic::theme::spacing().space_s)
                .align_y(Alignment::Center),
        ]
        .spacing(cosmic::theme::spacing().space_s)
        .into()
    }

    pub fn view(&self) -> Element<'_, Message> {
        self.video_preview()
    }
}
