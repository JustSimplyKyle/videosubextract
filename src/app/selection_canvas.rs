use cosmic::iced;
use cosmic::iced::Color;

use cosmic::iced::Point;
use cosmic::iced::core::mouse;
use cosmic::widget::canvas;

use crate::app::Message;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) enum ClickState {
    #[default]
    WaitingFirst,
    WaitingSecond(iced::Point),
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum HandleDrag {
    #[default]
    None,
    Picture,
    TopLeft,
    BottomRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum KeyboardEdge {
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
pub(crate) struct SelectionCanvas {
    pub(crate) last_reset_generation: u32,
    pub(crate) last_bounds: iced::Rectangle,
    pub(crate) click_state: ClickState,
    pub(crate) keyboard_edge: Option<KeyboardEdge>,
    pub(crate) selection: Option<iced::Rectangle>,
    pub(crate) handle_drag: HandleDrag,
    pub(crate) drag_anchor: iced::Point,
    pub(crate) cache: canvas::Cache,
    pub(crate) drag_start: iced::Point,
    pub(crate) previous_selection: iced::Rectangle,
}

#[derive(Default)]
pub(crate) struct SelectionProgram {
    pub(crate) reset_generation: u32,
}

pub(crate) const HANDLE_RADIUS: f32 = 7.0;

pub(crate) const EDGE_HANDLE: f32 = 5.0;

pub(crate) fn hit_handle(point: iced::Point, handle: iced::Point) -> bool {
    (point.x - handle.x).abs() <= HANDLE_RADIUS && (point.y - handle.y).abs() <= HANDLE_RADIUS
}

pub(crate) fn hit_edge(point: iced::Point, bounds: iced::Rectangle, edge: KeyboardEdge) -> bool {
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
