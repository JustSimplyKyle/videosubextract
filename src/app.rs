// SPDX-License-Identifier: MPL-2.0

mod selection_canvas;

use crate::OCR;
use crate::config::Config;
use crate::subfinder::{Params, SubtitleSearch};
use crate::video_player::{self, InnerPlayer, VideoFrame, create_video_player};
use crate::{fl, video_player::VideoPlayerController, video_player::VideoPlayerIterator};
use cosmic::app::context_drawer;
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::widget::Stack;
use cosmic::iced::{self, Alignment, Length, Subscription, event, futures};
use cosmic::prelude::*;
use cosmic::widget::Widget;

use cosmic::widget::segmented_button::SingleSelectModel;
use cosmic::widget::{self, about::About, icon, menu, nav_bar, tab_bar};
use ffmpeg_sidecar::child::FfmpegChild;
use ffmpeg_sidecar::command::FfmpegCommand;
use ffmpeg_the_third::{self as ffmpeg, codec};
use futures::SinkExt;
use image::{DynamicImage, RgbaImage, imageops};
use opencv::core::{MatTraitConst, MatTraitConstManual};
use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use rfd::AsyncFileDialog;

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
    video_path: Option<std::path::PathBuf>,
    watch_is_active: bool,
    screenshot_selection: Option<iced::Rectangle>,
    screenshot_selection_scaled: Option<iced::Rectangle>,
    canvas_dimensions: iced::Rectangle,
    canvas_generation: u32,
    video_controller: Option<VideoPlayerController>,
    video_allocation: Option<(iced::advanced::image::Allocation, iced::Size)>,
    // Track if the GPU is currently busy
    is_allocating_frame: bool,
    subtitle_page_id: nav_bar::Id,
    video_frame_rate: f64,
    subtitle: SubtitleStatus,
    post_production_page_id: nav_bar::Id,
    post_prod_tabs: SingleSelectModel,
    post_prod_feedback: Option<String>,
}

#[derive(Default)]
pub struct SubtitleStatus {
    search_active: bool,
    search_gen: usize,
    search_path: Option<std::path::PathBuf>,
    search_selection: Option<iced::Rectangle>,
    results: Vec<SubtitleResult>,
    preview: Option<widget::image::Handle>,
    current_frame: usize,
    done: bool,
}

impl SubtitleStatus {
    /// Converts frame numbers to an SRT timestamp: `HH:MM:SS,mmm`
    fn frame_to_srt_timestamp(frame: Duration) -> String {
        let total_ms = frame.as_millis();
        let ms = total_ms % 1000;
        let s = (total_ms / 1000) % 60;
        let m = (total_ms / 60_000) % 60;
        let h = total_ms / 3_600_000;
        format!("{h:02}:{m:02}:{s:02},{ms:03}")
    }

    pub fn to_srt(&self) -> String {
        use std::fmt::Write;

        self.results
            .iter()
            .enumerate()
            .fold(String::new(), |mut out, (i, r)| {
                let start = Self::frame_to_srt_timestamp(r.start_timestamp);
                let end = Self::frame_to_srt_timestamp(r.end_timestamp);
                // SRT indices are 1-based
                let _ = write!(
                    out,
                    "{}\n{} --> {}\n{}\n\n",
                    i + 1,
                    start,
                    end,
                    r.text.trim()
                );
                out
            })
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    LaunchUrl(String),
    ToggleContextPage(ContextPage),
    ScreenshotRegion(Option<iced::Rectangle>),
    CanvasSize(iced::Rectangle),
    UpdateConfig(Config),
    WatchTick(u32),
    ResetSelection,
    PickVideo,
    VideoFilePicked(Option<std::path::PathBuf>),
    LoadVideo(std::path::PathBuf),
    VideoFrame(RgbaImage),
    VideoFrameAllocated(Option<(iced::advanced::image::Allocation, iced::Size)>),
    VideoSeekForward(Duration),
    VideoSeekBackward(Duration),
    VideoError(String),
    ConvertToSrt,
    SrtSaved(Option<std::path::PathBuf>),
    StartSubtitleDisplay(std::path::PathBuf),
    SubtitleProgress {
        frame: usize,
        preview: RgbaImage,
    },
    SubtitleEventFound {
        start_timestamp: Duration,
        end_timestamp: Duration,
        text: String,
        preview: RgbaImage,
    },
    SubtitleSearchDone,
    SubtitleSearchError(String),
    GoToPostProduction,
    SelectPostProdTab(widget::segmented_button::Entity),
    MergeWithVideo,
    OpenCcS2T,
    OpenCcT2S,
}

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::multi::Executor;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = "dev.justsimplykyle.videosubextract";

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

        let prepare_id = nav
            .insert()
            .text(fl!("page-prepare"))
            .data::<Page>(Page::Prepare)
            .icon(icon::from_name("applications-system-symbolic"))
            .id();

        nav.activate(prepare_id);

        let subtitle_page_id = nav
            .insert()
            .text(fl!("page-subtitle"))
            .data::<Page>(Page::Subtitle)
            .icon(icon::from_name("applications-games-symbolic"))
            .id();

        let post_production_page_id = nav
            .insert()
            .text(fl!("page-post"))
            .data::<Page>(Page::PostProduction)
            .icon(icon::from_name("applications-games-symbolic"))
            .id();

        let mut post_prod_tabs = SingleSelectModel::default();
        let srt_tab = post_prod_tabs
            .insert()
            .text("Convert to SRT")
            .data::<PostProdTab>(PostProdTab::Srt)
            .id();
        post_prod_tabs
            .insert()
            .text("Merge Video")
            .data::<PostProdTab>(PostProdTab::Merge);
        post_prod_tabs
            .insert()
            .text("OpenCC Translate")
            .data::<PostProdTab>(PostProdTab::OpenCc);
        post_prod_tabs.activate(srt_tab);

        let about = About::default()
            .name(fl!("app-title"))
            .icon(widget::icon::from_svg_bytes(APP_ICON))
            .version(env!("CARGO_PKG_VERSION"))
            .links([(fl!("repository"), REPOSITORY)])
            .license(env!("CARGO_PKG_LICENSE"));

        let mut app = Self {
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
            video_allocation: None,
            is_allocating_frame: false,
            screenshot_selection_scaled: None,
            video_path: None,
            subtitle: Default::default(),
            subtitle_page_id,
            post_production_page_id,
            post_prod_tabs,
            post_prod_feedback: None,
            video_frame_rate: 24.0,
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
            Page::Prepare => {
                let header = widget::row::with_capacity(2)
                    .push(widget::text::title1(fl!("welcome")))
                    .push(widget::text::title3(fl!("page-prepare")))
                    .align_y(Alignment::End)
                    .spacing(space_s);

                let (full_img_handle, img_width, img_height) =
                    self.video_allocation.as_ref().map_or_else(
                        || {
                            (
                                (widget::image::Handle::from_rgba(
                                    1920,
                                    1080,
                                    RgbaImage::new(1920, 1080).to_vec(),
                                )),
                                1920.,
                                1080.,
                            )
                        },
                        |(img, size)| (img.handle().clone(), size.width, size.height),
                    );

                let full_img = widget::image(&full_img_handle)
                    .content_fit(iced::ContentFit::Contain)
                    .width(Length::Fill)
                    .height(Length::Shrink);

                let canvas_widget = widget::canvas(selection_canvas::SelectionProgram {
                    reset_generation: self.canvas_generation,
                })
                .width(Length::Fill)
                .height(Length::Fill);

                let cropped_img = self.screenshot_selection.unwrap_or_default().apply(|ele| {
                    let img_w = img_width;
                    let img_h = img_height;
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

                    let region = iced::Rectangle {
                        x: x as u32,
                        y: y as u32,
                        width: w as u32,
                        height: h as u32,
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

                let skip_backward =
                    widget::button::icon(icon::from_name("media-seek-backward-symbolic"))
                        .on_press(Message::VideoSeekBackward(Duration::from_secs(5)))
                        .class(cosmic::theme::Button::NavToggle);

                let skip_forward =
                    widget::button::icon(icon::from_name("media-seek-forward-symbolic"))
                        .on_press(Message::VideoSeekForward(Duration::from_secs(5)))
                        .class(cosmic::theme::Button::NavToggle);

                let selection_label = self
                    .screenshot_selection_scaled
                    .map_or_else(
                        || "Click twice on the image to set two corners".into(),
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
                let find_subs = if let Some(p) = &self.video_path {
                    find_subs
                        .on_press(Message::StartSubtitleDisplay(p.clone()))
                        .class(cosmic::theme::Button::Suggested)
                } else {
                    find_subs
                };

                widget::column! {
                    header,
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

            Page::Subtitle => {
                let header = widget::row::with_capacity(2)
                    .push(widget::text::title1(fl!("welcome")))
                    .push(widget::text::title3(fl!("page-subtitle")))
                    .align_y(Alignment::End)
                    .spacing(space_s);

                let fps = self.video_frame_rate.max(1.0);

                let status: Element<_> = if self.subtitle.done {
                    widget::text(format!(
                        "Complete — {} subtitle(s) found",
                        self.subtitle.results.len()
                    ))
                    .class(cosmic::theme::Text::Accent)
                    .into()
                } else if self.subtitle.search_active {
                    let current_sec = self.subtitle.current_frame as f64 / fps;
                    let elapsed_string = format!(
                        "{:02.0}:{:02.0}:{:02.0}",
                        (current_sec / 3600.0),
                        ((current_sec % 3600.0) / 60.0),
                        (current_sec % 60.0)
                    );

                    let status_text = widget::text(format!(
                        "Scanning... Elapsed: {} (frame {})",
                        elapsed_string, self.subtitle.current_frame
                    ))
                    .class(cosmic::theme::Text::Accent);

                    let progress_bar = widget::progress_bar::determinate_linear(
                        self.subtitle.current_frame as f32
                            / self
                                .video_controller
                                .as_ref()
                                .map(|x| x.inner.info.total_frames)
                                .unwrap_or_default() as f32,
                    )
                    .width(Length::Fill);

                    widget::row!(status_text, progress_bar)
                        .spacing(space_s)
                        .align_y(Alignment::Center)
                        .width(Length::Fill)
                        .into()
                } else {
                    widget::text(
                        "No active search. Load a video and select a subtitle region on Page Prepare.",
                    )
                    .class(cosmic::theme::Text::Accent)
                    .into()
                };

                let to_post_prod = widget::button::text("Post Production")
                    .class(cosmic::theme::Button::Suggested)
                    .on_press_maybe(
                        (!self.subtitle.search_active).then_some(Message::GoToPostProduction),
                    );

                let grid = widget::responsive(move |size| {
                    self.subtitle
                        .results
                        .iter()
                        .fold(widget::grid(), |grid, r| {
                            let t_start = r.start_timestamp.as_secs_f64();
                            let t_end = r.end_timestamp.as_secs_f64();

                            let text_width = dbg!((size.width * 0.35).max(160.0)); // 35% of available, floored at 160
                            let img_width = dbg!(size.width - text_width);

                            let img = widget::image(r.preview.clone())
                                .content_fit(iced::ContentFit::Contain)
                                .width(img_width) // grid column 0 determines the width; all cells inherit it
                                .height(Length::Shrink);

                            let timeline = widget::text(format!("{t_start:.1}s – {t_end:.1}s"));

                            let ocr =
                                widget::text(r.text.trim()).class(cosmic::theme::Text::Accent);

                            grid.push(img)
                                .push(
                                    widget::column![timeline, ocr]
                                        .spacing(space_s / 2)
                                        .width(text_width)
                                        .align_x(Alignment::Start),
                                )
                                .insert_row()
                        })
                        .row_spacing(space_s)
                        .row_alignment(Alignment::Center)
                        .column_spacing(space_s)
                        .padding([0, 40].into())
                        .into()
                });

                let mut col =
                    widget::column![header, widget::row!(status, to_post_prod).spacing(space_s)]
                        .spacing(space_s);

                if let Some(handle) = &self.subtitle.preview {
                    col = col.push(
                        widget::image(handle)
                            .width(Length::Fill)
                            .height(Length::Fixed(120.))
                            .content_fit(iced::ContentFit::Contain),
                    );
                }

                col.push(grid)
                    .height(Length::Fill)
                    .apply(widget::scrollable)
                    .into()
            }
            Page::PostProduction => {
                let header = widget::row::with_capacity(2)
                    .push(widget::text::title1(fl!("welcome")))
                    .push(widget::text::title3(fl!("page-post")))
                    .align_y(Alignment::End)
                    .spacing(space_s);

                let tabs = widget::tab_bar::horizontal(&self.post_prod_tabs)
                    .on_activate(Message::SelectPostProdTab);

                let tab_content: Element<_> = match self.post_prod_tabs.active_data::<PostProdTab>()
                {
                    Some(PostProdTab::Srt) => {
                        let btn = widget::button::text("Save as SRT")
                            .class(cosmic::theme::Button::Suggested)
                            .on_press_maybe(
                                (!self.subtitle.search_active).then_some(Message::ConvertToSrt),
                            );
                        widget::column![btn].into()
                    }
                    Some(PostProdTab::Merge) => {
                        let btn = widget::button::text("Merge Subtitles with Video")
                            .class(cosmic::theme::Button::Suggested)
                            .on_press_maybe(
                                (!self.subtitle.search_active).then_some(Message::MergeWithVideo),
                            );
                        widget::column![btn].into()
                    }
                    Some(PostProdTab::OpenCc) => {
                        let btn_s2t = widget::button::text("Simplified to Traditional")
                            .class(cosmic::theme::Button::Suggested)
                            .on_press_maybe(
                                (!self.subtitle.search_active).then_some(Message::OpenCcS2T),
                            );
                        let btn_t2s = widget::button::text("Traditional to Simplified")
                            .class(cosmic::theme::Button::Suggested)
                            .on_press_maybe(
                                (!self.subtitle.search_active).then_some(Message::OpenCcT2S),
                            );
                        widget::row![btn_s2t, btn_t2s].spacing(space_s).into()
                    }
                    None => widget::text("Select a tab").into(),
                };

                let mut col = widget::column![header, tabs, tab_content].spacing(space_s);

                if let Some(feedback) = &self.post_prod_feedback {
                    col = col.push(widget::text(feedback).class(cosmic::theme::Text::Accent));
                }

                col.into()
            }
        };

        widget::container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .apply(widget::container)
            .width(Length::Fill)
            .padding([0, 50])
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

        if self.subtitle.search_active {
            if let Some(path) = &self.subtitle.search_path {
                let key = SubtitleSearchKey {
                    generation: self.subtitle.search_gen,
                    path: path.clone(),
                };
                let sel = self.subtitle.search_selection;
                let fps = self.video_frame_rate;
                subscriptions.push(Subscription::run_with(
                    (key, FakeHashable(sel), FakeHashable(fps)),
                    move |(k, sel, fps)| subtitle_search_stream(k.path.clone(), sel.0, fps.0),
                ));
            }
        }

        Subscription::batch(subscriptions)
    }

    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        let needs_recompute = Self::scaled_selection_needs_recomputation(&message);

        match message {
            Message::WatchTick(time) => {
                self.time = time;
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

            Message::ResetSelection => {
                self.screenshot_selection = None;
                self.canvas_generation = self.canvas_generation.wrapping_add(1);
            }
            Message::ScreenshotRegion(msg) => {
                self.screenshot_selection = msg;
            }
            Message::CanvasSize(rectangle) => {
                self.canvas_dimensions = rectangle;
            }

            Message::PickVideo => {
                return Task::perform(
                    async {
                        let file = AsyncFileDialog::new()
                            .add_filter(
                                "Video",
                                &["mkv", "mp4", "avi", "mov", "webm", "flv", "wmv"],
                            )
                            .pick_file()
                            .await;
                        file.map(|f| f.path().to_path_buf())
                    },
                    Message::VideoFilePicked,
                )
                .map(Into::into);
            }

            Message::VideoFilePicked(Some(path)) => {
                return self.update(Message::LoadVideo(path));
            }
            Message::VideoFilePicked(None) => {}

            Message::LoadVideo(path) => {
                match ffmpeg::format::input(&path) {
                    Ok(input) => match video_player::create_video_player::<false>(input, None) {
                        Ok((controller, _iter)) => {
                            controller.seek_forward(Duration::from_mins(6)).unwrap();
                            self.video_path = Some(path);
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

                let (width, height) = (frame.width(), frame.height());

                let handle = widget::image::Handle::from_rgba(
                    frame.width(),
                    frame.height(),
                    frame.into_raw(),
                );

                // Spawn the allocation Task
                return iced::runtime::image::allocate(&handle)
                    .map(move |result| {
                        Message::VideoFrameAllocated(
                            result
                                .ok()
                                .map(|x| (x, iced::Size::new(width as f32, height as f32))),
                        )
                    })
                    .map(Into::into);
            }

            Message::VideoFrameAllocated(allocation_opt) => {
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
            Message::StartSubtitleDisplay(path) => {
                self.subtitle.search_active = true;
                self.subtitle.search_gen += 1;
                self.subtitle.search_path = Some(path);
                self.subtitle.search_selection = self.screenshot_selection_scaled; // snapshot current selection
                self.subtitle.results.clear();
                self.subtitle.preview = None;
                self.subtitle.current_frame = 0;
                self.subtitle.done = false;
                self.nav.activate(self.subtitle_page_id);
                return self.update_title();
            }
            Message::SubtitleProgress { frame, preview } => {
                self.subtitle.current_frame = frame;
                self.subtitle.preview = Some(widget::image::Handle::from_rgba(
                    preview.width(),
                    preview.height(),
                    preview.into_raw(),
                ));
            }

            Message::SubtitleEventFound {
                start_timestamp: start_frame,
                end_timestamp: end_frame,
                text,
                preview,
            } => {
                self.subtitle.results.push(SubtitleResult {
                    start_timestamp: start_frame,
                    end_timestamp: end_frame,
                    text,
                    preview: widget::image::Handle::from_rgba(
                        preview.width(),
                        preview.height(),
                        preview.into_raw(),
                    ),
                });
            }

            Message::SubtitleSearchDone => {
                self.subtitle.search_active = false;
                self.subtitle.done = true;
                self.subtitle.preview = None; // clear the scan preview when finished
            }

            Message::SubtitleSearchError(e) => {
                eprintln!("subtitle search error: {e}");
                self.subtitle.search_active = false;
            }
            Message::GoToPostProduction => {
                self.post_prod_feedback = None;
                self.nav.activate(self.post_production_page_id);
                return self.update_title();
            }
            Message::SelectPostProdTab(id) => {
                self.post_prod_tabs.activate(id);
                self.post_prod_feedback = None;
            }
            Message::MergeWithVideo => {
                let srt = self.subtitle.to_srt();
                let temp_srt = std::env::temp_dir().join("temp_subs.srt");

                if std::fs::write(&temp_srt, srt).is_err() {
                    self.post_prod_feedback =
                        Some("Failed to create temporary subtitle file for merge.".into());
                    std::fs::remove_file(temp_srt).ok();

                    return Task::none();
                }

                let Some(video) = &self.video_path else {
                    self.post_prod_feedback = Some("No video loaded to merge with.".into());
                    std::fs::remove_file(temp_srt).ok();

                    return Task::none();
                };

                let stem = video.file_stem().unwrap_or_default().to_string_lossy();
                // MKV is broadly compatible and safe for merging soft-subtitles
                let output = video.with_file_name(format!("{stem}_merged.mkv"));

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
                dbg!(ffmpeg.print_command());
                if let Ok(mut x) = ffmpeg.spawn() {
                    let mut s = String::new();
                    x.take_stderr().unwrap().read_to_string(&mut s);
                    dbg!(s);
                }

                self.post_prod_feedback =
                    Some(format!("Merged video saved successfully to {:?}", output));
            }
            Message::OpenCcS2T => {
                // Relies on the `opencc` rust wrapper to be available
                let cc = opencc::OpenCC::new("s2t.json");
                for res in &mut self.subtitle.results {
                    dbg!(&res.text);
                    res.text = cc.convert(&res.text);
                    dbg!(&res.text);
                }
                self.post_prod_feedback =
                    Some("Subtitles Converted to Traditional Chinese (S2T).".into());
            }
            Message::OpenCcT2S => {
                let cc = opencc::OpenCC::new("t2s.json");
                for res in &mut self.subtitle.results {
                    res.text = cc.convert(&res.text);
                }
                self.post_prod_feedback =
                    Some("Subtitles Converted to Simplified Chinese (T2S).".into());
            }
            Message::ConvertToSrt => {
                self.post_prod_feedback = None;
                let srt = self.subtitle.to_srt();
                return Task::perform(
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
                .map(Into::into);
            }
            Message::SrtSaved(path_opt) => {
                if let Some(p) = path_opt {
                    self.post_prod_feedback = Some(format!("Successfully saved SRT to {:?}", p));
                } else {
                    self.post_prod_feedback = Some("File save cancelled.".into());
                }
            }
        }
        if needs_recompute {
            self.recompute_scaled_selection();
        }
        Task::none()
    }

    fn on_nav_select(&mut self, id: nav_bar::Id) -> Task<cosmic::Action<Self::Message>> {
        self.nav.activate(id);
        self.update_title()
    }
}

struct FakeHashable<T>(T);

impl<T> std::hash::Hash for FakeHashable<T> {
    fn hash<H: std::hash::Hasher>(&self, _state: &mut H) {}
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
    // tracks [Message::VideoFrameAllocated, Message::CanvasSize, Message::ScreenshotRegion]
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
            Message::VideoFrameAllocated(_)
                | Message::CanvasSize(_)
                | Message::ScreenshotRegion(_)
                | Message::ResetSelection
        )
    }
}

pub enum Page {
    Prepare,
    Subtitle,
    PostProduction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PostProdTab {
    Srt,
    Merge,
    OpenCc,
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

#[derive(Clone, Debug)]
pub struct SubtitleResult {
    pub start_timestamp: Duration,
    pub end_timestamp: Duration,
    pub text: String,
    pub preview: widget::image::Handle,
}

// Wraps the frame iterator to tee progress frames into a channel at ~5fps.
struct ProgressIter<I> {
    inner: I,
    tx: tokio::sync::mpsc::Sender<Message>,
    count: usize,
    interval: usize,
}

impl<I: Iterator<Item = VideoFrame>> Iterator for ProgressIter<I> {
    type Item = VideoFrame;

    fn next(&mut self) -> Option<Self::Item> {
        let mat = self.inner.next()?;
        self.count += 1;
        if self.count.is_multiple_of(self.interval) {
            if let Ok(rgba) = mat_to_image_handle(&mat.mat) {
                // try_send: progress frames are display-only, drop them if the
                // channel is full rather than stalling the search thread.
                let _ = self.tx.try_send(Message::SubtitleProgress {
                    frame: self.count,
                    preview: rgba,
                });
            }
        }
        Some(mat)
    }
}

// Subscription key — gen changes each time a new search starts,
// which causes iced to tear down the old subscription.
#[derive(Clone, Hash, PartialEq, Eq)]
struct SubtitleSearchKey {
    generation: usize,
    path: std::path::PathBuf,
}

fn subtitle_search_stream(
    path: std::path::PathBuf,
    selection: Option<iced::Rectangle>,
    frame_rate: f64,
) -> impl futures::Stream<Item = Message> + Send {
    // One progress frame every 1/5 s of source footage.
    let preview_interval = (frame_rate / 5.0).ceil().max(1.0) as usize;

    iced::stream::channel(
        8,
        async move |mut tx: futures::channel::mpsc::Sender<Message>| {
            let (btx, mut brx) = tokio::sync::mpsc::channel::<Message>(8);

            tokio::task::spawn_blocking(move || {
                let input = match ffmpeg::format::input(&path) {
                    Ok(x) => x,
                    Err(e) => {
                        let _ = btx.blocking_send(Message::SubtitleSearchError(e.to_string()));
                        return;
                    }
                };
                let (_, iter) = match create_video_player::<false>(input, selection) {
                    Ok(x) => x,
                    Err(e) => {
                        let _ = btx.blocking_send(Message::SubtitleSearchError(e.to_string()));
                        return;
                    }
                };
                let frame_iter = ProgressIter {
                    inner: iter.filter_map(Result::ok),
                    tx: btx.clone(),
                    count: 0,
                    interval: preview_interval,
                };

                let s = SubtitleSearch::new(frame_iter, Params::default());

                for event in s {
                    let Ok(preview) = mat_to_image_handle(&event.sample_bgr) else {
                        continue;
                    };

                    let text = mat_to_dynamic_image(&event.sample_bgr)
                        .and_then(|img| OCR.recognize(&img).map_err(Into::into))
                        .unwrap_or_default()
                        .iter()
                        .map(|x| &x.text)
                        .fold(String::new(), |mut acc, x| {
                            acc.push_str(x);
                            acc.push('\n');
                            acc
                        });

                    let msg = Message::SubtitleEventFound {
                        start_timestamp: event.start_timestamp,
                        end_timestamp: event.end_timestamp,
                        text,
                        preview,
                    };
                    if btx.blocking_send(msg).is_err() {
                        return; // app closed / new search started
                    }
                }

                let _ = btx.blocking_send(Message::SubtitleSearchDone);
            });

            while let Some(msg) = brx.recv().await {
                if tx.send(msg).await.is_err() {
                    break;
                }
            }
        },
    )
}

fn mat_to_dynamic_image(mat: &opencv::core::Mat) -> eyre::Result<DynamicImage> {
    mat_to_image_handle(mat).and_then(|x| {
        RgbaImage::from_raw(x.width(), x.height(), x.into_raw())
            .map(DynamicImage::ImageRgba8)
            .ok_or_else(|| eyre::eyre!("invalid image dimensions"))
    })
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
                        Some(Ok(mat)) => match mat_to_image_handle(&mat.mat) {
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
