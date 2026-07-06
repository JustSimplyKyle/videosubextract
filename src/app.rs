// SPDX-License-Identifier: MPL-2.0

use crate::config::Config;
use crate::video_player::{self, InnerPlayer};
use crate::{fl, video_player::VideoPlayerController, video_player::VideoPlayerIterator};
use cosmic::app::context_drawer;
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::widget::{Stack, canvas};
use cosmic::iced::{self, Alignment, Color, Length, Point, Subscription, event, futures, mouse};
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
    current_video_frame: Option<RgbaImage>,
    video_texture: Option<widget::image::Handle>,
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
    VideoSeek(Duration),
    VideoError(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum ClickState {
    #[default]
    WaitingFirst,
    WaitingSecond(iced::Point),
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum HandleDrag {
    #[default]
    None,
    Picture,
    TopLeft,
    BottomRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum KeyboardEdge {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

impl KeyboardEdge {
    pub fn get_edge_rectangle(self, bounds: iced::Rectangle, padding: f32) -> iced::Rectangle {
        let horizontal = iced::Size::new(bounds.width, 1.);
        let vertical = iced::Size::new(1., bounds.height);
        match self {
            Self::Top => {
                iced::Rectangle::new(iced::Point::new(bounds.x, bounds.y), horizontal)
                    .expand(iced::Padding::default().vertical(padding)) // was horizontal
            }
            Self::Bottom => {
                iced::Rectangle::new(
                    iced::Point::new(bounds.x, bounds.y + bounds.height),
                    horizontal,
                )
                .expand(iced::Padding::default().vertical(padding)) // was horizontal
            }
            Self::Left => {
                iced::Rectangle::new(iced::Point::new(bounds.x, bounds.y), vertical)
                    .expand(iced::Padding::default().horizontal(padding)) // was vertical
            }
            Self::Right => {
                iced::Rectangle::new(
                    iced::Point::new(bounds.x + bounds.width, bounds.y),
                    vertical,
                )
                .expand(iced::Padding::default().horizontal(padding)) // was vertical
            }
        }
    }
}

#[derive(Default)]
struct SelectionCanvas {
    last_reset_generation: u32,
    last_bounds: iced::Rectangle,
    click_state: ClickState,
    keyboard_edge: Option<KeyboardEdge>,
    selection: Option<iced::Rectangle>,
    handle_drag: HandleDrag,
    drag_anchor: iced::Point,
    cache: canvas::Cache,
    drag_start: iced::Point,
    previous_selection: iced::Rectangle,
}

#[derive(Default)]
struct SelectionProgram {
    reset_generation: u32,
}

const HANDLE_RADIUS: f32 = 7.0;
const EDGE_HANDLE: f32 = 5.0;

fn hit_handle(point: iced::Point, handle: iced::Point) -> bool {
    (point.x - handle.x).abs() <= HANDLE_RADIUS && (point.y - handle.y).abs() <= HANDLE_RADIUS
}
fn hit_edge(point: iced::Point, bounds: iced::Rectangle, edge: KeyboardEdge) -> bool {
    edge.get_edge_rectangle(bounds, EDGE_HANDLE).contains(point)
}

impl canvas::Program<Message, cosmic::Theme, cosmic::Renderer> for SelectionProgram {
    type State = SelectionCanvas;

    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        bounds: iced::Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if self.reset_generation != state.last_reset_generation {
            *state = SelectionCanvas {
                last_reset_generation: self.reset_generation,
                ..SelectionCanvas::default()
            };
            state.cache.clear();
            return Some(canvas::Action::request_redraw());
        }

        if bounds != state.last_bounds {
            state.last_bounds = bounds;
            return Some(canvas::Action::publish(Message::CanvasSize(bounds)));
        }

        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let pos = cursor.position_in(bounds)?;

                match state.click_state {
                    ClickState::Done => {
                        if let Some(sel) = state.selection {
                            let tl = iced::Point::new(sel.x, sel.y);
                            let br = iced::Point::new(sel.x + sel.width, sel.y + sel.height);

                            let edges = [
                                KeyboardEdge::Top,
                                KeyboardEdge::Bottom,
                                KeyboardEdge::Left,
                                KeyboardEdge::Right,
                            ];

                            if hit_handle(pos, tl) {
                                state.handle_drag = HandleDrag::TopLeft;
                                state.drag_anchor = br;
                            } else if hit_handle(pos, br) {
                                state.handle_drag = HandleDrag::BottomRight;
                                state.drag_anchor = tl;
                            } else if let Some(x) = edges
                                .iter()
                                .find(|&&x| hit_edge(pos, state.selection.unwrap_or_default(), x))
                            {
                                state.keyboard_edge = Some(*x);
                            } else if sel.contains(pos) {
                                state.handle_drag = HandleDrag::Picture;
                                state.drag_start = pos;
                                state.previous_selection = sel;
                            } else if !sel.contains(pos) {
                                state.keyboard_edge = None;
                            } else {
                                return None;
                            }
                            state.cache.clear();
                            return Some(canvas::Action::request_redraw());
                        }
                        None
                    }

                    ClickState::WaitingFirst => {
                        state.click_state = ClickState::WaitingSecond(pos);
                        state.selection =
                            Some(iced::Rectangle::new(pos, iced::Size::new(1.0, 1.0)));
                        state.cache.clear();
                        Some(canvas::Action::request_redraw())
                    }

                    ClickState::WaitingSecond(first) => {
                        let x = first.x.min(pos.x);
                        let y = first.y.min(pos.y);
                        let w = (first.x - pos.x).abs().max(1.0);
                        let h = (first.y - pos.y).abs().max(1.0);
                        let rect =
                            iced::Rectangle::new(iced::Point::new(x, y), iced::Size::new(w, h));
                        state.selection = Some(rect);
                        state.click_state = ClickState::Done;
                        state.cache.clear();
                        Some(canvas::Action::publish(Message::ScreenshotRegion(Some(
                            rect,
                        ))))
                    }
                }
            }

            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if let ClickState::WaitingSecond(first) = state.click_state {
                    let pos = cursor.position_in(bounds).unwrap_or(*position);
                    let x = first.x.min(pos.x);
                    let y = first.y.min(pos.y);
                    let w = (first.x - pos.x).abs().max(1.0);
                    let h = (first.y - pos.y).abs().max(1.0);
                    state.selection = Some(iced::Rectangle::new(
                        iced::Point::new(x, y),
                        iced::Size::new(w, h),
                    ));
                    state.cache.clear();
                    return Some(canvas::Action::request_redraw());
                }

                if state.handle_drag != HandleDrag::None {
                    let pos = cursor.position_in(bounds)?;
                    let anchor = state.drag_anchor;

                    let (point, size) = match state.handle_drag {
                        HandleDrag::TopLeft => {
                            let x = pos.x.min(anchor.x);
                            let y = pos.y.min(anchor.y);
                            let width = (anchor.x - pos.x).abs().max(1.0);
                            let height = (anchor.y - pos.y).abs().max(1.0);
                            (iced::Point { x, y }, iced::Size { width, height })
                        }
                        HandleDrag::BottomRight => {
                            let x = pos.x.min(anchor.x);
                            let y = pos.y.min(anchor.y);
                            let width = (pos.x - anchor.x).abs().max(1.0);
                            let height = (anchor.y - pos.y).abs().max(1.0);
                            (iced::Point { x, y }, iced::Size { width, height })
                        }
                        HandleDrag::Picture => {
                            let dx = pos.x - state.drag_start.x;
                            let dy = pos.y - state.drag_start.y;
                            let init = state.previous_selection;
                            let x = (init.x + dx).clamp(0.0, bounds.width - init.width);
                            let y = (init.y + dy).clamp(0.0, bounds.height - init.height);
                            (iced::Point::new(x, y), init.size())
                        }
                        HandleDrag::None => unreachable!(),
                    };

                    state.selection = Some(iced::Rectangle::new(point, size));
                    state.cache.clear();
                    return Some(canvas::Action::publish(Message::ScreenshotRegion(
                        state.selection,
                    )));
                }

                None
            }

            canvas::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key,
                repeat,
                modifiers,
                ..
            }) => {
                use iced::{
                    Padding,
                    keyboard::{Key, Modifiers, key::Named},
                };

                let base_amount = match *modifiers {
                    Modifiers::CTRL => 5.,
                    Modifiers::SHIFT => 10.,
                    _ => 1.,
                };

                let amount = if *repeat {
                    base_amount * 2.5
                } else {
                    base_amount
                };

                let Some(keyboard_edge) = state.keyboard_edge else {
                    let horizontal = iced::Vector::new(1., 0.);
                    let vertical = iced::Vector::new(0., 1.);

                    let selection = state.selection?;

                    let point = Point {
                        x: selection.x,
                        y: selection.y,
                    };
                    let mut point = match key {
                        Key::Named(Named::ArrowUp) => point + vertical * amount * -1.,
                        Key::Named(Named::ArrowDown) => point + vertical * amount,
                        Key::Named(Named::ArrowLeft) => point + horizontal * amount * -1.,
                        Key::Named(Named::ArrowRight) => point + horizontal * amount,
                        _ => {
                            return None;
                        }
                    };
                    point.x = point.x.clamp(0., bounds.width - selection.width);
                    point.y = point.y.clamp(0., bounds.height - selection.height);

                    state.selection = Some(iced::Rectangle::new(point, selection.size()));
                    state.cache.clear();
                    return Some(canvas::Action::publish(Message::ScreenshotRegion(
                        state.selection,
                    )));
                };
                let (delta, padding) = match (key, keyboard_edge) {
                    (Key::Named(Named::ArrowUp), KeyboardEdge::Top) => {
                        (1, Padding::default().top(amount))
                    }
                    (Key::Named(Named::ArrowDown), KeyboardEdge::Top) => {
                        (-1, Padding::default().top(amount))
                    }
                    (Key::Named(Named::ArrowUp), KeyboardEdge::Bottom) => {
                        (-1, Padding::default().bottom(amount))
                    }
                    (Key::Named(Named::ArrowDown), KeyboardEdge::Bottom) => {
                        (1, Padding::default().bottom(amount))
                    }
                    (Key::Named(Named::ArrowLeft), KeyboardEdge::Left) => {
                        (1, Padding::default().left(amount))
                    }
                    (Key::Named(Named::ArrowRight), KeyboardEdge::Left) => {
                        (-1, Padding::default().left(amount))
                    }
                    (Key::Named(Named::ArrowLeft), KeyboardEdge::Right) => {
                        (-1, Padding::default().right(amount))
                    }
                    (Key::Named(Named::ArrowRight), KeyboardEdge::Right) => {
                        (1, Padding::default().right(amount))
                    }
                    _ => return None,
                };

                let s = state.selection.as_mut()?;

                *s = if delta > 0 {
                    s.expand(padding)
                } else {
                    s.shrink(padding)
                };

                state.cache.clear();

                Some(canvas::Action::publish(Message::ScreenshotRegion(
                    state.selection,
                )))
            }

            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if state.handle_drag != HandleDrag::None {
                    state.handle_drag = HandleDrag::None;
                    return Some(canvas::Action::publish(Message::ScreenshotRegion(
                        state.selection,
                    )));
                }
                None
            }

            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                state.selection = None;
                state.click_state = ClickState::WaitingFirst;
                state.handle_drag = HandleDrag::None;
                state.cache.clear();
                Some(canvas::Action::publish(Message::ScreenshotRegion(None)))
            }

            _ => None,
        }
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &cosmic::Renderer,
        _theme: &cosmic::Theme,
        bounds: iced::Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let geometry = state.cache.draw(renderer, bounds.size(), |frame| {
            if let Some(selection) = state.selection {
                let dim = iced::Color::from_rgba(0.0, 0.0, 0.0, 0.35);
                let rects = [
                    (selection.y > 0.0).then_some((
                        iced::Point::ORIGIN,
                        iced::Size::new(bounds.width, selection.y),
                    )),
                    (selection.y + selection.height < bounds.height).then_some((
                        iced::Point::new(0.0, selection.y + selection.height),
                        iced::Size::new(
                            bounds.width,
                            bounds.height - selection.y - selection.height,
                        ),
                    )),
                    (selection.x > 0.0).then_some((
                        iced::Point::new(0.0, selection.y),
                        iced::Size::new(selection.x, selection.height),
                    )),
                    (selection.x + selection.width < bounds.width).then_some((
                        iced::Point::new(selection.x + selection.width, selection.y),
                        iced::Size::new(
                            bounds.width - selection.x - selection.width,
                            selection.height,
                        ),
                    )),
                ];
                for rect in rects.into_iter().flatten() {
                    frame.fill_rectangle(rect.0, rect.1, dim);
                }

                let border_color = Color::from_rgb(1.0, 0.0, 0.0);
                frame.stroke_rectangle(
                    selection.position(),
                    selection.size(),
                    canvas::Stroke::default()
                        .with_width(2.0)
                        .with_color(border_color),
                );

                if let Some(edge) = state
                    .keyboard_edge
                    .map(|x| x.get_edge_rectangle(selection, 0.))
                {
                    frame.stroke_rectangle(
                        Point {
                            x: edge.x,
                            y: edge.y,
                        },
                        edge.size(),
                        canvas::Stroke::default()
                            .with_width(2.0)
                            .with_color(Color::from_rgb(0.0, 1.0, 0.0)),
                    );
                }

                if matches!(state.click_state, ClickState::Done) {
                    let handle_size = HANDLE_RADIUS * 2.0;
                    let half = HANDLE_RADIUS;
                    let handle_color = iced::Color::WHITE;

                    let tl = iced::Point::new(selection.x - half, selection.y - half);
                    let br = iced::Point::new(
                        selection.x + selection.width - half,
                        selection.y + selection.height - half,
                    );

                    for corner in [tl, br] {
                        let size = iced::Size::new(handle_size, handle_size);
                        frame.fill_rectangle(corner, size, handle_color);
                        frame.stroke_rectangle(
                            corner,
                            size,
                            canvas::Stroke::default()
                                .with_width(1.5)
                                .with_color(border_color),
                        );
                    }
                }
            } else {
                frame.fill_rectangle(
                    iced::Point::ORIGIN,
                    bounds.size(),
                    iced::Color::from_rgba(0.0, 0.0, 0.0, 0.15),
                );
            }
        });

        vec![geometry]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: iced::Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if !cursor.is_over(bounds) {
            return mouse::Interaction::default();
        }

        match state.click_state {
            ClickState::WaitingFirst | ClickState::WaitingSecond(_) => {
                mouse::Interaction::Crosshair
            }
            ClickState::Done => {
                let Some(sel) = state.selection else {
                    return mouse::Interaction::default();
                };
                let Some(pos) = cursor.position_in(bounds) else {
                    return mouse::Interaction::default();
                };
                let tl = iced::Point::new(sel.x, sel.y);
                let br = iced::Point::new(sel.x + sel.width, sel.y + sel.height);
                if hit_handle(pos, tl) || hit_handle(pos, br) {
                    return mouse::Interaction::Grab;
                } else if sel.contains(pos) {
                    return if state.handle_drag == HandleDrag::Picture {
                        mouse::Interaction::Grabbing
                    } else {
                        mouse::Interaction::Move
                    };
                }
                mouse::Interaction::default()
            }
        }
    }
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
            current_video_frame: None,
            video_texture: None,
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

                let full_img_handle = self.video_texture.as_ref().map_or_else(
                    || widget::image::Handle::from_path("test-2.png"),
                    |texture| texture.clone(),
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

                let canvas_widget = widget::canvas(SelectionProgram {
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

                    full_img.crop(iced::Rectangle {
                        x: x as u32,
                        y: y as u32,
                        width: w as u32,
                        height: h as u32,
                    })
                });

                let full_img = widget::image(full_img_handle);
                let full_img = Stack::new().push(full_img).push(canvas_widget);

                let reset_btn =
                    widget::button::text("Reset Selection").on_press(Message::ResetSelection);

                let load_video = widget::button::text("Load Video")
                    .on_press(Message::LoadVideo("subtitle2.mkv".into()));

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
                        widget::text(selection_label)
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
                            // _iter is intentionally dropped; the subscription creates its own
                            // from the shared Arc<InnerPlayer> inside the controller.
                            self.video_controller = Some(controller);
                            self.current_video_frame = None;
                        }
                        Err(e) => eprintln!("video_player init error: {e}"),
                    },
                    Err(e) => eprintln!("ffmpeg open error: {e}"),
                }
            }

            Message::VideoFrame(frame) => {
                println!("hi");

                self.video_texture = Some(widget::image::Handle::from_rgba(
                    frame.width(),
                    frame.height(),
                    frame.to_vec(),
                ));
                self.current_video_frame = Some(frame);
            }

            Message::VideoSeek(duration) => {
                if let Some(ref controller) = self.video_controller {
                    if let Err(e) = controller.seek(duration) {
                        eprintln!("seek error: {e}");
                    }
                }
            }

            Message::VideoError(msg) => {
                eprintln!("video error: {msg}");
                self.video_controller = None;
                self.current_video_frame = None;
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
    // let frame_dur = Duration::from_secs_f64(1.0 / frame_rate.max(1.0));
    let frame_dur = Duration::from_secs_f64(0.25);

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
                                println!("hihih");
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
