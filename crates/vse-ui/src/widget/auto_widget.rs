//! Lets builders that only assemble other widgets be used as widgets directly.
//!
//! A builder implements [`IntoWidget`] and is exposed as an [`AutoWidget`] of
//! itself. The builder runs once, when Iced first touches the widget, so its
//! builder methods keep working until then and nothing is boxed.

use std::cell::{Cell, OnceCell};

use iced::advanced::widget::{Meta, Operation, Tree, tree};
use iced::advanced::{Layout, Shell, Widget, layout, mouse, overlay, renderer};
use iced::{Event, Length, Rectangle, Size, Vector};

/// A builder that finishes into a widget.
pub trait IntoWidget {
    type Message;
    type Theme;
    type Renderer;
    type Widget: Widget<Self::Message, Self::Theme, Self::Renderer>;

    fn into_widget(self) -> Self::Widget;
}

/// A [`IntoWidget`] builder that acts as the widget it builds.
pub struct AutoWidget<B: IntoWidget> {
    builder: Cell<Option<B>>,
    widget: OnceCell<B::Widget>,
}

impl<B: IntoWidget> AutoWidget<B> {
    pub(crate) fn from_builder(builder: B) -> Self {
        Self {
            builder: Cell::new(Some(builder)),
            widget: OnceCell::new(),
        }
    }

    /// Changes the builder; only valid before the widget is built.
    pub(crate) fn map_builder(self, f: impl FnOnce(B) -> B) -> Self {
        Self::from_builder(f(self.into_builder()))
    }

    /// Takes the builder back; only valid before the widget is built.
    pub(crate) fn into_builder(self) -> B {
        self.builder
            .into_inner()
            .expect("an `AutoWidget` cannot be changed once built")
    }

    fn widget(&self) -> &B::Widget {
        self.widget.get_or_init(|| {
            self.builder
                .take()
                .expect("an `AutoWidget` builder runs once")
                .into_widget()
        })
    }

    fn widget_mut(&mut self) -> &mut B::Widget {
        let _ = self.widget();
        self.widget.get_mut().expect("the widget was just built")
    }
}

impl<B: IntoWidget> Meta for AutoWidget<B> {
    fn is_void(&self) -> bool {
        self.widget().is_void()
    }
}

impl<B> Widget<B::Message, B::Theme, B::Renderer> for AutoWidget<B>
where
    B: IntoWidget,
    B::Renderer: renderer::Renderer,
{
    fn size(&self) -> Size<Length> {
        self.widget().size()
    }

    fn tag(&self) -> tree::Tag {
        self.widget().tag()
    }

    fn state(&self) -> tree::State {
        self.widget().state()
    }

    fn diff(&mut self, tree: &mut Tree) {
        self.widget_mut().diff(tree);
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &B::Renderer, limits: &layout::Limits) {
        self.widget_mut().layout(tree, renderer, limits);
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut B::Renderer,
        theme: &B::Theme,
        style: &renderer::Style,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &B::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.widget_mut()
            .operate(tree, layout, viewport, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &B::Renderer,
        shell: &mut Shell<'_, B::Message>,
        viewport: &Rectangle,
    ) {
        self.widget_mut()
            .update(tree, event, layout, cursor, renderer, shell, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &B::Renderer,
    ) -> mouse::Interaction {
        self.widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout,
        renderer: &B::Renderer,
        viewport: &Rectangle,
        translation: Vector,
        window: Size,
    ) -> Vec<overlay::Element<'b, B::Message, B::Theme, B::Renderer>> {
        self.widget_mut()
            .overlay(tree, layout, renderer, viewport, translation, window)
    }
}
