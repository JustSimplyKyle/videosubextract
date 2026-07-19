use crate::apply_traits::ApplyConditional;

use super::*;
use cosmic::iced::widget::Stack;
use iced::{futures::SinkExt, id::Id};
use rfd::AsyncFileDialog;
use std::{
    env::current_dir,
    hash::{DefaultHasher, Hash, Hasher},
    time::Duration,
};

#[derive(Default)]
pub struct Model {
    pub video_path: Option<std::path::PathBuf>,
    pub video_controller: Option<VideoPlayerController>,
    pub video_allocation: Option<(iced::advanced::image::Allocation, iced::Size)>,
    pub is_allocating_frame: bool,
    pub screenshot_selection: Option<iced::Rectangle>,
    pub screenshot_selection_scaled: Option<iced::Rectangle>,
    pub canvas_dimensions: iced::Rectangle,
    pub canvas_generation: u32,
}

#[derive(Debug, Clone)]
pub enum Message {
    ResetSelection,
    Canvas(selection_canvas::Message),
    PickVideo,
    VideoFilePicked(Option<std::path::PathBuf>),
    LoadVideo(std::path::PathBuf),
    VideoFrame(RgbaImage),
    VideoFrameAllocated(Option<(iced::advanced::image::Allocation, iced::Size)>),
    VideoSeekForward(Duration),
    VideoSeekBackward(Duration),
    VideoError(String),
    StartSubtitleDisplay,
}

pub enum Event {
    StartSubtitleSearch(std::path::PathBuf, Option<iced::Rectangle>),
    Run(Task<Message>),
    None,
}

impl Model {
    pub fn update(&mut self, message: Message) -> Event {
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
                            "Video",
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
                    Ok(input) => match create_video_player::<false>(input, None) {
                        Ok((controller, _iter)) => {
                            self.video_path = Some(path);
                            self.video_controller = Some(controller);
                        }
                        Err(e) => eprintln!("video_player init error: {e}"),
                    },
                    Err(e) => eprintln!("ffmpeg open error: {e}"),
                }
                Event::None
            }
            Message::VideoFrame(frame) => {
                if self.is_allocating_frame {
                    println!("ui overdrive");
                    return Event::None;
                }

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
                iced::runtime::image::allocate(&handle)
                    .map(move |result| {
                        Message::VideoFrameAllocated(
                            result
                                .ok()
                                .map(|x| (x, iced::Size::new(width as f32, height as f32))),
                        )
                    })
                    .apply(Event::Run)
            }
            Message::VideoFrameAllocated(allocation_opt) => {
                self.is_allocating_frame = false;
                if let Some(allocation) = allocation_opt {
                    self.video_allocation = Some(allocation);
                } else {
                    eprintln!("Failed to allocate video frame on GPU");
                }
                Event::None
            }
            Message::VideoSeekForward(duration) => {
                if let Some(ref controller) = self.video_controller {
                    if let Err(e) = controller.seek_forward(duration) {
                        eprintln!("seek error: {e}");
                    }
                }
                Event::None
            }
            Message::VideoSeekBackward(duration) => {
                if let Some(ref controller) = self.video_controller {
                    if let Err(e) = controller.seek_backward(duration) {
                        eprintln!("seek error: {e}");
                    }
                }
                Event::None
            }
            Message::VideoError(msg) => {
                eprintln!("video error: {msg}");
                self.video_controller = None;
                Event::None
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

    pub fn view(&self) -> Element<'_, Message> {
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

        let reset_btn = widget::button::text("Reset Selection")
            .on_press(Message::ResetSelection)
            .class(cosmic::theme::Button::Destructive);

        let load_video = widget::button::text(if self.video_path.is_none() {
            "Load Video"
        } else {
            "Change Video"
        })
        .on_press(Message::PickVideo);

        let load_video = if self.video_path.is_none() {
            load_video.class(cosmic::theme::Button::Suggested)
        } else {
            load_video.class(cosmic::theme::Button::Standard)
        };

        let skip_backward = widget::button::icon(icon::from_name("media-seek-backward-symbolic"))
            .on_press(Message::VideoSeekBackward(Duration::from_secs(5)))
            .class(cosmic::theme::Button::NavToggle);

        let skip_forward = widget::button::icon(icon::from_name("media-seek-forward-symbolic"))
            .on_press(Message::VideoSeekForward(Duration::from_secs(5)))
            .class(cosmic::theme::Button::NavToggle);

        let selection_label = self
            .screenshot_selection_scaled
            .map_or_else(
                || "Click twice on the image to two corners".into(),
                |r| {
                    format!(
                        "Selection: ({:.0}, {:.0})  {:.0}×{:.0}",
                        r.x, r.y, r.width, r.height
                    )
                },
            )
            .apply(widget::text)
            .class(cosmic::theme::Text::Accent);

        let find_subs = widget::button::text("Find Subtitles");
        let find_subs = if self.video_path.is_some() {
            find_subs
                .on_press(Message::StartSubtitleDisplay)
                .class(cosmic::theme::Button::Suggested)
        } else {
            find_subs
        };

        widget::column! {
            full_img,
            cropped_img,
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

    pub fn subscription(&self) -> Subscription<Message> {
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

fn video_frame_stream(
    inner: Arc<InnerPlayer>,
    frame_rate: f64,
) -> impl futures::Stream<Item = Message> + Send {
    let frame_dur = Duration::from_secs_f64(1.0 / frame_rate.max(1.0));

    iced::stream::channel(
        2,
        async move |mut tx: futures::channel::mpsc::Sender<Message>| {
            let (btx, mut brx) = tokio::sync::mpsc::channel::<Message>(2);

            tokio::task::spawn_blocking(move || {
                let mut iter = video_player::VideoPlayerIterator::<false> {
                    inner,
                    current_generation: 0,
                };
                loop {
                    let t = std::time::Instant::now();
                    match iter.next() {
                        Some(Ok(mat)) => match super::mat_to_image_handle(&mat.mat) {
                            Ok(handle) => {
                                if btx.blocking_send(Message::VideoFrame(handle)).is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                let _ = btx.blocking_send(Message::VideoError(e.to_string()));
                                break;
                            }
                        },
                        Some(Err(e)) => {
                            let _ = btx.blocking_send(Message::VideoError(e.to_string()));
                            break;
                        }
                        None => break,
                    }
                    if let Some(rem) = frame_dur.checked_sub(t.elapsed()) {
                        std::thread::sleep(rem);
                    }
                }
            });

            while let Some(msg) = brx.recv().await {
                if tx.send(msg).await.is_err() {
                    break;
                }
            }
        },
    )
}
