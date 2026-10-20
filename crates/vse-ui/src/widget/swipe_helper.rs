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

use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Layout, Shell, Widget, layout, mouse, overlay, renderer};
use iced::mouse::ScrollDelta;
use iced::{Element, Event, Length, Rectangle, Size, Vector};

/// Accumulated touchpad distance, in pixels, that counts as one step.
pub const SWIPE_THRESHOLD: f32 = 40.;

pub fn swipe_helper<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> SwipeHelper<'a, Message, Theme, Renderer> {
    SwipeHelper {
        content: content.into(),
        on_next: None,
        on_previous: None,
    }
}

pub struct SwipeHelper<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    on_next: Option<Message>,
    on_previous: Option<Message>,
}

impl<Message, Theme, Renderer> SwipeHelper<'_, Message, Theme, Renderer> {
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

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for SwipeHelper<'_, Message, Theme, Renderer>
where
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
        self.content.as_widget().size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
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
        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            layout,
            viewport,
            renderer,
            operation,
        );
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
        self.content.as_widget_mut().update(
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
                if state.accumulated.abs() < SWIPE_THRESHOLD {
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
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
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
        self.content.as_widget().draw(
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
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
            window,
        )
    }
}

impl<'a, Message, Theme, Renderer> From<SwipeHelper<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(helper: SwipeHelper<'a, Message, Theme, Renderer>) -> Self {
        Element::new(helper)
    }
}
