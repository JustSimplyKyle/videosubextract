//! Turns wheel scrolls and touchpad swipes over its content into discrete
//! "next" / "previous" messages.
//!
//! A wheel notch is one step; touchpad pixel deltas accumulate until they
//! cross [`SWIPE_THRESHOLD`]. Scrolling down or swiping left means next.
//!
//! ```ignore
//! widget::swipe_helper(indicator(model))
//!     .on_next_maybe(model.next_entity())
//!     .on_previous_maybe(model.previous_entity())
//! ```

use iced::advanced::widget::{Meta, Operation, Tree, tree};
use iced::advanced::{Layout, Shell, Widget, layout, mouse, overlay, renderer};
use iced::mouse::ScrollDelta;
use iced::{Event, Length, Rectangle, Size, Vector};

pub const SWIPE_THRESHOLD: f32 = 40.;

pub fn swipe_helper<Message, W: Meta>(content: W) -> SwipeHelper<Message, W> {
    SwipeHelper {
        content,
        on_next: None,
        on_previous: None,
        threshold: SWIPE_THRESHOLD,
    }
}

pub struct SwipeHelper<Message, W: Meta> {
    content: W,
    threshold: f32,
    on_next: Option<Message>,
    on_previous: Option<Message>,
}

impl<Message, W: Meta> SwipeHelper<Message, W> {
    #[must_use]
    pub fn on_next(self, message: Message) -> Self {
        self.on_next_maybe(Some(message))
    }

    #[must_use]
    pub fn on_previous(self, message: Message) -> Self {
        self.on_previous_maybe(Some(message))
    }

    /// `None` swallows the swipe, e.g. on the last page.
    #[must_use]
    pub fn on_next_maybe(mut self, message: Option<Message>) -> Self {
        self.on_next = message;
        self
    }

    /// `None` swallows the swipe, e.g. on the first page.
    #[must_use]
    pub fn on_previous_maybe(mut self, message: Option<Message>) -> Self {
        self.on_previous = message;
        self
    }

    #[must_use]
    pub const fn threshold(mut self, threshold: f32) -> Self {
        self.threshold = threshold;
        self
    }
}

/// Touchpad distance accumulated toward the next step.
#[derive(Default)]
struct State {
    accumulated: f32,
}

/// Uses the dominant axis, since a plain wheel only reports `y`.
fn dominant(x: f32, y: f32) -> f32 {
    if x.abs() > y.abs() { x } else { y }
}

impl<Message, W: Meta> Meta for SwipeHelper<Message, W> {}

impl<Message, W, Theme, Renderer> Widget<Message, Theme, Renderer> for SwipeHelper<Message, W>
where
    W: Widget<Message, Theme, Renderer>,
    Message: Clone,
    Renderer: renderer::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) {
        self.content.layout(&mut tree.children[0], renderer, limits);
        tree.size = tree.children[0].size;
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .operate(&mut tree.children[0], layout, viewport, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            shell,
            viewport,
        );

        if shell.is_event_captured() || !cursor.is_over(layout.bounds()) {
            return;
        }
        let Event::Mouse(mouse::Event::WheelScrolled { delta }) = event else {
            return;
        };

        let state: &mut State = tree.state.downcast_mut();
        let step = match *delta {
            ScrollDelta::Lines { x, y } => {
                state.accumulated = 0.;
                dominant(x, y)
            }
            ScrollDelta::Pixels { x, y } => {
                state.accumulated += dominant(x, y);
                if state.accumulated.abs() < self.threshold {
                    return;
                }
                std::mem::take(&mut state.accumulated)
            }
        };

        let message = if step < 0. {
            self.on_next.as_ref()
        } else if step > 0. {
            self.on_previous.as_ref()
        } else {
            None
        };
        // Left uncaptured at the ends so an outer scrollable still scrolls.
        if let Some(message) = message {
            shell.publish(message.clone());
            shell.capture_event();
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content
            .mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
        window: Size,
    ) -> Vec<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
            window,
        )
    }
}
