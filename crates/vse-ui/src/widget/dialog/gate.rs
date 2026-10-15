//! Mutes a closing dialog's content. `opaque` only shields the page below;
//! the content itself still receives events and would emit messages.

use iced::advanced::shell::Bus;
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Layout, Shell, Widget, layout, mouse, overlay, renderer};
use iced::{Element, Event, Length, Rectangle, Size, Theme, Vector};

/// Passes events to `content` but drops its messages and overlays while closed.
/// The tree is the content's own, so toggling the gate keeps child state.
pub(super) fn gate<'a, Message: 'a, Renderer: renderer::Renderer + 'a>(
    is_open: bool,
    content: Element<'a, Message, Theme, Renderer>,
) -> Element<'a, Message, Theme, Renderer> {
    Element::new(Gate { is_open, content })
}

struct Gate<'a, Message, Renderer> {
    is_open: bool,
    content: Element<'a, Message, Theme, Renderer>,
}

impl<Message, Renderer: renderer::Renderer> Widget<Message, Theme, Renderer>
    for Gate<'_, Message, Renderer>
{
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
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if self.is_open {
            self.content
                .as_widget_mut()
                .update(tree, event, layout, cursor, renderer, shell, viewport);
            return;
        }

        // Keep capture, redraws, and invalidation; drop only the messages.
        let mut muted = Bus::new();
        let mut local = shell.local(&mut muted);
        self.content
            .as_widget_mut()
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
        // Menus close with the dialog; their state stays in the tree.
        if !self.is_open {
            return Vec::new();
        }
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation, window)
    }
}
