//! A resizable bottom sheet layered over a background, for compact layouts.
//!
//! The caller owns the [`State`] and feeds resize [`Event`]s back into
//! [`State::update`].

use crate::{Apply, ApplyConditional, Element};
use std::cell::Cell;

use iced::advanced::widget::{Meta, Operation, Tree, tree};
use iced::advanced::{Layout, Shell, Widget, layout, mouse, overlay, renderer};
use iced::widget::{Widget as _, column, container, opaque, space, stack};
use iced::{Color, Fill, Length, Pixels, Rectangle, Renderer, Size, Vector};

use crate::theme::{COSMIC, WithRadius};
use crate::widget::{AutoWidget, IntoWidget, swipe_helper};
use iced::Theme;

/// Height of the grab strip at the top of the sheet; the whole strip is draggable.
const HANDLE_HEIGHT: f32 = 20.0;
/// Default smallest sheet height in pixels: the handle plus room for a title row.
const MINIMUM_HEIGHT: f32 = 120.0;
/// Pixel slack when comparing the sheet against `backdrop_height`, so a drag
/// that lands on it doesn't flicker the backdrop.
const HEIGHT_EPSILON: f32 = 1.0;

/// Lays `foreground` out as a draggable bottom sheet over `background`.
pub fn bottom_sheet<'a, Message, F, B>(
    state: &'a State,
    foreground: F,
    background: B,
) -> BottomSheet<'a, Message, F, B>
where
    Message: 'a,
    F: Widget<Message, Theme, Renderer> + 'a,
    B: Widget<Message, Theme, Renderer> + 'a,
{
    AutoWidget::from_builder(Builder {
        state,
        foreground,
        background,
        on_event: None,
        minimum_height: MINIMUM_HEIGHT,
        backdrop_height: None,
    })
}

pub type BottomSheet<'a, Message, F, B> = AutoWidget<Builder<'a, Message, F, B>>;

pub struct Builder<'a, Message, F, B> {
    state: &'a State,
    foreground: F,
    background: B,
    on_event: Option<Box<dyn Fn(Event) -> Message + 'a>>,
    /// Pixels.
    minimum_height: f32,
    /// Pixels; `None` follows `minimum_height`.
    backdrop_height: Option<f32>,
}

impl<'a, Message, F, B> BottomSheet<'a, Message, F, B>
where
    Message: 'a,
    F: Widget<Message, Theme, Renderer> + 'a,
    B: Widget<Message, Theme, Renderer> + 'a,
{
    /// Makes the sheet draggable; route the [`Event`] to [`State::update`].
    #[must_use]
    pub fn on_event(self, f: impl Fn(Event) -> Message + 'a) -> Self {
        self.map_builder(|mut sheet| {
            sheet.on_event = Some(Box::new(f));
            sheet
        })
    }

    /// Height in pixels the sheet starts at and can't be dragged below. Unless
    /// [`Self::backdrop_height`] is set, the backdrop is off at this height and
    /// the background stays interactive.
    ///
    /// Enforced while dragging; after a window resize the sheet keeps its
    /// pixel height until the next drag.
    #[must_use]
    pub fn minimum_height(self, height: impl Into<Pixels>) -> Self {
        self.map_builder(|mut sheet| {
            sheet.minimum_height = height.into().0;
            sheet
        })
    }

    /// Height in pixels the sheet must be dragged above before the backdrop
    /// appears and the background stops taking input. Defaults to
    /// [`Self::minimum_height`].
    #[must_use]
    pub fn backdrop_height(self, height: impl Into<Pixels>) -> Self {
        self.map_builder(|mut sheet| {
            sheet.backdrop_height = Some(height.into().0);
            sheet
        })
    }
}

/// The sheet's height; owned by the caller.
#[derive(Debug, Default)]
pub struct State {
    /// Sheet height in pixels once dragged.
    sheet_height: Option<f32>,
    /// Last laid-out height of the whole area in pixels, to cap drags.
    area_height: Cell<f32>,
}

impl State {
    pub fn update(
        &mut self,
        Event {
            height,
            minimum_height,
        }: Event,
    ) {
        let area_height = self.area_height.get();
        let maximum_height = if area_height > 0.0 {
            area_height.max(minimum_height)
        } else {
            f32::INFINITY
        };
        self.sheet_height = Some(height.clamp(minimum_height, maximum_height));
    }

    /// Current sheet height in pixels.
    fn sheet_height(&self, minimum_height: f32) -> f32 {
        self.sheet_height.unwrap_or(minimum_height)
    }
}

/// An opaque interaction with the sheet, for [`State::update`].
#[derive(Debug, Clone, Copy)]
pub struct Event {
    /// Requested sheet height in pixels.
    height: f32,
    /// Pixels.
    minimum_height: f32,
}

impl<'a, Message, F, B> IntoWidget for Builder<'a, Message, F, B>
where
    Message: 'a,
    F: Widget<Message, Theme, Renderer> + 'a,
    B: Widget<Message, Theme, Renderer> + 'a,
{
    type Message = Message;
    type Theme = Theme;
    type Renderer = Renderer;
    type Widget = impl Widget<Message, Theme, Renderer> + 'a;

    fn into_widget(self) -> Self::Widget {
        let minimum_height = self.minimum_height;
        let backdrop_height = self.backdrop_height.unwrap_or(minimum_height);
        let sheet_height = self.state.sheet_height(minimum_height);
        let backdrop = sheet_height > backdrop_height + HEIGHT_EPSILON;

        let sheet = column![handle(), self.foreground]
            .apply(container)
            .width(Fill)
            .height(sheet_height)
            // libcosmic's context drawer surface: the primary container,
            // one layer above the background-layer cards beneath.
            .style(|theme| {
                let radius = COSMIC.corner_radii.radius_m[0];
                crate::theme::container::navigation(theme).with_radius(iced::border::top(radius))
            })
            // Keep self clicks from reaching the background beneath.
            .apply(opaque)
            .boxed();

        let overlay = stack![
            backdrop.then_some(
                space()
                    .width(Fill)
                    .height(Fill)
                    .apply(container)
                    .style(|_| container::background(Color::BLACK.scale_alpha(0.55)))
                    .apply(opaque)
            ),
            column![space().height(Fill), sheet]
        ];

        let overlay = MeasureArea {
            content: overlay,
            state: self.state,
        };

        stack![self.background, overlay]
    }
}

/// Grab pill drawn at the top of the sheet.
fn handle<Message>() -> impl Widget<Message, Theme, Renderer> {
    space()
        .width(36)
        .height(4)
        .apply(container)
        .style(|_: &Theme| container::Style {
            background: Some(Color::WHITE.scale_alpha(0.35).into()),
            border: iced::border::rounded(2),
            ..container::Style::default()
        })
        .apply(container)
        .center_x(Fill)
        .center_y(HANDLE_HEIGHT)
}

/// Records the laid-out height of the whole area into [`State`].
struct MeasureArea<'a, W: Meta> {
    content: W,
    state: &'a State,
}

impl<W: Meta> Meta for MeasureArea<'_, W> {}

impl<W, Message> Widget<Message, Theme, Renderer> for MeasureArea<'_, W>
where
    W: Widget<Message, Theme, Renderer>,
{
    fn tag(&self) -> tree::Tag {
        self.content.tag()
    }

    fn state(&self) -> tree::State {
        self.content.state()
    }

    fn diff(&mut self, tree: &mut Tree) {
        self.content.diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.content.size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) {
        self.content.layout(tree, renderer, limits);
        self.state.area_height.set(tree.size.height);
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
        self.content
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
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
            .operate(tree, layout, viewport, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &iced::Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content
            .update(tree, event, layout, cursor, renderer, shell, viewport);
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
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
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
        self.content
            .overlay(tree, layout, renderer, viewport, translation, window)
    }
}
