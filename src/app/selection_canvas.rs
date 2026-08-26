use cosmic::iced;
use cosmic::iced::Color;
use cosmic::iced::Point;
use cosmic::iced::core::mouse;
use cosmic::widget::canvas;
use iced::Rectangle;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ClickState {
    #[default]
    WaitingFirst,
    WaitingSecond(iced::Point),
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HandleDrag {
    #[default]
    None,
    Picture,
    TopLeft,
    BottomRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyboardEdge {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

impl KeyboardEdge {
    pub fn get_edge_rectangle(self, bounds: Rectangle, padding: f32) -> Rectangle {
        let horizontal = iced::Size::new(bounds.width, 1.0);
        let vertical = iced::Size::new(1.0, bounds.height);
        match self {
            Self::Top => Rectangle::new(Point::new(bounds.x, bounds.y), horizontal)
                .expand(iced::Padding::default().vertical(padding)),
            Self::Bottom => {
                Rectangle::new(Point::new(bounds.x, bounds.y + bounds.height), horizontal)
                    .expand(iced::Padding::default().vertical(padding))
            }
            Self::Left => Rectangle::new(Point::new(bounds.x, bounds.y), vertical)
                .expand(iced::Padding::default().horizontal(padding)),
            Self::Right => Rectangle::new(Point::new(bounds.x + bounds.width, bounds.y), vertical)
                .expand(iced::Padding::default().horizontal(padding)),
        }
    }
}

#[derive(Default)]
pub struct SelectionCanvas {
    pub last_reset_generation: u32,
    pub last_bounds: Rectangle,
    pub click_state: ClickState,
    pub keyboard_edge: Option<KeyboardEdge>,
    pub selection: Option<Rectangle>,
    pub handle_drag: HandleDrag,
    pub drag_anchor: Point,
    pub cache: canvas::Cache,
    pub drag_start: Point,
    pub previous_selection: Rectangle,
}

pub struct SelectionProgram {
    pub reset_generation: u32,
}

pub const HANDLE_RADIUS: f32 = 7.0;
pub const EDGE_HANDLE: f32 = 5.0;

pub fn hit_handle(point: Point, handle: Point) -> bool {
    (point.x - handle.x).abs() <= HANDLE_RADIUS && (point.y - handle.y).abs() <= HANDLE_RADIUS
}

pub fn hit_edge(point: Point, bounds: Rectangle, edge: KeyboardEdge) -> bool {
    edge.get_edge_rectangle(bounds, EDGE_HANDLE).contains(point)
}

#[derive(Debug, Clone)]
pub enum Message {
    CanvasSize(Rectangle),
    ScreenshotRegion(Option<Rectangle>),
}

/// Pure selection rectangle operations used by both interaction and rendering.
#[derive(Debug, Clone, Copy)]
struct SelectionGeometry(Rectangle);

impl SelectionGeometry {
    fn from_points(first: Point, second: Point) -> Self {
        let x = first.x.min(second.x);
        let y = first.y.min(second.y);
        let width = (first.x - second.x).abs().max(1.0);
        let height = (first.y - second.y).abs().max(1.0);

        Self(Rectangle::new(
            Point::new(x, y),
            iced::Size::new(width, height),
        ))
    }

    fn vertices(self) -> (Point, Point) {
        let rectangle = self.0;
        (
            Point::new(rectangle.x, rectangle.y),
            Point::new(
                rectangle.x + rectangle.width,
                rectangle.y + rectangle.height,
            ),
        )
    }

    fn translated(self, delta: iced::Vector, bounds: Rectangle) -> Self {
        let rectangle = self.0;
        let x = (rectangle.x + delta.x).clamp(0.0, bounds.width - rectangle.width);
        let y = (rectangle.y + delta.y).clamp(0.0, bounds.height - rectangle.height);

        Self(Rectangle::new(Point::new(x, y), rectangle.size()))
    }
}

impl SelectionCanvas {
    fn reset(&mut self, generation: u32) {
        *self = Self {
            last_reset_generation: generation,
            ..Self::default()
        };
        self.cache.clear();
    }

    fn update(
        &mut self,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        reset_generation: u32,
    ) -> Option<canvas::Action<Message>> {
        if reset_generation != self.last_reset_generation {
            self.reset(reset_generation);
            return Some(canvas::Action::request_redraw());
        }

        if bounds != self.last_bounds {
            self.last_bounds = bounds;
            return Some(canvas::Action::publish(Message::CanvasSize(bounds)));
        }

        SelectionInteraction {
            state: self,
            bounds,
            cursor,
        }
        .update(event)
    }
}

struct SelectionInteraction<'a> {
    state: &'a mut SelectionCanvas,
    bounds: Rectangle,
    cursor: mouse::Cursor,
}

impl SelectionInteraction<'_> {
    fn update(&mut self, event: &canvas::Event) -> Option<canvas::Action<Message>> {
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                self.left_pressed()
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                self.cursor_moved(*position)
            }
            canvas::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key,
                repeat,
                modifiers,
                ..
            }) => self.key_pressed(key, *repeat, *modifiers),
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                self.left_released()
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                self.right_pressed()
            }
            _ => None,
        }
    }

    fn left_pressed(&mut self) -> Option<canvas::Action<Message>> {
        let position = self.cursor.position_in(self.bounds)?;

        match self.state.click_state {
            ClickState::Done => self.start_drag(position),
            ClickState::WaitingFirst => {
                self.state.click_state = ClickState::WaitingSecond(position);
                self.state.selection = Some(SelectionGeometry::from_points(position, position).0);
                self.redraw()
            }
            ClickState::WaitingSecond(first) => {
                let selection = SelectionGeometry::from_points(first, position).0;
                self.state.selection = Some(selection);
                self.state.click_state = ClickState::Done;
                self.publish_selection()
            }
        }
    }

    fn start_drag(&mut self, position: Point) -> Option<canvas::Action<Message>> {
        let selection = self.state.selection?;
        let (top_left, bottom_right) = SelectionGeometry(selection).vertices();
        let edges = [
            KeyboardEdge::Top,
            KeyboardEdge::Bottom,
            KeyboardEdge::Left,
            KeyboardEdge::Right,
        ];

        if hit_handle(position, top_left) {
            self.state.handle_drag = HandleDrag::TopLeft;
            self.state.drag_anchor = bottom_right;
        } else if hit_handle(position, bottom_right) {
            self.state.handle_drag = HandleDrag::BottomRight;
            self.state.drag_anchor = top_left;
        } else if let Some(edge) = edges
            .iter()
            .find(|&&edge| hit_edge(position, selection, edge))
        {
            self.state.keyboard_edge = Some(*edge);
        } else if selection.contains(position) {
            self.state.handle_drag = HandleDrag::Picture;
            self.state.drag_start = position;
            self.state.previous_selection = selection;
        } else {
            self.state.keyboard_edge = None;
        }

        self.redraw()
    }

    fn cursor_moved(&mut self, event_position: Point) -> Option<canvas::Action<Message>> {
        if let ClickState::WaitingSecond(first) = self.state.click_state {
            return self.draw_selection(first, event_position);
        }

        let position = self.cursor.position_in(self.bounds)?;
        let selection = self.dragged_selection(position)?;

        self.state.selection = Some(selection);
        self.publish_selection()
    }

    fn draw_selection(
        &mut self,
        first: Point,
        event_position: Point,
    ) -> Option<canvas::Action<Message>> {
        let position = self
            .cursor
            .position_in(self.bounds)
            .unwrap_or(event_position);
        self.state.selection = Some(SelectionGeometry::from_points(first, position).0);
        self.redraw()
    }

    fn dragged_selection(&self, position: Point) -> Option<Rectangle> {
        Some(match self.state.handle_drag {
            HandleDrag::TopLeft | HandleDrag::BottomRight => {
                SelectionGeometry::from_points(self.state.drag_anchor, position).0
            }
            HandleDrag::Picture => {
                let delta = iced::Vector::new(
                    position.x - self.state.drag_start.x,
                    position.y - self.state.drag_start.y,
                );
                SelectionGeometry(self.state.previous_selection)
                    .translated(delta, self.bounds)
                    .0
            }
            HandleDrag::None => return None,
        })
    }

    fn key_pressed(
        &mut self,
        key: &iced::keyboard::Key,
        repeat: bool,
        modifiers: iced::keyboard::Modifiers,
    ) -> Option<canvas::Action<Message>> {
        use iced::keyboard::Modifiers;

        let base_amount = match modifiers {
            Modifiers::CTRL => 5.0,
            Modifiers::SHIFT => 10.0,
            _ => 1.0,
        };
        let amount = if repeat {
            base_amount * 2.5
        } else {
            base_amount
        };
        let selection = self.state.selection?;

        let selection = match self.state.keyboard_edge {
            Some(edge) => Self::resized_selection(selection, key, edge, amount)?,
            None => self.moved_selection(selection, key, amount)?,
        };

        self.state.selection = Some(selection);
        self.publish_selection()
    }

    fn moved_selection(
        &self,
        selection: Rectangle,
        key: &iced::keyboard::Key,
        amount: f32,
    ) -> Option<Rectangle> {
        use iced::keyboard::{Key, key::Named};

        let delta = match key {
            Key::Named(Named::ArrowUp) => iced::Vector::new(0.0, -amount),
            Key::Named(Named::ArrowDown) => iced::Vector::new(0.0, amount),
            Key::Named(Named::ArrowLeft) => iced::Vector::new(-amount, 0.0),
            Key::Named(Named::ArrowRight) => iced::Vector::new(amount, 0.0),
            _ => return None,
        };

        Some(
            SelectionGeometry(selection)
                .translated(delta, self.bounds)
                .0,
        )
    }

    fn resized_selection(
        selection: Rectangle,
        key: &iced::keyboard::Key,
        edge: KeyboardEdge,
        amount: f32,
    ) -> Option<Rectangle> {
        use iced::{
            Padding,
            keyboard::{Key, key::Named},
        };

        let (delta, padding) = match (key, edge) {
            (Key::Named(Named::ArrowUp), KeyboardEdge::Top) => (1, Padding::default().top(amount)),
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

        Some(if delta > 0 {
            selection.expand(padding)
        } else {
            selection.shrink(padding)
        })
    }

    fn left_released(&mut self) -> Option<canvas::Action<Message>> {
        if self.state.handle_drag == HandleDrag::None {
            return None;
        }

        self.state.handle_drag = HandleDrag::None;
        self.publish_selection()
    }

    fn right_pressed(&mut self) -> Option<canvas::Action<Message>> {
        self.state.selection = None;
        self.state.click_state = ClickState::WaitingFirst;
        self.state.handle_drag = HandleDrag::None;
        self.publish_selection()
    }

    fn redraw(&mut self) -> Option<canvas::Action<Message>> {
        self.state.cache.clear();
        Some(canvas::Action::request_redraw())
    }

    fn publish_selection(&mut self) -> Option<canvas::Action<Message>> {
        self.state.cache.clear();
        Some(canvas::Action::publish(Message::ScreenshotRegion(
            self.state.selection,
        )))
    }
}

struct SelectionView<'a> {
    state: &'a SelectionCanvas,
    bounds: Rectangle,
}

struct CanvasBackground {
    bounds: Rectangle,
}

impl CanvasBackground {
    fn draw(self, frame: &mut canvas::Frame<cosmic::Renderer>) {
        frame.fill_rectangle(
            Point::ORIGIN,
            self.bounds.size(),
            Color::from_rgba(0.0, 0.0, 0.0, 0.15),
        );
    }
}

struct SelectionOverlay {
    selection: Rectangle,
    bounds: Rectangle,
}

impl SelectionOverlay {
    fn draw(self, frame: &mut canvas::Frame<cosmic::Renderer>) {
        let selection = self.selection;
        let bounds = self.bounds;
        let dim = Color::from_rgba(0.0, 0.0, 0.0, 0.35);
        let rects = [
            (selection.y > 0.0)
                .then_some((Point::ORIGIN, iced::Size::new(bounds.width, selection.y))),
            (selection.y + selection.height < bounds.height).then_some((
                Point::new(0.0, selection.y + selection.height),
                iced::Size::new(bounds.width, bounds.height - selection.y - selection.height),
            )),
            (selection.x > 0.0).then_some((
                Point::new(0.0, selection.y),
                iced::Size::new(selection.x, selection.height),
            )),
            (selection.x + selection.width < bounds.width).then_some((
                Point::new(selection.x + selection.width, selection.y),
                iced::Size::new(
                    bounds.width - selection.x - selection.width,
                    selection.height,
                ),
            )),
        ];

        for (position, size) in rects.into_iter().flatten() {
            frame.fill_rectangle(position, size, dim);
        }
    }
}

struct SelectionBorder {
    selection: Rectangle,
}

impl SelectionBorder {
    fn color() -> Color {
        Color::from_rgb(1.0, 0.0, 0.0)
    }

    fn draw(self, frame: &mut canvas::Frame<cosmic::Renderer>) {
        frame.stroke_rectangle(
            self.selection.position(),
            self.selection.size(),
            canvas::Stroke::default()
                .with_width(2.0)
                .with_color(Self::color()),
        );
    }
}

struct KeyboardEdgeHighlight {
    selection: Rectangle,
    edge: KeyboardEdge,
}

impl KeyboardEdgeHighlight {
    fn draw(self, frame: &mut canvas::Frame<cosmic::Renderer>) {
        let edge = self.edge.get_edge_rectangle(self.selection, 0.0);
        frame.stroke_rectangle(
            edge.position(),
            edge.size(),
            canvas::Stroke::default()
                .with_width(2.0)
                .with_color(Color::from_rgb(0.0, 1.0, 0.0)),
        );
    }
}

struct SelectionHandles {
    selection: Rectangle,
}

impl SelectionHandles {
    fn draw(self, frame: &mut canvas::Frame<cosmic::Renderer>) {
        let (top_left, bottom_right) = SelectionGeometry(self.selection).vertices();
        let handle_size = iced::Size::new(HANDLE_RADIUS * 2.0, HANDLE_RADIUS * 2.0);

        for handle in [top_left, bottom_right] {
            let position = Point::new(handle.x - HANDLE_RADIUS, handle.y - HANDLE_RADIUS);
            frame.fill_rectangle(position, handle_size, Color::WHITE);
            frame.stroke_rectangle(
                position,
                handle_size,
                canvas::Stroke::default()
                    .with_width(1.5)
                    .with_color(SelectionBorder::color()),
            );
        }
    }
}

impl SelectionView<'_> {
    fn draw(&self, renderer: &cosmic::Renderer) -> Vec<canvas::Geometry> {
        let geometry = self
            .state
            .cache
            .draw(renderer, self.bounds.size(), |frame| {
                if let Some(selection) = self.state.selection {
                    SelectionOverlay {
                        selection,
                        bounds: self.bounds,
                    }
                    .draw(frame);
                    SelectionBorder { selection }.draw(frame);
                    if let Some(edge) = self.state.keyboard_edge {
                        KeyboardEdgeHighlight { selection, edge }.draw(frame);
                    }

                    if matches!(self.state.click_state, ClickState::Done) {
                        SelectionHandles { selection }.draw(frame);
                    }
                } else {
                    CanvasBackground {
                        bounds: self.bounds,
                    }
                    .draw(frame);
                }
            });

        vec![geometry]
    }

    fn mouse_interaction(&self, cursor: mouse::Cursor) -> mouse::Interaction {
        if !cursor.is_over(self.bounds) {
            return mouse::Interaction::default();
        }

        match self.state.click_state {
            ClickState::WaitingFirst | ClickState::WaitingSecond(_) => {
                mouse::Interaction::Crosshair
            }
            ClickState::Done => {
                let Some(selection) = self.state.selection else {
                    return mouse::Interaction::default();
                };
                let Some(position) = cursor.position_in(self.bounds) else {
                    return mouse::Interaction::default();
                };
                let (top_left, bottom_right) = SelectionGeometry(selection).vertices();

                if hit_handle(position, top_left) || hit_handle(position, bottom_right) {
                    mouse::Interaction::Grab
                } else if selection.contains(position) {
                    if self.state.handle_drag == HandleDrag::Picture {
                        mouse::Interaction::Grabbing
                    } else {
                        mouse::Interaction::Move
                    }
                } else {
                    mouse::Interaction::default()
                }
            }
        }
    }
}

impl canvas::Program<Message, cosmic::Theme, cosmic::Renderer> for SelectionProgram {
    type State = SelectionCanvas;

    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        state.update(event, bounds, cursor, self.reset_generation)
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &cosmic::Renderer,
        _theme: &cosmic::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        SelectionView { state, bounds }.draw(renderer)
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        SelectionView { state, bounds }.mouse_interaction(cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_geometry_normalizes_points_and_keeps_a_minimum_size() {
        assert_eq!(
            SelectionGeometry::from_points(Point::new(20.0, 30.0), Point::new(5.0, 10.0)).0,
            Rectangle::new(Point::new(5.0, 10.0), iced::Size::new(15.0, 20.0)),
        );
        assert_eq!(
            SelectionGeometry::from_points(Point::new(5.0, 10.0), Point::new(5.0, 10.0)).0,
            Rectangle::new(Point::new(5.0, 10.0), iced::Size::new(1.0, 1.0)),
        );
    }

    #[test]
    fn selection_geometry_clamps_movement_to_canvas_bounds() {
        let selection = SelectionGeometry(Rectangle::new(
            Point::new(10.0, 20.0),
            iced::Size::new(30.0, 40.0),
        ));
        let bounds = Rectangle::new(Point::ORIGIN, iced::Size::new(100.0, 100.0));

        assert_eq!(
            selection
                .translated(iced::Vector::new(100.0, -100.0), bounds)
                .0,
            Rectangle::new(Point::new(70.0, 0.0), iced::Size::new(30.0, 40.0)),
        );
    }
}
