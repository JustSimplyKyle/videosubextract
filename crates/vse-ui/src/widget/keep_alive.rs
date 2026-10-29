//! Keeps the widget state of keyed content alive while it is unmounted.
//!
//! Iced drops a subtree's [`Tree`] as soon as a different widget takes its
//! place, so scroll offsets, canvas state, editor cursors, etc. reset when
//! switching pages. [`KeepAlive`] parks the outgoing subtree under its key and
//! restores it when that key is shown again.
//!
//! ```ignore
//! widget::keep_alive(self.active_page, page_content)
//! ```
//!
//! Parked subtrees receive no events or operations, so time-based state (e.g.
//! animations) resumes where it stopped.

use std::collections::HashMap;
use std::hash::Hash;

use iced::advanced::widget::{Meta, Operation, Tree, tree};
use iced::advanced::{Layout, Shell, Widget, layout, mouse, overlay, renderer};
use iced::{Event, Length, Rectangle, Size, Vector};

pub fn keep_alive<K, W: Meta>(key: K, content: W) -> KeepAlive<K, W> {
    KeepAlive { key, content }
}

pub struct KeepAlive<K, W: Meta> {
    key: K,
    content: W,
}

struct State<K> {
    active: Option<K>,
    parked: HashMap<K, Tree>,
}

impl<K, W: Meta> Meta for KeepAlive<K, W> {}

impl<K, W, Message, Theme, Renderer> Widget<Message, Theme, Renderer> for KeepAlive<K, W>
where
    K: Clone + Eq + Hash + 'static,
    W: Widget<Message, Theme, Renderer>,
    Renderer: renderer::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State<K>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::<K> {
            active: None,
            parked: HashMap::new(),
        })
    }

    fn diff(&mut self, tree: &mut Tree) {
        let state: &mut State<K> = tree.state.downcast_mut();

        if state.active.as_ref() != Some(&self.key) {
            let restored = state
                .parked
                .remove(&self.key)
                .unwrap_or_else(|| Tree::new(&self.content));
            let outgoing = std::mem::replace(&mut tree.children, vec![restored]);
            if let (Some(previous), Some(outgoing)) = (
                state.active.replace(self.key.clone()),
                outgoing.into_iter().next(),
            ) {
                state.parked.insert(previous, outgoing);
            }
        }

        tree.children[0].diff(&mut self.content);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Stateful leaf whose tree state counts how often it has been diffed.
    struct Counter;

    impl Meta for Counter {}

    impl Widget<(), (), ()> for Counter {
        fn tag(&self) -> tree::Tag {
            tree::Tag::of::<u32>()
        }

        fn state(&self) -> tree::State {
            tree::State::new(0u32)
        }

        fn diff(&mut self, tree: &mut Tree) {
            *tree.state.downcast_mut::<u32>() += 1;
        }

        fn size(&self) -> Size<Length> {
            Size::new(Length::Shrink, Length::Shrink)
        }

        fn layout(&mut self, _tree: &mut Tree, _renderer: &(), _limits: &layout::Limits) {}

        fn draw(
            &self,
            _tree: &Tree,
            _renderer: &mut (),
            _theme: &(),
            _style: &renderer::Style,
            _layout: Layout,
            _cursor: mouse::Cursor,
            _viewport: &Rectangle,
        ) {
        }
    }

    fn diffs(tree: &Tree) -> u32 {
        *tree.children[0].state.downcast_ref::<u32>()
    }

    #[test]
    fn restores_parked_state_by_key() {
        let mut widget = keep_alive(1, Counter);
        let mut tree = Tree::new(&widget);
        tree.diff(&mut widget);
        tree.diff(&mut widget);
        assert_eq!(diffs(&tree), 2);

        tree.diff(&mut keep_alive(2, Counter));
        assert_eq!(diffs(&tree), 1, "a new key starts with fresh state");

        tree.diff(&mut keep_alive(1, Counter));
        assert_eq!(diffs(&tree), 3, "returning to a key restores its state");

        tree.diff(&mut keep_alive(2, Counter));
        assert_eq!(diffs(&tree), 2);
    }
}
