use super::*;
use crate::{app::ReportLike, impl_report_residual};
use cosmic::Apply;
use eyre::Context;
use iced::advanced::graphics::core::length::Constraint;
use iced::alignment::Horizontal;
use iced::futures::SinkExt;
use image::RgbaImage;
use rfd::AsyncFileDialog;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use vse_ui as cosmic;

/// A missing player means no video has been loaded.
pub type Model = Option<Player>;

/// Constructed only after opening the video and creating its controller succeed.
pub struct Player {
    video_path: PathBuf,
    video_controller: VideoPlayerController,
    video_allocation: Option<(widget::image::Allocation, iced::Size)>,
    current_time: Duration,
    // Keep the requested position visible while pre-seek frames drain.
    pending_seek: Option<Duration>,
    paused: Arc<AtomicBool>,
    is_allocating_frame: bool,
}

#[derive(derive_more::Debug, Clone)]
pub enum Message {
    Loading(LoadingMessage),
    Playback(PlayerMessage),
}

#[derive(Debug, Clone)]
pub enum LoadingMessage {
    PickVideo,
    VideoFilePicked(Option<PathBuf>),
    LoadVideo(PathBuf),
}

#[derive(derive_more::Debug, Clone)]
pub enum PlayerMessage {
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
    PauseToggle,
}

pub enum Event<M = Message> {
    Run(Task<M>),
    Error(eyre::Report),
    None,
}

impl<M> ReportLike for Event<M> {
    fn err(e: eyre::Report) -> Self {
        Self::Error(e)
    }
    fn none() -> Self {
        Self::None
    }
}

impl_report_residual!(Event);
impl_report_residual!(Event<PlayerMessage>);

impl<M: Send + 'static> Event<M> {
    fn map<N: Send + 'static>(self, f: impl Fn(M) -> N + Send + Sync + 'static) -> Event<N> {
        match self {
            Self::Run(task) => Event::Run(task.map(f)),
            Self::Error(error) => Event::Error(error),
            Self::None => Event::None,
        }
    }
}

impl Player {
    pub fn load(path: PathBuf) -> eyre::Result<Self> {
        let input =
            ffmpeg_the_third::format::input(&path).wrap_err("opening the video with ffmpeg")?;
        let (controller, _) =
            create_video_player::<false>(input, None, crate::config::ProcessingResolution::None)?;
        Ok(Self {
            video_path: path,
            video_controller: controller,
            video_allocation: None,
            current_time: Duration::ZERO,
            pending_seek: None,
            paused: Arc::new(false.into()),
            is_allocating_frame: false,
        })
    }
}

fn update(model: &mut Model, message: Message) -> Event {
    match message {
        Message::Loading(LoadingMessage::PickVideo) => Task::perform(
            async move {
                let dialog = AsyncFileDialog::new().add_filter(
                    fl!("video"),
                    &["mkv", "mp4", "avi", "mov", "webm", "flv", "wmv"],
                );
                dialog
                    .pick_file()
                    .await
                    .map(|file| file.path().to_path_buf())
            },
            |path| Message::Loading(LoadingMessage::VideoFilePicked(path)),
        )
        .apply(Event::Run),
        Message::Loading(
            LoadingMessage::VideoFilePicked(Some(path)) | LoadingMessage::LoadVideo(path),
        ) => {
            // Build the replacement first so a failed load preserves the current video.
            *model = Some(Player::load(path)?);
            Event::None
        }
        Message::Loading(LoadingMessage::VideoFilePicked(None)) => Event::None,
        Message::Playback(message) => {
            let Some(player) = model.as_mut() else {
                return Event::None;
            };
            if let PlayerMessage::VideoError(error) = message {
                *model = None;
                return Event::Error(eyre::eyre!("video playback failed: {error}"));
            }
            player.update(message).map(Message::Playback)
        }
    }
}

fn subscription(model: &Model) -> Subscription<Message> {
    model.as_ref().map_or_else(Subscription::none, |player| {
        player.subscription().map(Message::Playback)
    })
}

fn view(model: &Model) -> Element<'_, Message> {
    let load = widget::button(widget::text(if model.is_some() {
        fl!("change-video")
    } else {
        fl!("load-video")
    }))
    .on_press(Message::Loading(LoadingMessage::PickVideo));
    match model {
        Some(player) => widget::column![player.view().map(Message::Playback), load]
            .spacing(cosmic::theme::spacing().space_s)
            .into(),
        None => load.style(cosmic::theme::button::suggested).into(),
    }
}

impl Player {
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    fn update(&mut self, message: PlayerMessage) -> Event<PlayerMessage> {
        match message {
            PlayerMessage::VideoFrame {
                image: frame,
                timestamp,
            } => {
                if self.is_allocating_frame {
                    println!("can't keep up!");
                    WARNING_CHANNEL.0.try_send("can't keep up".to_string()).ok();
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
                        PlayerMessage::VideoFrameAllocated(
                            result.map_err(|error| error.to_string()).map(|x| (x, size)),
                        )
                    })
                    .apply(Event::Run)
            }
            PlayerMessage::VideoFrameAllocated(allocation) => {
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
            PlayerMessage::SeekForward(duration) => {
                let target = self
                    .current_time()
                    .saturating_add(duration)
                    .min(self.video_duration());
                self.seek(target)
            }
            PlayerMessage::SeekBackward(duration) => {
                self.seek(self.current_time().saturating_sub(duration))
            }
            PlayerMessage::SeekAbsolute(duration) => self.seek(duration.min(self.video_duration())),
            PlayerMessage::VideoError(error) => {
                Event::Error(eyre::eyre!("video playback failed: {error}"))
            }
            PlayerMessage::PauseToggle => {
                self.paused
                    .update(Ordering::Relaxed, Ordering::Relaxed, |x| !x);
                Event::None
            }
        }
    }

    pub fn seek(&mut self, target: Duration) -> Event<PlayerMessage> {
        self.video_controller
            .seek_absolute(target)
            .wrap_err_with(|| format!("seeking to {:.2}s", target.as_secs_f64()))?;
        self.pending_seek = Some(target);
        Event::None
    }

    pub fn video_duration(&self) -> Duration {
        self.video_controller.inner.info.video_time
    }

    fn subscription(&self) -> Subscription<PlayerMessage> {
        iced::Subscription::run_with(
            VideoFrameStreamData {
                controller: self.controller().clone(),
                paused: self.paused.clone(),
            },
            |data| video_frame_stream(data.clone()),
        )
    }

    fn view(&self) -> Element<'_, PlayerMessage> {
        let image = self.image(0.);
        let backward = widget::button(icon::from_name("media-seek-backward-symbolic"))
            .on_press(PlayerMessage::SeekBackward(Duration::from_secs(5)))
            .style(cosmic::theme::button::nav_toggle);
        let forward = widget::button(icon::from_name("media-seek-forward-symbolic"))
            .on_press(PlayerMessage::SeekForward(Duration::from_secs(5)))
            .style(cosmic::theme::button::nav_toggle);
        let slider = self.slider();
        let current_time = widget::text(format_duration(self.current_time()))
            .width(Length::Fill)
            .align_x(Horizontal::Right);

        widget::column![
            image,
            slider,
            widget::row![backward, forward, current_time]
                .spacing(cosmic::theme::spacing().space_s)
                .align_y(Alignment::Center),
        ]
        .spacing(cosmic::theme::spacing().space_s)
        .into()
    }

    pub fn slider(&self) -> Element<'_, PlayerMessage> {
        widget::slider(
            0.0..=self.video_duration().as_secs_f64(),
            self.current_time().as_secs_f64(),
            |seconds| PlayerMessage::SeekAbsolute(Duration::from_secs_f64(seconds)),
        )
        .into()
    }

    pub const fn path(&self) -> &PathBuf {
        &self.video_path
    }

    pub const fn controller(&self) -> &VideoPlayerController {
        &self.video_controller
    }

    pub const fn allocation(&self) -> Option<&(widget::image::Allocation, iced::Size)> {
        self.video_allocation.as_ref()
    }

    pub fn current_time(&self) -> Duration {
        self.pending_seek.unwrap_or(self.current_time)
    }

    pub fn frame_handle(&self) -> Option<widget::image::Handle> {
        self.video_allocation
            .as_ref()
            .map(|x| x.0.handle())
            .cloned()
    }

    pub fn image(&self, radius: f32) -> Element<'_, PlayerMessage> {
        let image: Element<'_, PlayerMessage> = self
            .frame_handle()
            .map(widget::image)
            .map(|x| {
                x.content_fit(iced::ContentFit::Contain)
                    .expand(true)
                    .width(Length::Shrink)
                    .height(Length::Shrink)
                    .border_radius(radius)
            })
            .map_or_else(|| widget::space().into(), Element::from);

        // Reserve the controls' intrinsic height first, then fit the frame into
        // the remaining space. Expand + Shrink keeps both image bounds tight
        // to its aspect ratio. Fluid(Max) releases unused height so the controls
        // sit directly below the image instead of at the bottom of the page.
        widget::container(image)
            .center_x(Length::Fill)
            .height(Length::Fluid(Constraint::Max))
            .into()
    }
}

const SEEK_SETTLE_TOLERANCE: Duration = Duration::from_millis(250);

fn seek_has_settled(target: Duration, timestamp: Duration) -> bool {
    target.abs_diff(timestamp) <= SEEK_SETTLE_TOLERANCE
}

#[derive(Clone)]
struct VideoFrameStreamData {
    controller: VideoPlayerController,
    paused: Arc<AtomicBool>,
}

impl std::hash::Hash for VideoFrameStreamData {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.controller.hash(state);
    }
}

fn video_frame_stream(
    VideoFrameStreamData {
        controller,
        paused: playback_paused,
    }: VideoFrameStreamData,
) -> impl futures::Stream<Item = PlayerMessage> + Send {
    let inner = controller.inner;
    let frame_rate = inner.info.frame_rate;
    let frame_duration = Duration::from_secs_f64(1.0 / frame_rate.max(1.0));

    iced::stream::channel(
        2,
        async move |mut output: futures::channel::mpsc::Sender<PlayerMessage>| {
            let (sender, receiver) = async_channel::bounded::<PlayerMessage>(2);
            smol::spawn(smol::unblock(move || {
                let mut frames = video_player::VideoPlayerIterator::<false> {
                    inner,
                    current_generation: 0,
                };
                loop {
                    if playback_paused.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    let started = std::time::Instant::now();
                    match frames.next() {
                        Some(Ok(frame)) => match video_player::mat_to_rgba(&frame.mat) {
                            Ok(image) => {
                                if sender
                                    .send_blocking(PlayerMessage::VideoFrame {
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
                                    .send_blocking(PlayerMessage::VideoError(error.to_string()))
                                    .ok();
                                break;
                            }
                        },
                        Some(Err(error)) => {
                            sender
                                .send_blocking(PlayerMessage::VideoError(error.to_string()))
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

impl Composition for Model {
    type Message = Message;
    type Event = Event;
    type ViewContext<'a> = ();
    type UpdateContext<'a> = ();
    type SubscriptionContext<'a> = ();

    fn view(&self, (): Self::ViewContext<'_>) -> Element<'_, Self::Message> {
        view(self)
    }

    fn update(&mut self, message: Self::Message, (): Self::UpdateContext<'_>) -> Self::Event {
        update(self, message)
    }

    fn subscription(&self, (): Self::SubscriptionContext<'_>) -> Subscription<Self::Message> {
        subscription(self)
    }
}
