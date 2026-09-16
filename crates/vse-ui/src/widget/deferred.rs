//! Keep a cold page responsive while its glyphs are prepared over several frames.

use iced::advanced::{Renderer as _, Shell};
use iced::advanced::{Widget, layout, mouse, overlay, renderer, widget};
use iced::{Element, Event, Length, Rectangle, Size, Vector};
use std::{cell::RefCell, time::Duration};

const GLYPH_BUDGET: Duration = Duration::from_millis(2);

pub struct Deferred<'a, Message> {
    content: Element<'a, Message>,
    enabled: bool,
    revision: u64,
}

impl<'a, Message> Deferred<'a, Message> {
    pub fn new(content: impl Into<Element<'a, Message>>, enabled: bool, revision: u64) -> Self {
        Self {
            content: content.into(),
            enabled,
            revision,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Signature {
    bounds: Rectangle,
    scale: f32,
    revision: u64,
}

#[derive(Default)]
struct Gate {
    enabled: bool,
    signature: Option<Signature>,
    required_frame: Option<u64>,
    ready: bool,
    last_draw_was_ready: bool,
}

impl Gate {
    fn enable(&mut self, enabled: bool) {
        if self.enabled != enabled && !self.ready {
            // A frame rendered on another page cannot complete this page's load.
            self.required_frame = None;
        }
        self.enabled = enabled;
    }

    fn observe(&mut self, signature: Signature, frame: u64, pending: bool) -> bool {
        if self.signature != Some(signature) {
            self.signature = Some(signature);
            self.required_frame = None;
            self.ready = false;
            self.last_draw_was_ready = false;
        }
        if self.ready && pending {
            self.ready = false;
            self.last_draw_was_ready = false;
            self.required_frame = None;
        }
        let required = *self.required_frame.get_or_insert(frame.saturating_add(1));
        if frame >= required && !pending {
            self.ready = true;
        }
        self.ready
    }

    fn record_draw(&mut self, ready: bool) {
        self.last_draw_was_ready = ready;
    }

    fn accepts_input(&self) -> bool {
        self.ready && self.last_draw_was_ready
    }
}

impl<Message> Deferred<'_, Message> {
    fn interactive(
        &self,
        tree: &widget::Tree,
        layout: layout::Layout<'_>,
        renderer: &iced::Renderer,
    ) -> bool {
        self.ready(tree, layout, renderer)
            && (!self.enabled
                || tree
                    .state
                    .downcast_ref::<RefCell<Gate>>()
                    .borrow()
                    .accepts_input())
    }

    fn ready(
        &self,
        tree: &widget::Tree,
        layout: layout::Layout<'_>,
        renderer: &iced::Renderer,
    ) -> bool {
        if !self.enabled {
            return true;
        }
        tree.state
            .downcast_ref::<RefCell<Gate>>()
            .borrow_mut()
            .observe(
                Signature {
                    bounds: layout.bounds(),
                    scale: renderer.scale().unwrap_or_default().total(),
                    revision: self.revision,
                },
                renderer.text_preparation_frame(),
                renderer.text_preparation_pending(),
            )
    }
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for Deferred<'_, Message> {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<RefCell<Gate>>()
    }
    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(RefCell::new(Gate {
            enabled: self.enabled,
            ..Gate::default()
        }))
    }
    fn diff(&mut self, tree: &mut widget::Tree) {
        tree.state
            .downcast_mut::<RefCell<Gate>>()
            .get_mut()
            .enable(self.enabled);
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        tree.diff_children(std::slice::from_mut(&mut self.content));
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }
    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: layout::Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if self.interactive(tree, layout, renderer) {
            self.content.as_widget_mut().update(
                &mut tree.children[0],
                event,
                layout,
                cursor,
                renderer,
                shell,
                viewport,
            );
        } else if matches!(
            event,
            Event::Window(iced::window::Event::RedrawRequested(_))
        ) {
            // Scrollables publish their initial viewport and controls settle
            // their visual state on redraw. Let that happen beneath the mask
            // so the first revealed frame includes every visible row.
            self.content.as_widget_mut().update(
                &mut tree.children[0],
                event,
                layout,
                mouse::Cursor::Unavailable,
                renderer,
                shell,
                viewport,
            );
            // Also schedules the final reveal when this frame has no new misses.
            shell.request_redraw();
        } else if cursor.is_over(layout.bounds()) {
            shell.capture_event();
        }
    }
    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: layout::Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        if self.enabled {
            renderer.set_text_prepare_budget(Some(GLYPH_BUDGET));
        }
        let ready = self.ready(tree, layout, renderer);
        if self.enabled {
            // Preparation can complete before this frame is drawn. Input stays
            // blocked until a draw has actually removed the loading mask.
            tree.state
                .downcast_ref::<RefCell<Gate>>()
                .borrow_mut()
                .record_draw(ready);
        }
        // Record the real layout even while covered: these exact physical glyph
        // positions, sizes and clips are what the renderer must prepare.
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            if ready {
                cursor
            } else {
                mouse::Cursor::Unavailable
            },
            viewport,
        );
        if !ready {
            if let Some(bounds) = layout.bounds().intersection(viewport) {
                renderer.with_layer(bounds, |renderer| {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds,
                            ..renderer::Quad::default()
                        },
                        theme.palette().background.base.color,
                    );
                    // Geometry-only placeholders introduce no extra cold text.
                    for (x, y, width, height) in [
                        (0.0, 46.0, 0.43, 32.0),
                        (0.0, 94.0, 0.31, 14.0),
                        (0.0, 132.0, 1.0, 92.0),
                        (0.0, 244.0, 0.4, 220.0),
                        (0.43, 244.0, 0.57, 330.0),
                    ] {
                        let rect = Rectangle {
                            x: bounds.x + bounds.width * x,
                            y: bounds.y + y,
                            width: bounds.width * width,
                            height,
                        };
                        if let Some(rect) = rect.intersection(&bounds) {
                            renderer.fill_quad(
                                renderer::Quad {
                                    bounds: rect,
                                    border: iced::Border {
                                        radius: 8.0.into(),
                                        ..iced::Border::default()
                                    },
                                    ..renderer::Quad::default()
                                },
                                theme.palette().background.weak.color,
                            );
                        }
                    }
                });
            }
        }
    }
    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: layout::Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        if self.interactive(tree, layout, renderer) {
            self.content.as_widget().mouse_interaction(
                &tree.children[0],
                layout,
                cursor,
                viewport,
                renderer,
            )
        } else {
            mouse::Interaction::None
        }
    }
    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: layout::Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        if self.interactive(tree, layout, renderer) {
            self.content.as_widget_mut().operate(
                &mut tree.children[0],
                layout,
                renderer,
                operation,
            );
        }
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: layout::Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        if self.interactive(tree, layout, renderer) {
            self.content.as_widget_mut().overlay(
                &mut tree.children[0],
                layout,
                renderer,
                viewport,
                translation,
            )
        } else {
            None
        }
    }
}

impl<'a, Message: 'a> From<Deferred<'a, Message>> for Element<'a, Message> {
    fn from(value: Deferred<'a, Message>) -> Self {
        Element::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn signature() -> Signature {
        Signature {
            bounds: Rectangle::with_size(Size::new(800.0, 700.0)),
            scale: 2.0,
            revision: 0,
        }
    }
    #[test]
    fn waits_for_its_own_completed_frame() {
        let mut gate = Gate::default();
        gate.enable(true);
        assert!(!gate.observe(signature(), 4, false));
        assert!(!gate.observe(signature(), 5, true));
        assert!(!gate.observe(signature(), 8, true));
        assert!(gate.observe(signature(), 9, false));
    }
    #[test]
    fn unfinished_navigation_cannot_be_completed_by_another_page() {
        let mut gate = Gate::default();
        gate.enable(true);
        assert!(!gate.observe(signature(), 4, false));
        gate.enable(false);
        gate.enable(true);
        assert!(!gate.observe(signature(), 20, false));
        assert!(gate.observe(signature(), 21, false));
    }
    #[test]
    fn completed_preparation_does_not_enable_controls_beneath_the_last_mask() {
        let mut gate = Gate::default();
        gate.enable(true);
        let ready = gate.observe(signature(), 4, false);
        gate.record_draw(ready);
        assert!(!gate.accepts_input());

        // The renderer finishes while the previous masked frame is onscreen.
        assert!(gate.observe(signature(), 5, false));
        assert!(!gate.accepts_input());

        // Repeated event/overlay queries cannot reveal invisible input targets.
        assert!(gate.observe(signature(), 5, false));
        assert!(!gate.accepts_input());
        gate.record_draw(true);
        assert!(gate.accepts_input());
    }

    #[test]
    fn new_pending_work_disables_controls_until_another_visible_draw() {
        let mut gate = Gate::default();
        gate.enable(true);
        assert!(!gate.observe(signature(), 4, false));
        assert!(gate.observe(signature(), 5, false));
        gate.record_draw(true);
        assert!(gate.accepts_input());

        assert!(!gate.observe(signature(), 6, true));
        assert!(!gate.accepts_input());
        assert!(gate.observe(signature(), 7, false));
        assert!(!gate.accepts_input());
        gate.record_draw(true);
        assert!(gate.accepts_input());

        let changed = Signature {
            revision: 1,
            ..signature()
        };
        assert!(!gate.observe(changed, 8, false));
        assert!(!gate.accepts_input());
        assert!(gate.observe(changed, 9, false));
        assert!(!gate.accepts_input());
    }

    #[test]
    fn warm_navigation_reuses_readiness_but_scale_and_content_invalidate_it() {
        let mut gate = Gate::default();
        gate.enable(true);
        assert!(!gate.observe(signature(), 4, false));
        assert!(gate.observe(signature(), 5, false));
        gate.record_draw(true);
        gate.enable(false);
        gate.enable(true);
        assert!(gate.observe(signature(), 20, false));
        assert!(gate.accepts_input());
        let scaled = Signature {
            scale: 1.0,
            ..signature()
        };
        assert!(!gate.observe(scaled, 20, false));
        assert!(gate.observe(scaled, 21, false));
        assert!(!gate.observe(
            Signature {
                revision: 1,
                ..scaled
            },
            21,
            false
        ));
    }
}
