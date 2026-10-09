//! Generic style animation around a native widget.
//!
//! Build the widget first, then wrap it with `animate(widget)`. Input, messages,
//! layout, operations, and overlays remain the responsibility of that widget.
//! Use `Animated::bind` to connect other widgets' style functions directly.

use crate::motion::{AnimationState, SpringConfig, TransitionStyle, presets};
use iced::advanced::{
    Layout, Shell, Widget, layout, mouse, overlay, renderer,
    widget::{Operation, Tree, tree},
};
use iced::time::Instant;
use iced::{Element, Event, Length, Rectangle, Size, Vector, window};
use std::{cell::RefCell, rc::Rc};

/// Connects a widget's style function to the generic animation wrapper.
pub trait Animatable {
    type Style: TransitionStyle + 'static;
    type Widget;
    fn into_animated(self) -> Animated<Self::Widget, Self::Style>;
}

pub fn animate<W: Animatable>(widget: W) -> Animated<W::Widget, W::Style> {
    widget.into_animated()
}

/// A native widget plus its style animation and redraw scheduling.
pub struct Animated<W, S> {
    inner: W,
    binding: StyleBinding<S>,
    config: SpringConfig,
}

impl<W, S> Animated<W, S> {
    /// The inner widget's style function must call this binding's `resolve`.
    pub fn new(inner: W, binding: StyleBinding<S>) -> Self {
        Self {
            inner,
            binding,
            config: presets::snappy(),
        }
    }

    #[must_use]
    pub fn with_config(mut self, config: SpringConfig) -> Self {
        self.config = config;
        self
    }
}

impl<W, S: TransitionStyle> Animated<W, S> {
    /// Builds a widget with a shared style binding and redraw scheduling.
    ///
    /// Resolve the style function's result through the supplied binding. Its
    /// arguments are unrestricted: the widget can pass a theme, status, both,
    /// or any other inputs.
    ///
    /// ```
    /// use vse_ui::{theme, widget::{Animated, dropdown}};
    ///
    /// let control = dropdown(vec!["First", "Second"], Some(0), |index| index);
    /// let animated = Animated::bind(|binding| {
    ///     control.style(move |theme, status| {
    ///         binding.resolve(theme::pick_list::standard(theme, status))
    ///     })
    /// });
    /// ```
    pub fn bind(build: impl FnOnce(StyleBinding<S>) -> W) -> Self {
        let binding = StyleBinding::default();
        let inner = build(binding.clone());
        Self::new(inner, binding)
    }
}

struct WidgetAnimationState<S> {
    animation: AnimationState<S>,
    needs_observation: bool,
}

impl<S> Default for WidgetAnimationState<S> {
    fn default() -> Self {
        Self {
            animation: AnimationState::default(),
            needs_observation: true,
        }
    }
}

impl<S: TransitionStyle> WidgetAnimationState<S> {
    fn update_config(&mut self, config: SpringConfig) {
        self.needs_observation = true;
        self.animation.set_config(config, Instant::now());
    }
    fn update(&mut self, event: &Event) -> bool {
        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            self.animation.advance(*now);
        } else if matches!(
            event,
            Event::Mouse(_) | Event::Touch(_) | Event::Keyboard(_)
        ) {
            // Reserve a following frame for target styles resolved on draw.
            self.needs_observation = true;
        }
        self.needs_observation || self.animation.is_animating()
    }
}

/// Animation state shared by the style function and its widget wrapper.
#[derive(Clone)]
pub struct StyleBinding<S>(Rc<RefCell<WidgetAnimationState<S>>>);

impl<S> Default for StyleBinding<S> {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(WidgetAnimationState::default())))
    }
}

impl<S: TransitionStyle> StyleBinding<S> {
    fn restore_state(&self, persisted: &mut Self) {
        if !Rc::ptr_eq(&self.0, &persisted.0) {
            // A rebuilt style closure already shares this builder's cell.
            // Move the animation into it, then let the tree share it too.
            *self.0.borrow_mut() = std::mem::take(&mut *persisted.0.borrow_mut());
            *persisted = self.clone();
        }
    }

    pub fn resolve(&self, target: S) -> S {
        self.0
            .borrow_mut()
            .animation
            .resolve(target, Instant::now())
    }
}

impl<Message, Theme, Renderer, W, S> Widget<Message, Theme, Renderer> for Animated<W, S>
where
    W: Widget<Message, Theme, Renderer>,
    S: TransitionStyle + 'static,
    Renderer: renderer::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<StyleBinding<S>>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(self.binding.clone())
    }
    fn diff(&mut self, tree: &mut Tree) {
        self.binding
            .restore_state(tree.state.downcast_mut::<StyleBinding<S>>());
        self.binding.0.borrow_mut().update_config(self.config);
        tree.diff_children_custom(
            std::slice::from_mut(&mut self.inner),
            |tree, inner| tree.diff(inner as &mut dyn Widget<Message, Theme, Renderer>),
            |inner| Tree::new(inner as &dyn Widget<Message, Theme, Renderer>),
        );
    }
    fn size(&self) -> Size<Length> {
        self.inner.size()
    }
    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) {
        self.inner.layout(&mut tree.children[0], renderer, limits);
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
        self.inner
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
        self.inner.update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            shell,
            viewport,
        );
        let redraw = tree
            .state
            .downcast_ref::<StyleBinding<S>>()
            .0
            .borrow_mut()
            .update(event);
        // Clipped widgets may never be drawn to observe their pending style.
        if redraw && layout.bounds().intersects(viewport) {
            shell.request_redraw();
        }
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
        self.inner.draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
        // A redraw update can run repeatedly before draw. Only drawing resolves
        // the new style, so keep reserving a frame until it has been observed.
        tree.state
            .downcast_ref::<StyleBinding<S>>()
            .0
            .borrow_mut()
            .needs_observation = false;
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.inner
            .mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
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
        self.inner.overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
            window,
        )
    }
}

impl<'a, Message, Theme, Renderer, W, S> From<Animated<W, S>>
    for Element<'a, Message, Theme, Renderer>
where
    W: Widget<Message, Theme, Renderer> + 'a,
    S: TransitionStyle + 'static,
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(animated: Animated<W, S>) -> Self {
        Element::new(animated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::button::{Button, Style};
    type TestButton<'a, Message> = Animated<iced::widget::Button<'a, Message, Theme, ()>, Style>;
    use iced::advanced::shell::{Bus, Waker};
    use iced::{Color, Point, Theme, time::Duration};

    // A different widget, style, and theme exercise the generic wrapper.
    struct ColorProbe {
        color: Color,
        binding: StyleBinding<Color>,
    }

    impl Animatable for ColorProbe {
        type Style = Color;
        type Widget = Self;
        fn into_animated(self) -> Animated<Self, Color> {
            let binding = self.binding.clone();
            Animated::new(self, binding)
        }
    }

    impl Widget<u8, (), ()> for ColorProbe {
        fn size(&self) -> Size<Length> {
            Size::new(Length::Fixed(20.0), Length::Fixed(20.0))
        }
        fn layout(&mut self, tree: &mut Tree, _: &(), _: &layout::Limits) {
            tree.size = Size::new(20.0, 20.0);
        }
        fn draw(
            &self,
            _: &Tree,
            _: &mut (),
            _: &(),
            _: &renderer::Style,
            _: Layout,
            _: mouse::Cursor,
            _: &Rectangle,
        ) {
            let _ = self.binding.resolve(self.color);
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
            if matches!(
                event,
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            ) {
                shell.publish(4);
            }
        }
    }

    #[test]
    fn generic_wrapper_animates_another_widget_style_and_theme() {
        let mut first = Animated::bind(|binding| ColorProbe {
            color: Color::BLACK,
            binding,
        });
        let mut tree = Tree::new(&first as &dyn Widget<u8, (), ()>);
        first.diff(&mut tree);
        let limits = layout::Limits::new(Size::ZERO, Size::new(100.0, 100.0));
        first.layout(&mut tree, &(), &limits);
        let layout = Layout::new(tree.size);
        let viewport = layout.bounds();
        first.draw(
            &tree,
            &mut (),
            &(),
            &renderer::Style::default(),
            layout,
            mouse::Cursor::Unavailable,
            &viewport,
        );
        let mut next = Animated::bind(|binding| ColorProbe {
            color: Color::WHITE,
            binding,
        });
        tree.diff(&mut next as &mut dyn Widget<u8, (), ()>);
        next.draw(
            &tree,
            &mut (),
            &(),
            &renderer::Style::default(),
            layout,
            mouse::Cursor::Unavailable,
            &viewport,
        );
        assert!(
            tree.state
                .downcast_ref::<StyleBinding<Color>>()
                .0
                .borrow()
                .animation
                .is_animating()
        );

        let mut messages = Bus::new();
        let now = Instant::now();
        for event in [
            release(),
            Event::Window(window::Event::RedrawRequested(now + Duration::from_secs(1))),
        ] {
            let mut shell = Shell::new(&window::Headless, Waker::new(|| {}), &mut messages);
            next.update(
                &mut tree,
                &event,
                layout,
                mouse::Cursor::Unavailable,
                &(),
                &mut shell,
                &viewport,
            );
        }
        assert_eq!(messages.into_iter().collect::<Vec<_>>(), [4]);
        let state = tree.state.downcast_ref::<StyleBinding<Color>>().0.borrow();
        assert_eq!(state.animation.current(), Some(Color::WHITE));
        assert!(!state.animation.is_animating());
    }

    #[test]
    fn repeated_diffs_and_rebuilds_preserve_an_in_flight_trajectory() {
        let mut first = animate(ColorProbe {
            color: Color::WHITE,
            binding: StyleBinding::default(),
        });
        let mut tree = Tree::new(&first as &dyn Widget<u8, (), ()>);
        first.diff(&mut tree);
        let now = Instant::now();
        let mut expected = AnimationState::default();
        {
            let mut state = first.binding.0.borrow_mut();
            for animation in [&mut state.animation, &mut expected] {
                let _ = animation.resolve(Color::BLACK, now);
                let _ = animation.resolve(Color::WHITE, now);
                animation.advance(now + Duration::from_millis(40));
            }
        }
        let displayed = expected.current().unwrap();
        assert_ne!(displayed, Color::BLACK);
        assert_ne!(displayed, Color::WHITE);

        let mut rebuilt = animate(ColorProbe {
            color: Color::WHITE,
            binding: StyleBinding::default(),
        });
        tree.diff(&mut rebuilt as &mut dyn Widget<u8, (), ()>);
        // Re-diffing the same builder must neither borrow its cell twice nor
        // move its state out of that same cell.
        tree.diff(&mut rebuilt as &mut dyn Widget<u8, (), ()>);
        assert_eq!(rebuilt.inner.binding.resolve(Color::WHITE), displayed);

        // Match an uninterrupted animation later in its trajectory. Keeping
        // just the displayed color and restarting its spring would diverge.
        let later = now + Duration::from_millis(90);
        let mut messages = Bus::new();
        let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut messages);
        rebuilt.update(
            &mut tree,
            &Event::Window(window::Event::RedrawRequested(later)),
            Layout::new(Size::new(20.0, 20.0)),
            mouse::Cursor::Unavailable,
            &(),
            &mut shell,
            &Rectangle::with_size(Size::new(100.0, 100.0)),
        );
        expected.advance(later);
        assert_eq!(
            rebuilt.inner.binding.resolve(Color::WHITE),
            expected.current().unwrap(),
        );
    }

    fn layout_button<'a, Message: Clone + 'a>(control: &mut TestButton<'a, Message>) -> Tree {
        let mut tree = Tree::new(control as &dyn Widget<Message, Theme, ()>);
        control.diff(&mut tree);
        control.layout(
            &mut tree,
            &(),
            &layout::Limits::new(Size::ZERO, Size::new(200.0, 200.0)),
        );
        tree
    }

    fn send<'a, Message: Clone + 'a>(
        control: &mut TestButton<'a, Message>,
        tree: &mut Tree,
        event: Event,
        cursor: mouse::Cursor,
        messages: &mut Bus<Message>,
    ) -> (bool, window::RedrawRequest) {
        let mut shell = Shell::new(&window::Headless, Waker::new(|| {}), messages);
        control.update(
            tree,
            &event,
            Layout::new(tree.size),
            cursor,
            &(),
            &mut shell,
            &Rectangle::with_size(Size::new(200.0, 200.0)),
        );
        (shell.is_event_captured(), shell.redraw_request())
    }

    fn draw<'a, Message: Clone + 'a>(control: &TestButton<'a, Message>, tree: &Tree) {
        control.draw(
            tree,
            &mut (),
            &Theme::Dark,
            &renderer::Style::default(),
            Layout::new(tree.size),
            mouse::Cursor::Unavailable,
            &Rectangle::with_size(Size::new(200.0, 200.0)),
        );
    }

    fn inside() -> mouse::Cursor {
        mouse::Cursor::Available(Point::new(5.0, 5.0))
    }
    fn press() -> Event {
        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
    }
    fn release() -> Event {
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
    }

    #[test]
    fn clicks_emit_the_original_message_once_across_view_rebuilds() {
        // Borrowed messages also work: there is no Component::Event 'static bound.
        let message = String::from("original message");
        let mut control = animate(
            Button::<_, ()>::new(iced::widget::Space::new().width(30).height(20))
                .on_press(message.as_str()),
        );
        let mut tree = layout_button(&mut control);
        let mut messages = Bus::new();
        assert!(send(&mut control, &mut tree, press(), inside(), &mut messages).0);
        assert!(messages.is_empty());
        let mut rebuilt = animate(
            Button::<_, ()>::new(iced::widget::Space::new().width(30).height(20))
                .on_press(message.as_str()),
        );
        tree.diff(&mut rebuilt as &mut dyn Widget<_, Theme, ()>);
        assert!(send(&mut rebuilt, &mut tree, release(), inside(), &mut messages).0);
        send(&mut rebuilt, &mut tree, release(), inside(), &mut messages);
        assert_eq!(messages.into_iter().collect::<Vec<_>>(), [message.as_str()]);
    }

    #[test]
    fn canceled_and_disabled_clicks_emit_nothing() {
        let mut control = animate(
            Button::<_, ()>::new(iced::widget::Space::new().width(30).height(20)).on_press(1),
        );
        let mut tree = layout_button(&mut control);
        let mut messages = Bus::new();
        send(&mut control, &mut tree, press(), inside(), &mut messages);
        send(
            &mut control,
            &mut tree,
            release(),
            mouse::Cursor::Unavailable,
            &mut messages,
        );
        send(&mut control, &mut tree, press(), inside(), &mut messages);
        let mut disabled = animate(Button::<_, ()>::new(
            iced::widget::Space::new().width(30).height(20),
        ));
        tree.diff(&mut disabled as &mut dyn Widget<_, Theme, ()>);
        assert!(!send(&mut disabled, &mut tree, release(), inside(), &mut messages).0);
        assert!(messages.is_empty());
    }

    #[test]
    fn clipped_buttons_do_not_schedule_frames_for_unobserved_styles() {
        let mut control = animate(
            Button::<(), ()>::new(iced::widget::Space::new().width(30).height(20)).on_press(()),
        );
        let mut tree = layout_button(&mut control);
        let mut messages = Bus::new();
        let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut messages);
        let layout = Layout::new(tree.size).move_to(Point::new(300.0, 300.0));
        control.update(
            &mut tree,
            &Event::Window(window::Event::RedrawRequested(Instant::now())),
            layout,
            mouse::Cursor::Unavailable,
            &(),
            &mut shell,
            &Rectangle::with_size(Size::new(200.0, 200.0)),
        );
        assert_eq!(shell.redraw_request(), window::RedrawRequest::Wait);
    }

    struct EmitsOnRelease {
        capture: bool,
    }
    impl Widget<u8, Theme, ()> for EmitsOnRelease {
        fn size(&self) -> Size<Length> {
            Size::new(Length::Fixed(30.0), Length::Fixed(20.0))
        }
        fn layout(&mut self, tree: &mut Tree, _: &(), _: &layout::Limits) {
            tree.size = Size::new(30.0, 20.0);
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
            if matches!(
                event,
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            ) {
                shell.publish(7);
                shell.publish(8);
                if self.capture {
                    shell.capture_event();
                }
            }
        }
    }

    #[test]
    fn child_messages_keep_their_order_and_capture_prevents_parent_activation() {
        for capture in [false, true] {
            let mut control =
                animate(Button::<_, ()>::new(Element::new(EmitsOnRelease { capture })).on_press(9));
            let mut tree = layout_button(&mut control);
            let mut messages = Bus::new();
            send(&mut control, &mut tree, press(), inside(), &mut messages);
            send(&mut control, &mut tree, release(), inside(), &mut messages);
            let expected = if capture { vec![7, 8] } else { vec![7, 8, 9] };
            assert_eq!(messages.into_iter().collect::<Vec<_>>(), expected);
        }
    }

    fn appearance_style(color: Color) -> Style {
        Style {
            background: Some(color.into()),
            text_color: color,
            ..Style::default()
        }
    }

    #[test]
    fn holding_a_button_settles_to_its_pressed_style() {
        use iced::widget::button::Status;

        let mut control = animate(
            Button::<(), ()>::new(iced::widget::Space::new().width(30).height(20)).on_press(()),
        );
        let mut tree = layout_button(&mut control);
        let mut messages = Bus::new();
        let now = Instant::now();
        send(
            &mut control,
            &mut tree,
            Event::Window(window::Event::RedrawRequested(now)),
            inside(),
            &mut messages,
        );
        draw(&control, &tree);
        let hovered = crate::theme::button::standard(&Theme::Dark, Status::Hovered);
        let pressed = crate::theme::button::standard(&Theme::Dark, Status::Pressed);
        assert_ne!(hovered.background, pressed.background);
        assert!(send(&mut control, &mut tree, press(), inside(), &mut messages).0);

        for frame in 1..=60 {
            let (_, redraw) = send(
                &mut control,
                &mut tree,
                Event::Window(window::Event::RedrawRequested(
                    now + Duration::from_millis(frame * 16),
                )),
                inside(),
                &mut messages,
            );
            draw(&control, &tree);
            if redraw == window::RedrawRequest::Wait {
                break;
            }
        }
        let state = tree.state.downcast_ref::<StyleBinding<Style>>().0.borrow();
        assert_eq!(state.animation.current(), Some(pressed));
        assert!(!state.animation.is_animating());
        drop(state);
        assert!(messages.is_empty());
        send(&mut control, &mut tree, release(), inside(), &mut messages);
        assert_eq!(messages.into_iter().collect::<Vec<_>>(), [()]);
    }

    struct Segments;

    impl<'a> iced::widget::Component<'a, u8, Theme, ()> for Segments {
        type State = ();
        type Event = u8;

        fn update(&self, _: &mut (), event: u8, _: &()) -> Option<u8> {
            Some(event)
        }

        fn view(&self, _: &()) -> Element<'a, u8, Theme, ()> {
            iced::widget::Row::with_children((0..3).map(|index| {
                Button::<_, ()>::new(iced::widget::Space::new().width(30).height(20))
                    .on_press(index)
                    .padding([6, 14])
                    .style(move |theme, status| {
                        crate::theme::segmented_button(
                            theme,
                            status,
                            index == 1,
                            index == 0,
                            index == 2,
                        )
                    })
                    .into()
            }))
            .spacing(1)
            .into()
        }
    }

    #[test]
    fn holding_a_segment_settles_to_its_pressed_style() {
        use iced::widget::button::Status;

        for index in 0..3 {
            let mut control: Element<'_, u8, Theme, ()> = iced::widget::component(Segments);
            let mut tree = Tree::new(control.as_widget());
            tree.diff(control.as_widget_mut());
            control.as_widget_mut().layout(
                &mut tree,
                &(),
                &layout::Limits::new(Size::ZERO, Size::new(200.0, 200.0)),
            );
            let layout = Layout::new(tree.size).move_to(Point::new(20.0, 40.0));
            let viewport = Rectangle::with_size(Size::new(300.0, 300.0));
            let cursor = mouse::Cursor::Available(Point::new(25.0 + index as f32 * 59.0, 45.0));
            let mut messages = Bus::new();
            let now = Instant::now();
            let mut stopped = false;
            for (frame, event) in
                std::iter::once(Event::Window(window::Event::RedrawRequested(now)))
                    .chain(std::iter::once(press()))
                    .chain((1..=60).map(|frame| {
                        Event::Window(window::Event::RedrawRequested(
                            now + Duration::from_millis(frame * 16),
                        ))
                    }))
                    .enumerate()
            {
                // The runtime can repeat a redraw update before drawing, for
                // example after a component invalidates an overlay.
                let mut redraw = window::RedrawRequest::Wait;
                let updates = if matches!(event, Event::Window(window::Event::RedrawRequested(_))) {
                    2
                } else {
                    1
                };
                for _ in 0..updates {
                    let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut messages);
                    control.as_widget_mut().update(
                        &mut tree,
                        &event,
                        layout,
                        cursor,
                        &(),
                        &mut shell,
                        &viewport,
                    );
                    redraw = shell.redraw_request();
                }
                if matches!(event, Event::Window(window::Event::RedrawRequested(_))) {
                    control.as_widget().draw(
                        &tree,
                        &mut (),
                        &Theme::Dark,
                        &renderer::Style::default(),
                        layout,
                        cursor,
                        &viewport,
                    );
                }
                if frame > 1 && redraw == window::RedrawRequest::Wait {
                    stopped = true;
                    break;
                }
            }
            let state = tree.children[0].children[index]
                .state
                .downcast_ref::<StyleBinding<Style>>()
                .0
                .borrow();
            let pressed = crate::theme::segmented_button(
                &Theme::Dark,
                Status::Pressed,
                index == 1,
                index == 0,
                index == 2,
            );
            assert_eq!(state.animation.current(), Some(pressed));
            assert!(!state.animation.is_animating());
            assert!(stopped);
            drop(state);
            assert!(messages.is_empty());
            for _ in 0..2 {
                let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut messages);
                control.as_widget_mut().update(
                    &mut tree,
                    &release(),
                    layout,
                    cursor,
                    &(),
                    &mut shell,
                    &viewport,
                );
            }
            assert_eq!(messages.into_iter().collect::<Vec<_>>(), [index as u8]);
        }
    }

    #[test]
    fn style_changes_animate_across_rebuilds_and_redraws_stop() {
        let mut control = animate(
            Button::<(), ()>::new(iced::widget::Space::new().width(30).height(20))
                .style(|_, _| appearance_style(Color::BLACK)),
        );
        let mut tree = layout_button(&mut control);
        let mut messages = Bus::new();
        draw(&control, &tree);
        let mut rebuilt = animate(
            Button::<(), ()>::new(iced::widget::Space::new().width(30).height(20))
                .style(|_, _| appearance_style(Color::WHITE)),
        );
        tree.diff(&mut rebuilt as &mut dyn Widget<(), Theme, ()>);
        let now = Instant::now();
        assert_ne!(
            send(
                &mut rebuilt,
                &mut tree,
                Event::Window(window::Event::RedrawRequested(now)),
                mouse::Cursor::Unavailable,
                &mut messages
            )
            .1,
            window::RedrawRequest::Wait
        );
        draw(&rebuilt, &tree);
        assert!(
            tree.state
                .downcast_ref::<StyleBinding<Style>>()
                .0
                .borrow()
                .animation
                .is_animating()
        );
        send(
            &mut rebuilt,
            &mut tree,
            Event::Window(window::Event::RedrawRequested(
                now + Duration::from_millis(40),
            )),
            mouse::Cursor::Unavailable,
            &mut messages,
        );
        draw(&rebuilt, &tree);
        let style = tree
            .state
            .downcast_ref::<StyleBinding<Style>>()
            .0
            .borrow()
            .animation
            .current()
            .unwrap();
        assert!(style.text_color.r > 0.0 && style.text_color.r < 1.0);
        assert_eq!(
            send(
                &mut rebuilt,
                &mut tree,
                Event::Window(window::Event::RedrawRequested(now + Duration::from_secs(1))),
                mouse::Cursor::Unavailable,
                &mut messages
            )
            .1,
            window::RedrawRequest::Wait
        );
        assert!(messages.is_empty());
    }

    #[test]
    fn dropdown_hover_animates_across_rebuilds_and_its_menu_selects_once() {
        use crate::widget::{AnimatedDropdown, dropdown};
        use iced::widget::pick_list::{Status, Style};

        fn build_control() -> AnimatedDropdown<'static, String, Vec<String>, String, usize> {
            AnimatedDropdown::from(
                dropdown(vec!["First", "Second"], Some(0), |index| index).width(120),
            )
        }
        let mut control = build_control();
        let mut tree = Tree::new(&control as &dyn Widget<usize, Theme, ()>);
        Widget::<usize, Theme, ()>::diff(&mut control, &mut tree);
        control.layout(
            &mut tree,
            &(),
            &layout::Limits::new(Size::ZERO, Size::new(200.0, 200.0)),
        );
        let layout = Layout::new(tree.size);
        let viewport = Rectangle::with_size(Size::new(200.0, 200.0));
        let mut messages = Bus::new();
        let now = Instant::now();
        let mut frame = |control: &mut Animated<_, Style>, tree: &mut Tree, time, cursor| {
            let redraw = {
                let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut messages);
                control.update(
                    tree,
                    &Event::Window(window::Event::RedrawRequested(time)),
                    layout,
                    cursor,
                    &(),
                    &mut shell,
                    &viewport,
                );
                shell.redraw_request()
            };
            control.draw(
                tree,
                &mut (),
                &Theme::Dark,
                &renderer::Style::default(),
                layout,
                cursor,
                &viewport,
            );
            redraw
        };
        frame(&mut control, &mut tree, now, mouse::Cursor::Unavailable);
        let active = crate::theme::pick_list::standard(&Theme::Dark, Status::Active);
        let hovered = crate::theme::pick_list::standard(&Theme::Dark, Status::Hovered);
        assert_ne!(active.border.color, hovered.border.color);
        frame(&mut control, &mut tree, now, inside());
        frame(
            &mut control,
            &mut tree,
            now + Duration::from_millis(40),
            inside(),
        );
        let displayed = control.binding.0.borrow().animation.current().unwrap();
        assert_ne!(displayed.border.color, active.border.color);
        assert_ne!(displayed.border.color, hovered.border.color);

        let mut rebuilt = build_control();
        tree.diff(&mut rebuilt as &mut dyn Widget<usize, Theme, ()>);
        assert_eq!(
            rebuilt.binding.0.borrow().animation.current(),
            Some(displayed)
        );
        frame(
            &mut rebuilt,
            &mut tree,
            now + Duration::from_secs(1),
            inside(),
        );
        assert_eq!(
            rebuilt.binding.0.borrow().animation.current(),
            Some(hovered)
        );
        assert_eq!(
            frame(
                &mut rebuilt,
                &mut tree,
                now + Duration::from_secs(2),
                inside()
            ),
            window::RedrawRequest::Wait,
        );

        let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut messages);
        rebuilt.update(
            &mut tree,
            &press(),
            layout,
            inside(),
            &(),
            &mut shell,
            &viewport,
        );
        assert!(shell.is_event_captured());
        // Rebuilding while open preserves the native pick list's menu state.
        let mut rebuilt = build_control();
        tree.diff(&mut rebuilt as &mut dyn Widget<usize, Theme, ()>);
        {
            let mut overlays = rebuilt.overlay(
                &mut tree,
                layout,
                &(),
                &viewport,
                Vector::ZERO,
                viewport.size(),
            );
            assert_eq!(overlays.len(), 1);
            // Each menu row has the same height as the control.
            let second = mouse::Cursor::Available(Point::new(5.0, layout.bounds().height * 2.5));
            let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut messages);
            overlays[0]
                .as_overlay_mut()
                .update(&press(), second, &(), &mut shell);
            assert!(shell.is_event_captured());
        }
        assert!(
            rebuilt
                .overlay(
                    &mut tree,
                    layout,
                    &(),
                    &viewport,
                    Vector::ZERO,
                    viewport.size(),
                )
                .is_empty()
        );
        assert_eq!(messages.into_iter().collect::<Vec<_>>(), [1]);
    }
}
