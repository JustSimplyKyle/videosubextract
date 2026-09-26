use crate::apply_traits::ApplyConditional;

use super::*;
use cosmic::iced::widget::Stack;
use cosmic::{Apply, Element};
use iced::alignment::Horizontal;
use iced::futures::SinkExt;
use image::RgbaImage;
use rfd::AsyncFileDialog;
use std::{env::current_dir, time::Duration};
use vse_ui as cosmic;

#[derive(Default)]
pub struct Model {
    pub video_path: Option<std::path::PathBuf>,
    pub video_controller: Option<VideoPlayerController>,
    pub video_allocation: Option<(widget::image::Allocation, iced::Size)>,
    pub is_allocating_frame: bool,
    pub screenshot_selection: Option<iced::Rectangle>,
    pub screenshot_selection_scaled: Option<iced::Rectangle>,
    pub canvas_dimensions: iced::Rectangle,
    pub canvas_generation: u32,
    current_time: Duration,
}

#[derive(derive_more::Debug, Clone)]
pub enum Message {
    ResetSelection,
    Canvas(selection_canvas::Message),
    PickVideo,
    VideoFilePicked(Option<std::path::PathBuf>),
    LoadVideo(std::path::PathBuf),

    #[debug("{}x{}@{}", image.width(), image.height(), format_duration(*timestamp))]
    VideoFrame {
        image: RgbaImage,
        timestamp: Duration,
    },
    VideoFrameAllocated(Result<(widget::image::Allocation, iced::Size), String>),
    VideoSeekForward(Duration),
    VideoSeekBackward(Duration),
    VideoSeekAbsolute(Duration),
    CopySelectionDimensions(String),
    VideoError(String),
    StartSubtitleDisplay,
}

pub enum Event {
    StartSubtitleSearch(std::path::PathBuf, Option<iced::Rectangle>),
    Run(Task<Message>),
    CopySelectionDimensions(String),
    Error(eyre::Report),
    None,
}

impl Model {
    fn update(&mut self, message: Message) -> Event {
        let needs_recompute = Self::scaled_selection_needs_recomputation(&message);

        let task = match message {
            Message::ResetSelection => {
                self.screenshot_selection = None;
                self.canvas_generation = self.canvas_generation.wrapping_add(1);
                Event::None
            }
            Message::Canvas(x) => match x {
                selection_canvas::Message::ScreenshotRegion(rectangle) => {
                    self.screenshot_selection = rectangle;
                    Event::None
                }
                selection_canvas::Message::CanvasSize(rectangle) => {
                    self.canvas_dimensions = rectangle;
                    Event::None
                }
            },
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

                        file.map(|f| f.path().to_path_buf())
                    },
                    Message::VideoFilePicked,
                )
                .apply(Event::Run)
            }
            Message::VideoFilePicked(Some(path)) => self.update(Message::LoadVideo(path)),
            Message::VideoFilePicked(None) => Event::None,
            Message::LoadVideo(path) => {
                match ffmpeg_the_third::format::input(&path) {
                    Ok(input) => match create_video_player::<false>(
                        input,
                        None,
                        crate::config::ProcessingResolution::None,
                    ) {
                        Ok((controller, _iter)) => {
                            self.video_path = Some(path);
                            self.video_controller = Some(controller);
                        }
                        Err(error) => {
                            return Event::Error(error.wrap_err("initializing the video player"));
                        }
                    },
                    Err(error) => {
                        return Event::Error(
                            eyre::eyre!(error).wrap_err("opening the video with FFmpeg"),
                        );
                    }
                }
                Event::None
            }
            Message::VideoFrame {
                image: frame,
                timestamp,
            } => {
                if self.is_allocating_frame {
                    println!("ui overdrive");
                    return Event::None;
                }

                self.current_time = timestamp;

                // let mut hasher = DefaultHasher::new();
                // Id::unique().0.hash(&mut hasher);
                // frame.save(format!("test/{}.png", hasher.finish())).unwrap();

                self.is_allocating_frame = true;
                let (width, height) = (frame.width(), frame.height());
                let handle = widget::image::Handle::from_rgba(
                    frame.width(),
                    frame.height(),
                    frame.into_raw(),
                );
                widget::image::allocate(handle)
                    .map(move |result| {
                        Message::VideoFrameAllocated(
                            result
                                .map_err(|error| error.to_string())
                                .map(|x| (x, iced::Size::new(width as f32, height as f32))),
                        )
                    })
                    .apply(Event::Run)
            }
            Message::VideoFrameAllocated(allocation) => {
                self.is_allocating_frame = false;
                match allocation {
                    Ok(allocation) => self.video_allocation = Some(allocation),
                    Err(error) => {
                        return Event::Error(eyre::eyre!(
                            "failed to allocate video frame on GPU: {error}"
                        ));
                    }
                }
                Event::None
            }
            Message::VideoSeekForward(duration) => {
                if let Some(ref controller) = self.video_controller
                    && let Err(error) = controller.seek_forward(duration)
                {
                    return Event::Error(error.wrap_err("seeking forward"));
                }
                Event::None
            }
            Message::VideoSeekBackward(duration) => {
                if let Some(ref controller) = self.video_controller
                    && let Err(error) = controller.seek_backward(duration)
                {
                    return Event::Error(error.wrap_err("seeking backward"));
                }
                Event::None
            }
            Message::VideoSeekAbsolute(duration) => {
                self.current_time = duration;

                if let Some(ref controller) = self.video_controller
                    && let Err(error) = controller.seek_absolute(duration)
                {
                    return Event::Error(
                        error.wrap_err(format!("seeking to {:.2}s", duration.as_secs_f64())),
                    );
                }
                Event::None
            }
            Message::CopySelectionDimensions(dimensions) => {
                Event::CopySelectionDimensions(dimensions)
            }
            Message::VideoError(msg) => {
                self.video_controller = None;
                Event::Error(eyre::eyre!("video playback failed: {msg}"))
            }
            Message::StartSubtitleDisplay => {
                if let Some(path) = &self.video_path {
                    Event::StartSubtitleSearch(path.clone(), self.screenshot_selection_scaled)
                } else {
                    Event::None
                }
            }
        };

        if needs_recompute {
            self.recompute_scaled_selection();
        }
        task
    }

    fn view(&self) -> Element<'_, Message> {
        let space_s = cosmic::theme::spacing().space_s;

        let full_img_handle = self.video_allocation.as_ref().map_or_else(
            || widget::image::Handle::from_rgba(1920, 1080, RgbaImage::new(1920, 1080).to_vec()),
            |(img, _)| img.handle().clone(),
        );

        let full_img = widget::image(&full_img_handle)
            .content_fit(iced::ContentFit::Contain)
            .width(Length::Fill)
            .height(Length::Shrink);

        let canvas_widget = widget::canvas(selection_canvas::SelectionProgram {
            reset_generation: self.canvas_generation,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .apply(Element::from)
        .map(Message::Canvas);

        let cropped_img = self
            .screenshot_selection_scaled
            .unwrap_or_default()
            .apply(|ele| {
                let region = iced::Rectangle {
                    x: ele.x as u32,
                    y: ele.y as u32,
                    width: ele.width as u32,
                    height: ele.height as u32,
                };

                widget::image(full_img_handle)
                    .crop(region)
                    .width(Length::Shrink)
                    .height(Length::Shrink)
            });

        let full_img = Stack::new().push(full_img).push(canvas_widget);

        let reset_btn = widget::button(widget::text(fl!("reset-selection")))
            .on_press(Message::ResetSelection)
            .style(cosmic::theme::button::destructive);

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

        let skip_backward = widget::button(icon::from_name("media-seek-backward-symbolic"))
            .on_press(Message::VideoSeekBackward(Duration::from_secs(5)))
            .style(cosmic::theme::button::nav_toggle);

        let skip_forward = widget::button(icon::from_name("media-seek-forward-symbolic"))
            .on_press(Message::VideoSeekForward(Duration::from_secs(5)))
            .style(cosmic::theme::button::nav_toggle);
        let selection_label: Element<'_, Message> = self.screenshot_selection_scaled.map_or_else(
            || {
                widget::text(fl!("select-region"))
                    .style(cosmic::theme::text::accent)
                    .into()
            },
            |rectangle| {
                let dimensions = format!(
                    "{:.0}×{:.0}@{:.0},{:.0}",
                    rectangle.width, rectangle.height, rectangle.x, rectangle.y
                );
                let label = widget::text(fl!("selection", dimensions = dimensions.clone()))
                    .style(cosmic::theme::text::accent);

                widget::mouse_area(label)
                    .on_press(Message::CopySelectionDimensions(dimensions))
                    .interaction(iced::mouse::Interaction::Pointer)
                    .into()
            },
        );

        let find_subs = widget::button(widget::text(fl!("find-subtitles")));
        let find_subs = if self.video_path.is_some() {
            find_subs
                .on_press(Message::StartSubtitleDisplay)
                .style(cosmic::theme::button::suggested)
        } else {
            find_subs
        };

        let slider = self
            .video_controller
            .as_ref()
            .map(|x| x.inner.info.video_time.as_secs_f64())
            .map(|video_time| {
                widget::slider(0.0..=video_time, self.current_time.as_secs_f64(), |x| {
                    Message::VideoSeekAbsolute(Duration::from_secs_f64(x))
                })
            });

        let current_time = widget::text(self.current_time.apply(format_duration))
            .width(Length::Fill)
            .align_x(Horizontal::Right);

        widget::column! {
            full_img,
            cropped_img,
            slider,
            current_time,
            widget::row! {
                load_video,
                reset_btn,
                selection_label,
                skip_backward,
                skip_forward,
                find_subs,
            }
            .spacing(space_s)
            .align_y(Alignment::Center)
        }
        .spacing(space_s)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .apply(widget::scrollable)
        .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![];
        if let Some(ref controller) = self.video_controller {
            subscriptions.push(iced::Subscription::run_with(controller.clone(), |x| {
                video_frame_stream(x.inner.clone(), x.inner.info.frame_rate)
            }));
        }
        Subscription::batch(subscriptions)
    }

    fn recompute_scaled_selection(&mut self) {
        let Some((_, size)) = self.video_allocation.as_ref() else {
            self.screenshot_selection_scaled = None;
            return;
        };
        let Some(ele) = self.screenshot_selection else {
            self.screenshot_selection_scaled = None;
            return;
        };

        let (img_w, img_h) = (size.width, size.height);
        let canvas_w = self.canvas_dimensions.width;
        let canvas_h = self.canvas_dimensions.height;

        let scale = (canvas_w / img_w).min(canvas_h / img_h);
        let offset_x = (canvas_w - img_w * scale) / 2.0;
        let offset_y = (canvas_h - img_h * scale) / 2.0;

        self.screenshot_selection_scaled = Some(iced::Rectangle {
            x: ((ele.x - offset_x) / scale).clamp(0.0, img_w - 1.0),
            y: ((ele.y - offset_y) / scale).clamp(0.0, img_h - 1.0),
            width: (ele.width / scale).clamp(1.0, img_w),
            height: (ele.height / scale).clamp(1.0, img_h),
        });
    }

    const fn scaled_selection_needs_recomputation(message: &Message) -> bool {
        matches!(
            message,
            Message::VideoFrameAllocated(_) | Message::Canvas(_) | Message::ResetSelection
        )
    }
}

impl Composition for Model {
    type Message = Message;
    type Event = Event;
    type ViewContext<'a> = ();
    type UpdateContext<'a> = ();
    type SubscriptionContext<'a> = ();

    fn view(&self, (): Self::ViewContext<'_>) -> Element<'_, Self::Message> {
        Self::view(self)
    }

    fn update(&mut self, message: Self::Message, (): Self::UpdateContext<'_>) -> Self::Event {
        Self::update(self, message)
    }

    fn subscription(&self, (): Self::SubscriptionContext<'_>) -> Subscription<Self::Message> {
        Self::subscription(self)
    }
}

fn video_frame_stream(
    inner: Arc<InnerPlayer>,
    frame_rate: f64,
) -> impl futures::Stream<Item = Message> + Send {
    let frame_dur = Duration::from_secs_f64(1.0 / frame_rate.max(1.0));

    iced::stream::channel(
        2,
        async move |mut tx: futures::channel::mpsc::Sender<Message>| {
            let (btx, brx) = async_channel::bounded::<Message>(2);

            smol::spawn(smol::unblock(move || {
                let mut iter = video_player::VideoPlayerIterator::<false> {
                    inner,
                    current_generation: 0,
                };
                loop {
                    let t = std::time::Instant::now();
                    match iter.next() {
                        Some(Ok(mat)) => match video_player::mat_to_rgba(&mat.mat) {
                            Ok(handle) => {
                                if btx
                                    .send_blocking(Message::VideoFrame {
                                        image: handle,
                                        timestamp: mat.timestamp,
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(e) => {
                                btx.send_blocking(Message::VideoError(e.to_string())).ok();
                                break;
                            }
                        },
                        Some(Err(e)) => {
                            btx.send_blocking(Message::VideoError(e.to_string())).ok();
                            break;
                        }
                        None => break,
                    }
                    if let Some(rem) = frame_dur.checked_sub(t.elapsed()) {
                        std::thread::sleep(rem);
                    }
                }
            }))
            .detach();

            while let Ok(msg) = brx.recv().await {
                if tx.send(msg).await.is_err() {
                    break;
                }
            }
        },
    )
}
