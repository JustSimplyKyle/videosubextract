// SPDX-License-Identifier: MPL-2.0

mod selection_canvas;

use crate::config::Config;
use crate::video_player::{self, InnerPlayer};
use crate::{fl, video_player::VideoPlayerController, video_player::VideoPlayerIterator};
use cosmic::app::context_drawer;
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::widget::Stack;
use cosmic::iced::{self, Alignment, Length, Subscription, event, futures};
use cosmic::prelude::*;
use cosmic::widget::Widget;

use cosmic::widget::{self, about::About, icon, menu, nav_bar};
use ffmpeg_the_third::{self as ffmpeg, codec};
use futures::SinkExt;
use image::{DynamicImage, RgbaImage, imageops};
use opencv::core::{MatTraitConst, MatTraitConstManual};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const APP_ICON: &[u8] = include_bytes!("../resources/icons/hicolor/scalable/apps/icon.svg");

pub struct AppModel {
    core: cosmic::Core,
    context_page: ContextPage,
    about: About,
    nav: nav_bar::Model,
    key_binds: HashMap<menu::KeyBind, MenuAction>,
    config: Config,
    time: u32,
    watch_is_active: bool,
    screenshot_selection: Option<iced::Rectangle>,
    canvas_dimensions: iced::Rectangle,
    canvas_generation: u32,
    video_controller: Option<VideoPlayerController>,
    video_allocation: Option<iced::advanced::image::Allocation>,
    // Track if the GPU is currently busy
    is_allocating_frame: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    LaunchUrl(String),
    ToggleContextPage(ContextPage),
    ToggleWatch,
    ScreenshotRegion(Option<iced::Rectangle>),
    CanvasSize(iced::Rectangle),
    UpdateConfig(Config),
    WatchTick(u32),
    ResetSelection,
    LoadVideo(std::path::PathBuf),
    VideoFrame(RgbaImage),
    VideoFrameAllocated(
        widget::image::Handle,
        Option<iced::advanced::image::Allocation>,
    ),
    VideoSeekForward(Duration),
    VideoSeekBackward(Duration),
    VideoError(String),
}

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::multi::Executor;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = "dev.mmurphy.Test";

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(
        core: cosmic::Core,
        _flags: Self::Flags,
    ) -> (Self, Task<cosmic::Action<Self::Message>>) {
        let mut nav = nav_bar::Model::default();

        nav.insert()
            .text(fl!("page-id", num = 1))
            .data::<Page>(Page::Page1)
            .icon(icon::from_name("applications-science-symbolic"))
            .activate();

        nav.insert()
            .text(fl!("page-id", num = 2))
            .data::<Page>(Page::Page2)
            .icon(icon::from_name("applications-system-symbolic"));

        nav.insert()
            .text(fl!("page-id", num = 3))
            .data::<Page>(Page::Page3)
            .icon(icon::from_name("applications-games-symbolic"));

        let about = About::default()
            .name(fl!("app-title"))
            .icon(widget::icon::from_svg_bytes(APP_ICON))
            .version(env!("CARGO_PKG_VERSION"))
            .links([(fl!("repository"), REPOSITORY)])
            .license(env!("CARGO_PKG_LICENSE"));

        let mut app = AppModel {
            core,
            context_page: ContextPage::default(),
            about,
            nav,
            key_binds: HashMap::new(),
            config: cosmic_config::Config::new(Self::APP_ID, Config::VERSION)
                .map(|context| match Config::get_entry(&context) {
                    Ok(config) => config,
                    Err((_errors, config)) => config,
                })
                .unwrap_or_default(),
            time: 0,
            watch_is_active: false,
            screenshot_selection: None,
            canvas_generation: 0,
            canvas_dimensions: iced::Rectangle {
                x: 0.,
                y: 0.,
                width: 1.,
                height: 1.,
            },
            video_controller: None,
            // current_video_frame: None,
            video_allocation: None,
            is_allocating_frame: false,
        };

        let command = app.update_title();
        (app, command)
    }

    fn header_start(&self) -> Vec<Element<'_, Self::Message>> {
        let menu_bar = menu::bar(vec![menu::Tree::with_children(
            menu::root(fl!("view")).apply(Element::from),
            menu::items(
                &self.key_binds,
                vec![menu::Item::Button(fl!("about"), None, MenuAction::About)],
            ),
        )]);

        vec![menu_bar.into()]
    }

    fn nav_model(&self) -> Option<&nav_bar::Model> {
        Some(&self.nav)
    }

    fn context_drawer(&self) -> Option<context_drawer::ContextDrawer<'_, Self::Message>> {
        if !self.core.window.show_context {
            return None;
        }

        Some(match self.context_page {
            ContextPage::About => context_drawer::about(
                &self.about,
                |url| Message::LaunchUrl(url.to_string()),
                Message::ToggleContextPage(ContextPage::About),
            ),
        })
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let space_s = cosmic::theme::spacing().space_s;
        let content: Element<_> = match self.nav.active_data::<Page>().unwrap() {
            Page::Page1 => {
                let header = widget::row::with_capacity(2)
                    .push(widget::text::title1(fl!("welcome")))
                    .push(widget::text::title3(fl!("page-id", num = 1)))
                    .align_y(Alignment::End)
                    .spacing(space_s);

                let counter_label = ["Watch: ", self.time.to_string().as_str()].concat();
                let section = cosmic::widget::settings::section().add(
                    cosmic::widget::settings::item::builder(counter_label).control(
                        widget::button::text(if self.watch_is_active {
                            "Stop"
                        } else {
                            "Start"
                        })
                        .on_press(Message::ToggleWatch),
                    ),
                );

                widget::column::with_capacity(2)
                    .push(header)
                    .push(section)
                    .spacing(space_s)
                    .height(Length::Fill)
                    .into()
            }

            Page::Page2 => {
                let header = widget::row::with_capacity(2)
                    .push(widget::text::title1(fl!("welcome")))
                    .push(widget::text::title3(fl!("page-id", num = 2)))
                    .align_y(Alignment::End)
                    .spacing(space_s);

                let full_img_handle = self.video_allocation.as_ref().map_or_else(
                    || {
                        Box::leak(Box::new(iced::widget::image::Handle::from_path(
                            "test-2.png",
                        )))
                    },
                    iced::advanced::image::Allocation::handle,
                );

                let (img_width, img_height) = match &full_img_handle {
                    // temp
                    widget::image::Handle::Path(id, path_buf) => (2560, 1440),
                    widget::image::Handle::Bytes(id, bytes) => todo!(),
                    widget::image::Handle::Rgba {
                        id,
                        width,
                        height,
                        pixels,
                    } => (*width, *height),
                };

                let full_img = widget::image(full_img_handle.clone());

                let canvas_widget = widget::canvas(selection_canvas::SelectionProgram {
                    reset_generation: self.canvas_generation,
                })
                .width(Length::Fill)
                .height(Length::Fill);

                let cropped_img = self.screenshot_selection.unwrap_or_default().apply(|ele| {
                    let img_w = img_width as f32;
                    let img_h = img_height as f32;
                    let canvas_w = self.canvas_dimensions.width;
                    let canvas_h = self.canvas_dimensions.height;

                    // Contain fit: uniform scale, picks the axis that fits tighter.
                    let scale = (canvas_w / img_w).min(canvas_h / img_h);

                    // The image is centered inside the canvas — compute the dead-space offsets.
                    let offset_x = (canvas_w - img_w * scale) / 2.0;
                    let offset_y = (canvas_h - img_h * scale) / 2.0;
                    let x = ((ele.x - offset_x) / scale).clamp(0.0, img_w - 1.0);
                    let y = ((ele.y - offset_y) / scale).clamp(0.0, img_h - 1.0);
                    let w = (ele.width / scale).clamp(1.0, img_w - x);
                    let h = (ele.height / scale).clamp(1.0, img_h - y);

                    full_img
                        .crop(iced::Rectangle {
                            x: x as u32,
                            y: y as u32,
                            width: w as u32,
                            height: h as u32,
                        })
                        .width(Length::Shrink)
                        .height(Length::Shrink)
                });

                let full_img = widget::image(full_img_handle)
                    .width(Length::Fill)
                    .height(Length::Shrink);
                let full_img = Stack::new().push(full_img).push(canvas_widget);

                let reset_btn =
                    widget::button::text("Reset Selection").on_press(Message::ResetSelection);

                let load_video = widget::button::text("Load Video")
                    .on_press(Message::LoadVideo("with_subtitle.mkv".into()));

                let skip_backward = widget::button::text("Backwards 5")
                    .on_press(Message::VideoSeekBackward(Duration::from_secs(5)));

                let skip_forward = widget::button::text("Forward 5")
                    .on_press(Message::VideoSeekForward(Duration::from_secs(5)));

                let selection_label = match self.screenshot_selection {
                    Some(r) => format!(
                        "Selection: ({:.0}, {:.0})  {}×{}",
                        r.x, r.y, r.width as u32, r.height as u32
                    ),
                    None => "Click twice on the image to set two corners".into(),
                };

                widget::column! {
                    header,
                    full_img,
                    cropped_img,
                    widget::row! {
                        load_video,
                        reset_btn,
                        widget::text(selection_label),
                        skip_backward,
                        skip_forward
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

            Page::Page3 => {
                let header = widget::row::with_capacity(2)
                    .push(widget::text::title1(fl!("welcome")))
                    .push(widget::text::title3(fl!("page-id", num = 3)))
                    .align_y(Alignment::End)
                    .spacing(space_s);

                widget::column::with_capacity(1)
                    .push(header)
                    .spacing(space_s)
                    .height(Length::Fill)
                    .into()
            }
        };

        widget::container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .apply(widget::container)
            .width(Length::Fill)
            .padding([0, 20])
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .into()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let mut subscriptions = vec![
            self.core()
                .watch_config::<Config>(Self::APP_ID)
                .map(|update| Message::UpdateConfig(update.config)),
        ];
        if self.watch_is_active {
            subscriptions.push(Subscription::run(|| {
                iced::stream::channel(
                    1,
                    |mut emitter: futures::channel::mpsc::Sender<_>| async move {
                        let mut time = 1;
                        let mut interval = tokio::time::interval(Duration::from_secs(1));
                        loop {
                            interval.tick().await;
                            _ = emitter.send(Message::WatchTick(time)).await;
                            time += 1;
                        }
                    },
                )
            }));
        }

        if let Some(ref controller) = self.video_controller {
            subscriptions.push(iced::Subscription::run_with(controller.clone(), |x| {
                video_frame_stream(x.inner.clone(), x.inner.info.frame_rate)
            }));
        }

        Subscription::batch(subscriptions)
    }

    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::WatchTick(time) => {
                self.time = time;
            }

            Message::ToggleWatch => {
                self.watch_is_active = !self.watch_is_active;
            }

            Message::ToggleContextPage(context_page) => {
                if self.context_page == context_page {
                    self.core.window.show_context = !self.core.window.show_context;
                } else {
                    self.context_page = context_page;
                    self.core.window.show_context = true;
                }
            }

            Message::UpdateConfig(config) => {
                self.config = config;
            }

            Message::LaunchUrl(url) => match open::that_detached(&url) {
                Ok(()) => {}
                Err(err) => {
                    eprintln!("failed to open {url:?}: {err}");
                }
            },

            Message::ScreenshotRegion(msg) => {
                self.screenshot_selection = msg;
            }

            Message::ResetSelection => {
                self.screenshot_selection = None;
                self.canvas_generation = self.canvas_generation.wrapping_add(1);
            }
            Message::CanvasSize(rectangle) => {
                self.canvas_dimensions = rectangle;
            }
            Message::LoadVideo(path) => {
                match ffmpeg::format::input(&path) {
                    Ok(input) => match video_player::create_video_player::<false>(input, None) {
                        Ok((controller, _iter)) => {
                            controller.seek_forward(Duration::from_mins(6)).unwrap();
                            // _iter is intentionally dropped; the subscription creates its own
                            // from the shared Arc<InnerPlayer> inside the controller.
                            self.video_controller = Some(controller);
                        }
                        Err(e) => eprintln!("video_player init error: {e}"),
                    },
                    Err(e) => eprintln!("ffmpeg open error: {e}"),
                }
            }

            Message::VideoFrame(frame) => {
                // If the GPU is still busy allocating the last frame,
                // drop this frame to let the UI breathe and keep playback synced.
                if self.is_allocating_frame {
                    println!("ui overdrive");
                    return Task::none();
                }

                self.is_allocating_frame = true;

                let handle = widget::image::Handle::from_rgba(
                    frame.width(),
                    frame.height(),
                    frame.into_raw(),
                );

                // Spawn the allocation Task
                return iced::runtime::image::allocate(&handle)
                    .map(move |result| Message::VideoFrameAllocated(handle.clone(), result.ok()))
                    .map(Into::into);
            }

            Message::VideoFrameAllocated(handle, allocation_opt) => {
                self.is_allocating_frame = false;
                if let Some(allocation) = allocation_opt {
                    // Only update the UI state AFTER the texture has been uploaded to the GPU!
                    // This eliminates the 1-frame flickering gap.
                    self.video_allocation = Some(allocation);
                } else {
                    eprintln!("Failed to allocate video frame on GPU");
                }
            }
            Message::VideoSeekForward(duration) => {
                if let Some(ref controller) = self.video_controller {
                    if let Err(e) = controller.seek_forward(duration) {
                        eprintln!("seek error: {e}");
                    }
                }
            }
            Message::VideoSeekBackward(duration) => {
                if let Some(ref controller) = self.video_controller {
                    if let Err(e) = controller.seek_backward(duration) {
                        eprintln!("seek error: {e}");
                    }
                }
            }

            Message::VideoError(msg) => {
                eprintln!("video error: {msg}");
                self.video_controller = None;
            }
        }
        Task::none()
    }

    fn on_nav_select(&mut self, id: nav_bar::Id) -> Task<cosmic::Action<Self::Message>> {
        self.nav.activate(id);
        self.update_title()
    }
}

impl AppModel {
    pub fn update_title(&mut self) -> Task<cosmic::Action<Message>> {
        let mut window_title = fl!("app-title");

        if let Some(page) = self.nav.text(self.nav.active()) {
            window_title.push_str(" — ");
            window_title.push_str(page);
        }

        self.core
            .main_window_id()
            .map_or_else(Task::none, |id| self.set_window_title(window_title, id))
    }
}

pub enum Page {
    Page1,
    Page2,
    Page3,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum ContextPage {
    #[default]
    About,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuAction {
    About,
}

impl menu::action::MenuAction for MenuAction {
    type Message = Message;

    fn message(&self) -> Self::Message {
        match self {
            MenuAction::About => Message::ToggleContextPage(ContextPage::About),
        }
    }
}

/// Converts an OpenCV BGR Mat into an iced RGBA image handle.
fn mat_to_image_handle(mat: &opencv::core::Mat) -> eyre::Result<RgbaImage> {
    let rows = mat.rows() as u32;
    let cols = mat.cols() as u32;
    let bgr = mat.data_bytes()?;
    let rgba = bgr
        .chunks_exact(3)
        .flat_map(|p| [p[2], p[1], p[0], 255u8])
        .collect::<Vec<_>>();
    RgbaImage::from_raw(cols, rows, rgba).ok_or(eyre::eyre!("isn't a valid rgbaimage"))
}

/// Drives the video iterator on a blocking thread and emits VideoFrame messages
/// at the correct frame rate. Runs until EOF or the receiver is dropped.
fn video_frame_stream(
    inner: Arc<InnerPlayer>,
    frame_rate: f64,
) -> impl futures::Stream<Item = Message> + Send {
    let frame_dur = Duration::from_secs_f64(1.0 / frame_rate.max(1.0));
    // let frame_dur = Duration::from_secs_f64(0.1);

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
                        Some(Ok(mat)) => match mat_to_image_handle(&mat) {
                            Ok(handle) => {
                                if btx.blocking_send(Message::VideoFrame(handle)).is_err() {
                                    break; // receiver dropped (app closed / video changed)
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
                        None => break, // EOF
                    }
                    // Sleep the remainder of the frame budget so we don't busy-spin.
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
