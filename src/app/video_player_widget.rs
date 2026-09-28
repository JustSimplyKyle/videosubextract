use super::*;
use crate::apply_traits::ApplyConditional;
use cosmic::Apply;
use iced::alignment::Horizontal;
use iced::futures::SinkExt;
use image::RgbaImage;
use rfd::AsyncFileDialog;
use std::{env::current_dir, path::PathBuf, sync::Arc, time::Duration};
use vse_ui as cosmic;

#[derive(Default)]
pub struct Model {
    video_path: Option<PathBuf>,
    video_controller: Option<VideoPlayerController>,
    video_allocation: Option<(widget::image::Allocation, iced::Size)>,
    current_time: Duration,
    // Keep the requested position visible while pre-seek frames drain.
    pending_seek: Option<Duration>,
    is_allocating_frame: bool,
}

#[derive(derive_more::Debug, Clone)]
pub enum Message {
    PickVideo,
    VideoFilePicked(Option<PathBuf>),
    LoadVideo(PathBuf),
    #[debug("{}x{}@{}", image.width(), image.height(), format_duration(*timestamp))]
    VideoFrame {
        image: RgbaImage,
        timestamp: Duration,
    },
    VideoFrameAllocated(Result<(widget::image::Allocation, iced::Size), String>),
    SeekForward(Duration),
    SeekBackward(Duration),
    SeekAbsolute(Duration),
    VideoError(String),
}

pub enum Event {
    Run(Task<Message>),
    Error(eyre::Report),
    None,
}

impl Model {
    pub fn update(&mut self, message: Message) -> Event {
        match message {
            Message::PickVideo => {
                let pwd = current_dir();
                Task::perform(
                    async move {
                        let dialog = AsyncFileDialog::new().add_filter(
                            fl!("video"),
                            &["mkv", "mp4", "avi", "mov", "webm", "flv", "wmv"],
                        );
                        let file = dialog
                            .apply_if_ok_ref(&pwd, AsyncFileDialog::set_directory)
                            .pick_file()
                            .await;
                        file.map(|file| file.path().to_path_buf())
                    },
                    Message::VideoFilePicked,
                )
                .apply(Event::Run)
            }
            Message::VideoFilePicked(Some(path)) => self.update(Message::LoadVideo(path)),
            Message::VideoFilePicked(None) => Event::None,
            Message::LoadVideo(path) => match ffmpeg_the_third::format::input(&path) {
                Ok(input) => match create_video_player::<false>(
                    input,
                    None,
                    crate::config::ProcessingResolution::None,
                ) {
                    Ok((controller, _)) => {
                        self.video_path = Some(path);
                        self.video_controller = Some(controller);
                        self.video_allocation = None;
                        self.current_time = Duration::ZERO;
                        self.pending_seek = None;
                        Event::None
                    }
                    Err(error) => Event::Error(error.wrap_err("initializing the video player")),
                },
                Err(error) => {
                    Event::Error(eyre::eyre!(error).wrap_err("opening the video with FFmpeg"))
                }
            },
            Message::VideoFrame {
                image: frame,
                timestamp,
            } => {
                if self.is_allocating_frame {
                    return Event::None;
                }

                if self
                    .pending_seek
                    .is_none_or(|target| seek_has_settled(target, timestamp))
                {
                    self.current_time = timestamp;
                    self.pending_seek = None;
                }
                self.is_allocating_frame = true;
                let size = iced::Size::new(frame.width() as f32, frame.height() as f32);
                let handle = widget::image::Handle::from_rgba(
                    frame.width(),
                    frame.height(),
                    frame.into_raw(),
                );
                widget::image::allocate(handle)
                    .map(move |result| {
                        Message::VideoFrameAllocated(
                            result.map_err(|error| error.to_string()).map(|x| (x, size)),
                        )
                    })
                    .apply(Event::Run)
            }
            Message::VideoFrameAllocated(allocation) => {
                self.is_allocating_frame = false;
                match allocation {
                    Ok(allocation) => {
                        self.video_allocation = Some(allocation);
                        Event::None
                    }
                    Err(error) => Event::Error(eyre::eyre!(
                        "failed to allocate video frame on GPU: {error}"
                    )),
                }
            }
            Message::SeekForward(duration) => {
                let target = self
                    .current_time()
                    .saturating_add(duration)
                    .min(self.video_duration());
                self.seek(target)
            }
            Message::SeekBackward(duration) => {
                self.seek(self.current_time().saturating_sub(duration))
            }
            Message::SeekAbsolute(duration) => self.seek(duration.min(self.video_duration())),
            Message::VideoError(message) => {
                self.video_controller = None;
                Event::Error(eyre::eyre!("video playback failed: {message}"))
            }
        }
    }

    fn seek(&mut self, target: Duration) -> Event {
        match self
            .video_controller
            .as_ref()
            .map(|controller| controller.seek_absolute(target))
        {
            Some(Err(error)) => {
                Event::Error(error.wrap_err(format!("seeking to {:.2}s", target.as_secs_f64())))
            }
            Some(Ok(())) => {
                self.pending_seek = Some(target);
                Event::None
            }
            None => Event::None,
        }
    }

    fn video_duration(&self) -> Duration {
        self.video_controller
            .as_ref()
            .map_or(Duration::ZERO, |controller| {
                controller.inner.info.video_time
            })
    }

    pub fn subscription(&self) -> Subscription<Message> {
        self.video_controller
            .as_ref()
            .map_or_else(Subscription::none, |controller| {
                iced::Subscription::run_with(controller.clone(), |controller| {
                    video_frame_stream(controller.inner.clone(), controller.inner.info.frame_rate)
                })
            })
    }

    pub fn view(&self) -> Element<'_, Message> {
        let image = widget::image(self.frame_handle())
            .content_fit(iced::ContentFit::Contain)
            .width(Length::Fill)
            .height(Length::Shrink);
        let load_video = widget::button(widget::text(if self.video_path.is_none() {
            fl!("load-video")
        } else {
            fl!("change-video")
        }))
        .on_press(Message::PickVideo);
        let load_video = if self.video_path.is_none() {
            load_video.style(cosmic::theme::button::suggested)
        } else {
            load_video
        };
        let backward = widget::button(icon::from_name("media-seek-backward-symbolic"))
            .on_press(Message::SeekBackward(Duration::from_secs(5)))
            .style(cosmic::theme::button::nav_toggle);
        let forward = widget::button(icon::from_name("media-seek-forward-symbolic"))
            .on_press(Message::SeekForward(Duration::from_secs(5)))
            .style(cosmic::theme::button::nav_toggle);
        let slider = self.video_controller.as_ref().map(|controller| {
            widget::slider(
                0.0..=controller.inner.info.video_time.as_secs_f64(),
                self.current_time().as_secs_f64(),
                |seconds| Message::SeekAbsolute(Duration::from_secs_f64(seconds)),
            )
        });
        let current_time = widget::text(format_duration(self.current_time()))
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

    pub fn path(&self) -> Option<&PathBuf> {
        self.video_path.as_ref()
    }

    pub fn controller(&self) -> Option<&VideoPlayerController> {
        self.video_controller.as_ref()
    }

    pub fn allocation(&self) -> Option<&(widget::image::Allocation, iced::Size)> {
        self.video_allocation.as_ref()
    }

    pub fn current_time(&self) -> Duration {
        self.pending_seek.unwrap_or(self.current_time)
    }

    pub fn frame_handle(&self) -> widget::image::Handle {
        self.video_allocation.as_ref().map_or_else(
            || widget::image::Handle::from_rgba(1, 1, vec![0, 0, 0, 255]),
            |(allocation, _)| allocation.handle().clone(),
        )
    }
}

const SEEK_SETTLE_TOLERANCE: Duration = Duration::from_millis(250);

fn seek_has_settled(target: Duration, timestamp: Duration) -> bool {
    target.abs_diff(timestamp) <= SEEK_SETTLE_TOLERANCE
}

fn video_frame_stream(
    inner: Arc<InnerPlayer>,
    frame_rate: f64,
) -> impl futures::Stream<Item = Message> + Send {
    let frame_duration = Duration::from_secs_f64(1.0 / frame_rate.max(1.0));

    iced::stream::channel(
        2,
        async move |mut output: futures::channel::mpsc::Sender<Message>| {
            let (sender, receiver) = async_channel::bounded::<Message>(2);
            smol::spawn(smol::unblock(move || {
                let mut frames = video_player::VideoPlayerIterator::<false> {
                    inner,
                    current_generation: 0,
                };
                loop {
                    let started = std::time::Instant::now();
                    match frames.next() {
                        Some(Ok(frame)) => match video_player::mat_to_rgba(&frame.mat) {
                            Ok(image) => {
                                if sender
                                    .send_blocking(Message::VideoFrame {
                                        image,
                                        timestamp: frame.timestamp,
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(error) => {
                                sender
                                    .send_blocking(Message::VideoError(error.to_string()))
                                    .ok();
                                break;
                            }
                        },
                        Some(Err(error)) => {
                            sender
                                .send_blocking(Message::VideoError(error.to_string()))
                                .ok();
                            break;
                        }
                        None => break,
                    }
                    if let Some(remaining) = frame_duration.checked_sub(started.elapsed()) {
                        std::thread::sleep(remaining);
                    }
                }
            }))
            .detach();

            while let Ok(message) = receiver.recv().await {
                if output.send(message).await.is_err() {
                    break;
                }
            }
        },
    )
}
