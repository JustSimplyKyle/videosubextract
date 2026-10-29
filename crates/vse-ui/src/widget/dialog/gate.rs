//! Mutes a closing dialog's content. `opaque` only shields the page below;
//! the content itself still receives events and would emit messages.

use iced::advanced::shell::Bus;
use iced::advanced::widget::{Meta, Operation, Tree, tree};
use iced::advanced::{Layout, Shell, Widget, layout, mouse, overlay, renderer};
use iced::{Event, Length, Rectangle, Size, Theme, Vector};

/// Passes events to `content` but drops its messages and overlays while closed.
/// The tree is the content's own, so toggling the gate keeps child state.
pub(super) fn gate<W: Meta>(is_open: bool, content: W) -> Gate<W> {
    Gate { is_open, content }
}

pub(super) struct Gate<W: Meta> {
    is_open: bool,
    content: W,
}

impl<W: Meta> Meta for Gate<W> {}

impl<W, Message, Renderer> Widget<Message, Theme, Renderer> for Gate<W>
where
    W: Widget<Message, Theme, Renderer>,
    Renderer: renderer::Renderer,
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
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if self.is_open {
            self.content
                .update(tree, event, layout, cursor, renderer, shell, viewport);
            return;
        }

        // Keep capture, redraws, and invalidation; drop only the messages.
        let mut muted = Bus::new();
        let mut local = shell.local(&mut muted);
        self.content
            .update(tree, event, layout, cursor, renderer, &mut local, viewport);
        if local.is_event_captured() {
            shell.capture_event();
        }
        shell.invalidate(local.invalidation());
        shell.request_redraw_at(local.redraw_request());
        shell.request_input_method(local.input_method());
        shell.clipboard_mut().merge(local.clipboard_mut());
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
        // Menus close with the dialog; their state stays in the tree.
        if !self.is_open {
            return Vec::new();
        }
        self.content
            .overlay(tree, layout, renderer, viewport, translation, window)
    }
}
