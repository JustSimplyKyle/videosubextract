//! Component-owned modal presentation. Keep this component mounted and change
//! `open`; the content factory remains concrete and is called through the exit
//! animation. Only the rendered child is removed after the spring settles.

use crate::{
    Apply,
    motion::{Motion, SpringConfig, frame_action, presets},
};
use iced::advanced::{mouse, renderer};
use iced::time::Instant;
use iced::widget::{Action, Component, Space, component, container, float, opaque};
use iced::{Color, Element, Event, Fill, Rectangle, Theme, Vector};

/// Build a persistent host from requested visibility and a concrete content
/// factory. Rebuild the host on every application view, including while closed.
pub fn dialog<'a, Message, Renderer: renderer::Renderer>(
    open: bool,
    content: impl Fn() -> Element<'a, Message, Theme, Renderer> + 'a,
) -> Dialog<'a, Message, Renderer> {
    Dialog {
        open,
        content: Box::new(content),
        config: presets::snappy(),
    }
}

/// The parent owns requested visibility; this component owns displayed visibility.
pub struct Dialog<'a, Message, Renderer = iced::Renderer> {
    open: bool,
    content: Box<dyn Fn() -> Element<'a, Message, Theme, Renderer> + 'a>,
    config: SpringConfig,
}

impl<Message, Renderer> Dialog<'_, Message, Renderer> {
    #[must_use]
    pub fn with_config(mut self, config: SpringConfig) -> Self {
        self.config = config;
        self
    }
}

#[doc(hidden)]
#[derive(Default)]
pub struct DialogState {
    motion: Option<Motion>,
}

impl DialogState {
    fn is_present(&self, open: bool) -> bool {
        open || self.motion.as_ref().is_some_and(Motion::is_animating)
    }
}

#[doc(hidden)]
pub enum DialogEvent<Message> {
    Content(Message),
    Frame(Instant),
}

impl<'a, Message: 'static, Renderer: renderer::Renderer + 'a>
    Component<'a, Message, Theme, Renderer> for Dialog<'a, Message, Renderer>
{
    type State = DialogState;
    type Event = DialogEvent<Message>;

    fn diff(&mut self, state: &mut DialogState) {
        let now = Instant::now();
        let motion = state
            .motion
            .get_or_insert_with(|| Motion::new_with_config(0.0, now, self.config));
        motion.set_config(self.config, now);
        motion.retarget(if self.open { 1.0 } else { 0.0 }, now);
    }

    fn listen(
        &self,
        state: &DialogState,
        event: &Event,
        _: Rectangle,
        _: mouse::Cursor,
    ) -> Action<Self::Event> {
        frame_action(
            event,
            state.motion.as_ref().is_some_and(Motion::is_animating),
            DialogEvent::Frame,
        )
    }

    fn update(&self, state: &mut DialogState, event: Self::Event, _: &Renderer) -> Option<Message> {
        match event {
            DialogEvent::Content(message) => self.open.then_some(message),
            DialogEvent::Frame(now) => {
                state.motion.as_mut()?.advance(now);
                None
            }
        }
    }

    fn view(&self, state: &DialogState) -> Element<'a, Self::Event, Theme, Renderer> {
        if !state.is_present(self.open) {
            return Space::new().width(Fill).height(Fill).into();
        }

        let motion = state
            .motion
            .as_ref()
            .expect("diff initializes dialog motion");
        let progress = motion.value();
        let offset = crate::theme::spacing().space_s * (1.0 - progress);

        (self.content)()
            .map(DialogEvent::Content)
            .apply(opaque)
            .apply(float)
            .scale(0.05f32.mul_add(progress, 0.95).max(0.01))
            .translate(move |_, _| Vector::new(0.0, offset))
            .apply(container)
            .center(Fill)
            .style(move |_| container::background(Color::BLACK.scale_alpha(0.55 * progress)))
            .apply(opaque)
    }
}

impl<'a, Message: 'static, Renderer: renderer::Renderer + 'a> From<Dialog<'a, Message, Renderer>>
    for Element<'a, Message, Theme, Renderer>
{
    fn from(dialog: Dialog<'a, Message, Renderer>) -> Self {
        component(dialog)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::advanced::shell::{Bus, Waker};
    use iced::advanced::widget::tree;
    use iced::advanced::{Layout, Shell, Widget, layout, widget::Tree};
    use iced::{Length, Point, Size, keyboard, time::Duration, window};
    use std::{cell::Cell, rc::Rc};

    struct Probe(Rc<Cell<usize>>);

    impl Widget<u8, Theme, ()> for Probe {
        fn tag(&self) -> tree::Tag {
            tree::Tag::of::<Probe>()
        }

        fn state(&self) -> tree::State {
            self.0.set(self.0.get() + 1);
            tree::State::None
        }

        fn size(&self) -> Size<Length> {
            Size::new(100.into(), 80.into())
        }

        fn layout(&mut self, tree: &mut Tree, _: &(), _: &layout::Limits) {
            tree.size = Size::new(100.0, 80.0);
        }

        fn draw(
            &self,
            _: &Tree,
            _: &mut (),
            _: &Theme,
            _: &renderer::Style,
            _: Layout,
            _: mouse::Cursor,
            _: &Rectangle,
        ) {
        }

        fn update(
            &mut self,
            _: &mut Tree,
            event: &Event,
            _: Layout,
            _: mouse::Cursor,
            _: &(),
            shell: &mut Shell<'_, u8>,
            _: &Rectangle,
        ) {
            if matches!(event, Event::Mouse(_) | Event::Keyboard(_)) {
                shell.publish(7);
            }
        }
    }

    fn host(open: bool, mounts: Rc<Cell<usize>>) -> Element<'static, u8, Theme, ()> {
        dialog(open, move || Element::new(Probe(mounts.clone()))).into()
    }

    fn reconcile(control: &mut Element<'_, u8, Theme, ()>, tree: &mut Tree) {
        tree.diff(control.as_widget_mut());
        control.as_widget_mut().layout(
            tree,
            &(),
            &layout::Limits::new(Size::ZERO, Size::new(600.0, 400.0)),
        );
    }

    fn send(
        control: &mut Element<'_, u8, Theme, ()>,
        tree: &mut Tree,
        event: Event,
    ) -> (bool, window::RedrawRequest, Vec<u8>) {
        let mut messages = Bus::new();
        let mut shell = Shell::new(&window::Headless, Waker::new(|| {}), &mut messages);
        control.as_widget_mut().update(
            tree,
            &event,
            Layout::new(tree.size),
            mouse::Cursor::Available(Point::new(300.0, 200.0)),
            &(),
            &mut shell,
            &Rectangle::with_size(Size::new(600.0, 400.0)),
        );
        let captured = shell.is_event_captured();
        let redraw = shell.redraw_request();
        drop(shell);
        (
            captured,
            redraw,
            messages.drain().map(|(message, _)| message).collect(),
        )
    }

    fn frame(now: Instant) -> Event {
        Event::Window(window::Event::RedrawRequested(now))
    }

    fn click() -> Event {
        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
    }

    #[test]
    fn float_overlay_remains_drawable_and_opaque_through_exit() {
        let now = Instant::now();
        let mounts = Rc::new(Cell::new(0));
        let mut control = host(true, mounts.clone());
        let mut tree = Tree::new(control.as_widget());
        reconcile(&mut control, &mut tree);

        for closing in [false, true] {
            if closing {
                send(&mut control, &mut tree, frame(now + Duration::from_secs(5)));
                control = host(false, mounts.clone());
                reconcile(&mut control, &mut tree);
                send(
                    &mut control,
                    &mut tree,
                    frame(now + Duration::from_millis(5050)),
                );
            }
            let layout = Layout::new(tree.size);
            let viewport = Rectangle::with_size(Size::new(600.0, 400.0));
            control.as_widget().draw(
                &tree,
                &mut (),
                &Theme::Dark,
                &renderer::Style::default(),
                layout,
                mouse::Cursor::Unavailable,
                &viewport,
            );
            let mut overlays = control.as_widget_mut().overlay(
                &mut tree,
                layout,
                &(),
                &viewport,
                Vector::ZERO,
                viewport.size(),
            );
            assert_eq!(overlays.len(), 1, "Float supplies the animated panel");
            let overlay = overlays[0].as_overlay_mut();
            overlay.draw(
                &mut (),
                &Theme::Dark,
                &renderer::Style::default(),
                mouse::Cursor::Unavailable,
            );
            let mut messages = Bus::new();
            let mut shell = Shell::new(&window::Headless, Waker::new(|| {}), &mut messages);
            overlay.update(
                &click(),
                mouse::Cursor::Available(Point::new(300.0, 200.0)),
                &(),
                &mut shell,
            );
            assert!(shell.is_event_captured());
            drop(shell);
            let emitted: Vec<_> = messages.drain().map(|(message, _)| message).collect();
            assert_eq!(emitted, if closing { vec![] } else { vec![7] });
        }

        send(
            &mut control,
            &mut tree,
            frame(now + Duration::from_secs(10)),
        );
        let layout = Layout::new(tree.size);
        let viewport = Rectangle::with_size(Size::new(600.0, 400.0));
        assert!(
            control
                .as_widget_mut()
                .overlay(
                    &mut tree,
                    layout,
                    &(),
                    &viewport,
                    Vector::ZERO,
                    viewport.size(),
                )
                .is_empty(),
            "settled exits release the floating overlay"
        );
    }

    #[test]
    fn retains_child_state_through_exit_and_reversal_then_unmounts() {
        let mounts = Rc::new(Cell::new(0));
        let now = Instant::now();
        let mut control = host(false, mounts.clone());
        let mut tree = Tree::new(control.as_widget());
        reconcile(&mut control, &mut tree);
        assert_eq!(mounts.get(), 0, "closed hosts do not build content");

        control = host(true, mounts.clone());
        reconcile(&mut control, &mut tree);
        let initial_mounts = mounts.get();
        assert!(initial_mounts > 0);
        send(&mut control, &mut tree, frame(now + Duration::from_secs(5)));
        assert_eq!(send(&mut control, &mut tree, click()).2, vec![7]);

        control = host(false, mounts.clone());
        reconcile(&mut control, &mut tree);
        assert_eq!(mounts.get(), initial_mounts);
        let (captured, _, messages) = send(&mut control, &mut tree, click());
        assert!(captured, "the closing dialog still shields the page");
        assert!(
            messages.is_empty(),
            "closing content does not emit app messages"
        );
        send(
            &mut control,
            &mut tree,
            frame(now + Duration::from_millis(5050)),
        );

        control = host(true, mounts.clone());
        reconcile(&mut control, &mut tree);
        send(
            &mut control,
            &mut tree,
            frame(now + Duration::from_secs(10)),
        );
        assert_eq!(
            mounts.get(),
            initial_mounts,
            "reversing an exit preserves child state"
        );

        control = host(false, mounts.clone());
        reconcile(&mut control, &mut tree);
        send(
            &mut control,
            &mut tree,
            frame(now + Duration::from_secs(15)),
        );
        let (captured, redraw, messages) = send(&mut control, &mut tree, click());
        assert!(!captured, "hidden hosts let the page receive input");
        assert!(messages.is_empty());
        assert!(matches!(redraw, window::RedrawRequest::Wait));

        control = host(true, mounts.clone());
        reconcile(&mut control, &mut tree);
        assert!(
            mounts.get() > initial_mounts,
            "a completed exit releases the child tree"
        );
    }

    #[test]
    fn opaque_captures_pointer_after_child_dispatch() {
        let now = Instant::now();
        let mut control = host(true, Rc::new(Cell::new(0)));
        let mut tree = Tree::new(control.as_widget());
        reconcile(&mut control, &mut tree);
        assert!(send(&mut control, &mut tree, click()).0);
        send(&mut control, &mut tree, frame(now + Duration::from_secs(5)));
        let keyboard = Event::Keyboard(keyboard::Event::ModifiersChanged(
            keyboard::Modifiers::empty(),
        ));
        let (captured, _, messages) = send(&mut control, &mut tree, click());
        assert!(captured);
        assert_eq!(messages, vec![7], "capture happens after child dispatch");
        let (captured, _, messages) = send(&mut control, &mut tree, keyboard);
        assert!(!captured, "opaque only captures pointer presses");
        assert_eq!(messages, vec![7]);
        let (_, redraw, _) = send(&mut control, &mut tree, frame(now + Duration::from_secs(6)));
        assert!(matches!(redraw, window::RedrawRequest::Wait));
    }

    #[test]
    fn interrupted_motion_preserves_position_and_velocity() {
        let mut control = dialog::<u8, ()>(true, || Space::new().into());
        let mut state = DialogState::default();
        control.diff(&mut state);
        assert_eq!(state.motion.as_ref().unwrap().value(), 0.0);
        control.update(
            &mut state,
            DialogEvent::Frame(Instant::now() + Duration::from_millis(70)),
            &(),
        );
        let motion = state.motion.as_ref().unwrap();
        let position = motion.value();
        let velocity = motion.velocity();
        assert!(position > 0.0 && position < 1.0);
        control.open = false;
        control.diff(&mut state);
        assert_eq!(state.motion.as_ref().unwrap().value(), position);
        assert_eq!(state.motion.as_ref().unwrap().velocity(), velocity);
        assert!(state.is_present(false));
    }
}
