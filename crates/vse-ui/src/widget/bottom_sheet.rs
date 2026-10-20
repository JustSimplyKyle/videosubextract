//! A resizable bottom sheet layered over a background, for compact layouts.
//!
//! Like [`pane_grid`], the caller owns the [`State`] and feeds resize [`Event`]s
//! back into [`State::update`].

use std::cell::Cell;

use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Layout, Shell, Widget, layout, mouse, overlay, renderer};
use iced::widget::pane_grid::{self, ResizeEvent};
use iced::widget::{column, container, opaque, space, stack};
use iced::{Color, Fill, Length, Pixels, Rectangle, Renderer, Size, Vector, touch};

use crate::theme::{COSMIC, WithRadius};
use crate::{Apply, Element, Theme};

/// Height of the grab strip at the top of the sheet; the whole strip is draggable.
const HANDLE_HEIGHT: f32 = 20.0;
/// Default smallest sheet height in pixels: the handle plus room for a title row.
const MINIMUM_HEIGHT: f32 = 120.0;
/// Pixel slack when comparing the laid-out sheet against `backdrop_height`, as
/// `pane_grid` rounds split positions.
const HEIGHT_EPSILON: f32 = 1.0;

/// Lays `foreground` out as a draggable bottom sheet over `background`.
pub fn bottom_sheet<'a, Message: 'a>(
    state: &'a State,
    foreground: impl Into<Element<'a, Message>>,
    background: impl Into<Element<'a, Message>>,
) -> BottomSheet<'a, Message> {
    BottomSheet {
        state,
        foreground: foreground.into(),
        background: background.into(),
        on_event: None,
        minimum_height: MINIMUM_HEIGHT,
        backdrop_height: None,
    }
}

pub struct BottomSheet<'a, Message> {
    state: &'a State,
    foreground: Element<'a, Message>,
    background: Element<'a, Message>,
    on_event: Option<Box<dyn Fn(Event) -> Message + 'a>>,
    /// Pixels.
    minimum_height: f32,
    /// Pixels; `None` follows `minimum_height`.
    backdrop_height: Option<f32>,
}

impl<'a, Message> BottomSheet<'a, Message> {
    /// Makes the sheet draggable; route the [`Event`] to [`State::update`].
    #[must_use]
    pub fn on_event(mut self, f: impl Fn(Event) -> Message + 'a) -> Self {
        self.on_event = Some(Box::new(f));
        self
    }

    /// Height in pixels the sheet starts at and can't be dragged below. Unless
    /// [`Self::backdrop_height`] is set, the backdrop is off at this height and
    /// the background stays interactive.
    ///
    /// Enforced while dragging; after a window resize the sheet keeps its
    /// split ratio until the next drag.
    #[must_use]
    pub fn minimum_height(mut self, height: impl Into<Pixels>) -> Self {
        self.minimum_height = height.into().0;
        self
    }

    /// Height in pixels the sheet must be dragged above before the backdrop
    /// appears and the background stops taking input. Defaults to
    /// [`Self::minimum_height`].
    #[must_use]
    pub fn backdrop_height(mut self, height: impl Into<Pixels>) -> Self {
        self.backdrop_height = Some(height.into().0);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    Backdrop,
    Sheet,
}

/// The sheet's height; owned by the caller.
#[derive(Debug)]
pub struct State {
    panes: pane_grid::State<Pane>,
    /// Whether the sheet has been dragged. Until then it sits at `minimum_height`.
    resized: bool,
    /// Last laid-out height of the whole area in pixels, to convert between
    /// pixel heights and split ratios.
    area_height: Cell<f32>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            panes: pane_grid::State::with_configuration(pane_grid::Configuration::Split {
                axis: pane_grid::Axis::Horizontal,
                // Collapsed; `min_size` holds it at `minimum_height` until dragged.
                ratio: 1.0,
                a: Box::new(pane_grid::Configuration::Pane(Pane::Backdrop)),
                b: Box::new(pane_grid::Configuration::Pane(Pane::Sheet)),
            }),
            resized: false,
            area_height: Cell::new(0.0),
        }
    }
}

impl State {
    pub fn update(
        &mut self,
        Event {
            resize,
            minimum_height,
        }: Event,
    ) {
        let area_height = self.area_height.get();
        // The split ratio is the backdrop's share, so the sheet's pixel minimum
        // caps it from above.
        let split_ratio = if area_height > 0.0 {
            resize
                .ratio
                .min(1.0 - minimum_height / area_height)
                .max(0.0)
        } else {
            resize.ratio
        };
        self.panes.resize(resize.split, split_ratio);
        self.resized = true;
    }

    /// `pane_grid`'s shared per-pane minimum, in pixels. Only used to hold the
    /// sheet at `minimum_height` before the first drag; afterwards it would also
    /// stop the sheet short of the top, so drags are clamped in [`Self::update`].
    fn min_size(&self, minimum_height: f32) -> f32 {
        if self.resized { 0.0 } else { minimum_height }
    }

    /// Current sheet height in pixels, if the area has been laid out.
    fn sheet_height(&self, minimum_height: f32) -> Option<f32> {
        if !self.resized {
            return Some(minimum_height);
        }
        let area_height = self.area_height.get();
        let pane_grid::Node::Split { ratio, .. } = self.panes.layout() else {
            return None;
        };
        (area_height > 0.0).then(|| (1.0 - ratio) * area_height)
    }

    fn backdrop(&self) -> Option<pane_grid::Pane> {
        self.panes
            .iter()
            .find_map(|(id, pane)| (*pane == Pane::Backdrop).then_some(*id))
    }
}

/// An opaque interaction with the sheet, for [`State::update`].
#[derive(Debug, Clone, Copy)]
pub struct Event {
    resize: ResizeEvent,
    /// Pixels.
    minimum_height: f32,
}

impl<'a, Message: 'a> From<BottomSheet<'a, Message>> for Element<'a, Message> {
    fn from(sheet: BottomSheet<'a, Message>) -> Self {
        let minimum_height = sheet.minimum_height;
        let backdrop_height = sheet.backdrop_height.unwrap_or(minimum_height);
        let min_size = sheet.state.min_size(minimum_height);
        let backdrop = sheet
            .state
            .sheet_height(minimum_height)
            .is_some_and(|height| height > backdrop_height + HEIGHT_EPSILON);
        // pane_grid's view is `Fn`, but it is called once per pane.
        let foreground = Cell::new(Some(sheet.foreground));

        let panes = iced::widget::pane_grid(&sheet.state.panes, |_, pane, _| {
            pane_grid::Content::new(match pane {
                Pane::Backdrop if backdrop => space()
                    .width(Fill)
                    .height(Fill)
                    .apply(container)
                    .style(|_| container::background(Color::BLACK.scale_alpha(0.55)))
                    .apply(opaque),
                Pane::Backdrop => space().width(Fill).height(Fill).into(),
                Pane::Sheet => column![handle()]
                    .push(foreground.take())
                    .apply(container)
                    .width(Fill)
                    .height(Fill)
                    // libcosmic's context drawer surface: the primary container,
                    // one layer above the background-layer cards beneath.
                    .style(|theme| {
                        let radius = COSMIC.corner_radii.radius_m[0];
                        crate::theme::container::navigation(theme)
                            .with_radius(iced::border::top(radius))
                    })
                    // Keep sheet clicks from reaching the background beneath.
                    .apply(opaque),
            })
        })
        .width(Fill)
        .height(Fill)
        .spacing(0)
        .min_size(min_size)
        // The handle is the visual; hide pane_grid's own split line.
        .style(|theme| {
            let hidden = pane_grid::Line {
                color: Color::TRANSPARENT,
                width: 0.0,
            };
            pane_grid::Style {
                hovered_split: hidden,
                picked_split: hidden,
                ..pane_grid::default(theme)
            }
        });

        let panes = match sheet.on_event {
            // The grabbable band is `leeway / 2` either side of the split, so
            // this covers the whole handle strip.
            Some(on_event) => panes.on_resize(HANDLE_HEIGHT * 2.0, move |resize| {
                on_event(Event {
                    resize,
                    minimum_height,
                })
            }),
            None => panes,
        };

        let panes = Element::new(PassThrough {
            content: panes.into(),
            state: sheet.state,
            min_size,
            pass_through: !backdrop,
        });

        stack([sheet.background, panes]).into()
    }
}

/// Grab pill drawn at the top of the sheet, over the split's drag area.
fn handle<'a, Message: 'a>() -> Element<'a, Message> {
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
        .into()
}

/// Records the laid-out height into [`State`], and works around `pane_grid`
/// capturing every press inside its bounds, even over an empty pane: with
/// `pass_through`, presses over the backdrop get a levitated cursor so they fall
/// through to the background. Everything else (including drag moves) is passed
/// on unchanged.
struct PassThrough<'a, Message> {
    content: Element<'a, Message>,
    state: &'a State,
    /// Must match the `pane_grid`'s, so the hole lines up with its layout.
    min_size: f32,
    pass_through: bool,
}

impl<Message> PassThrough<'_, Message> {
    /// The backdrop region, minus the upper half of the split's drag band.
    fn hole(&self, bounds: Rectangle) -> Option<Rectangle> {
        let backdrop = self.state.backdrop()?;
        let region = *self
            .state
            .panes
            .layout()
            .pane_regions(0.0, self.min_size, bounds.size())
            .get(&backdrop)?;
        Some(Rectangle {
            height: (region.height - HANDLE_HEIGHT).max(0.0),
            ..region + Vector::new(bounds.x, bounds.y)
        })
    }
}

impl<Message> Widget<Message, Theme, Renderer> for PassThrough<'_, Message> {
    fn tag(&self) -> tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> tree::State {
        self.content.as_widget().state()
    }

    fn diff(&mut self, tree: &mut Tree) {
        self.content.as_widget_mut().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) {
        self.content.as_widget_mut().layout(tree, renderer, limits);
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
            .as_widget()
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
            .as_widget_mut()
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
        let is_press = self.pass_through
            && matches!(
                event,
                iced::Event::Mouse(mouse::Event::ButtonPressed(_))
                    | iced::Event::Touch(touch::Event::FingerPressed { .. })
            );
        let cursor = match self.hole(layout.bounds()) {
            Some(hole) if is_press && cursor.is_over(hole) => cursor.levitate(),
            _ => cursor,
        };
        self.content
            .as_widget_mut()
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
            .as_widget()
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
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation, window)
    }
}
