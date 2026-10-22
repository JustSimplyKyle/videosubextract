//! Generic value animation around a native widget, usually of its style.
//!
//! Build the widget first, then wrap it with `animate(widget)`. Input, messages,
//! layout, operations, and overlays remain the responsibility of that widget.
//! Use `Animated::bind` to connect other widgets' style functions directly.
//!
//! `bind` hands its builder a single-use [`Animate`] that can only wrap a
//! function, so the animation state cannot be shared or fed values from
//! elsewhere. Debug builds also assert that the wrapped function runs exactly
//! once per draw, which catches a wrapper that was never installed.

use crate::motion::{AnimationState, Mix, SpringConfig, presets};
use iced::advanced::{
    Layout, Shell, Widget, layout, mouse, overlay, renderer,
    widget::{Meta, Operation, Tree, tree},
};
use iced::time::Instant;
use iced::{Event, Length, Rectangle, Size, Vector, window};
use std::{cell::RefCell, rc::Rc};

/// Connects a widget's style function to the generic animation wrapper.
pub trait Animatable {
    type Value: Mix + 'static;
    type Widget;
    fn into_animated(self) -> Animated<Self::Widget, Self::Value>;
}

pub fn animate<Control: Animatable>(widget: Control) -> Animated<Control::Widget, Control::Value> {
    widget.into_animated()
}

/// A native widget plus its style animation and redraw scheduling.
pub struct Animated<Inner, Value> {
    inner: Inner,
    binding: Binding<Value>,
    config: SpringConfig,
}

impl<Inner, Value> Animated<Inner, Value> {
    /// The inner widget's style function must call this binding's `resolve`.
    pub(crate) fn new(inner: Inner, binding: Binding<Value>) -> Self {
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

impl<Inner, Value: Mix> Animated<Inner, Value> {
    /// Builds a widget that animates the output of one of its functions,
    /// usually its style function.
    ///
    /// The builder receives an [`Animate`] and must install the function it
    /// returns on the widget:
    ///
    /// ```
    /// use vse_ui::{theme, widget::{Animated, dropdown}};
    ///
    /// let control = dropdown(vec!["First", "Second"], Some(0), |index| index);
    /// let animated = Animated::bind(|animate| {
    ///     control.style(animate.wrap(theme::pick_list::standard))
    /// });
    /// ```
    ///
    /// [`Animate`] is consumed by wrapping, so one binding animates exactly
    /// one function:
    ///
    /// ```compile_fail,E0382
    /// use vse_ui::{theme, widget::{Animated, dropdown}};
    ///
    /// let first = dropdown(vec!["First"], Some(0), |index| index);
    /// let second = dropdown(vec!["Second"], Some(0), |index| index);
    /// let animated = Animated::bind(|animate| {
    ///     let _ = second.style(animate.wrap(theme::pick_list::standard));
    ///     first.style(animate.wrap(theme::pick_list::standard))
    /// });
    /// ```
    pub fn bind(build: impl FnOnce(Animate<Value>) -> Inner) -> Self {
        let binding = Binding::default();
        let inner = build(Animate(binding.clone()));
        Self::new(inner, binding)
    }
}

/// The single-use handle [`Animated::bind`] gives its builder.
pub struct Animate<Value>(Binding<Value>);

impl<Value: Mix> Animate<Value> {
    /// Wraps a two-argument function, such as a `(theme, status)` style
    /// function. Its output becomes the animation target; the wrapped
    /// function returns the displayed value instead.
    pub fn wrap<First: ?Sized, Second>(
        self,
        function: impl Fn(&First, Second) -> Value,
    ) -> impl Fn(&First, Second) -> Value {
        move |first, second| self.0.resolve(function(first, second))
    }

    /// Wraps a one-argument function, such as a theme-only style function.
    pub fn wrap_unary<First: ?Sized>(
        self,
        function: impl Fn(&First) -> Value,
    ) -> impl Fn(&First) -> Value {
        move |first| self.0.resolve(function(first))
    }
}

struct WidgetAnimationState<Value> {
    animation: AnimationState<Value>,
    needs_observation: bool,
    /// The latest frame time. Style functions run during draw, which has no
    /// timestamp, so retargets happen at the frame being drawn.
    now: Instant,
    /// Resolves since the current draw began, for the debug assertion.
    resolves: usize,
}

impl<Value> Default for WidgetAnimationState<Value> {
    fn default() -> Self {
        Self {
            animation: AnimationState::default(),
            needs_observation: true,
            now: Instant::now(),
            resolves: 0,
        }
    }
}

impl<Value: Mix> WidgetAnimationState<Value> {
    fn update_config(&mut self, config: SpringConfig) {
        self.needs_observation = true;
        self.animation.set_config(config, self.now);
    }
    fn update(&mut self, event: &Event) -> bool {
        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            // A repeated or stale redraw must not move the clock backwards.
            self.now = self.now.max(*now);
            self.animation.advance(self.now);
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

/// Animation state shared by the wrapped function and its widget wrapper.
#[derive(Clone)]
pub(crate) struct Binding<Value>(Rc<RefCell<WidgetAnimationState<Value>>>);

impl<Value> Default for Binding<Value> {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(WidgetAnimationState::default())))
    }
}

impl<Value: Mix> Binding<Value> {
    fn restore_state(&self, persisted: &mut Self) {
        if !Rc::ptr_eq(&self.0, &persisted.0) {
            // A rebuilt style closure already shares this builder's cell.
            // Move the animation into it, then let the tree share it too.
            *self.0.borrow_mut() = std::mem::take(&mut *persisted.0.borrow_mut());
            *persisted = self.clone();
        }
    }

    /// Retargets at the latest frame time rather than the wall clock, so a
    /// style change starts exactly at the frame that first displays it.
    pub(crate) fn resolve(&self, target: Value) -> Value {
        let mut state = self.0.borrow_mut();
        state.resolves += 1;
        let now = state.now;
        state.animation.resolve(target, now)
    }
}

impl<Message, Theme, Renderer, Inner, Value> Widget<Message, Theme, Renderer>
    for Animated<Inner, Value>
where
    Inner: Widget<Message, Theme, Renderer>,
    Value: Mix + 'static,
    Renderer: renderer::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Binding<Value>>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(self.binding.clone())
    }
    fn diff(&mut self, tree: &mut Tree) {
        self.binding
            .restore_state(tree.state.downcast_mut::<Binding<Value>>());
        self.binding.0.borrow_mut().update_config(self.config);
        tree.diff_children_custom(std::slice::from_mut(&mut self.inner), Tree::diff, Tree::new);
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
            .downcast_ref::<Binding<Value>>()
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
        self.binding.0.borrow_mut().resolves = 0;
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
        let mut state = self.binding.0.borrow_mut();
        state.needs_observation = false;
        debug_assert_eq!(
            state.resolves, 1,
            "an animated function must run exactly once per draw; \
             install the function from `Animate::wrap` on the wrapped widget",
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

impl<Value, Inner> Meta for Animated<Inner, Value>
where
    Inner: Meta,
    Value: Mix,
{
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::button::{Button, Style};

    type TestButton<'a, Message> =
        Animated<iced::widget::Button<'a, Message, iced::widget::Space, Theme>, Style>;

    /// `Tree::new` for widgets implemented for several renderers.
    fn new_tree<W, Message, Theme, Renderer>(widget: &W) -> Tree
    where
        W: Widget<Message, Theme, Renderer>,
        Renderer: renderer::Renderer,
    {
        Tree::new(widget)
    }

    /// `Tree::diff` for widgets implemented for several renderers.
    fn diff_tree<W, Message, Theme, Renderer>(tree: &mut Tree, widget: &mut W)
    where
        W: Widget<Message, Theme, Renderer>,
        Renderer: renderer::Renderer,
    {
        tree.diff(widget);
    }

    use iced::advanced::shell::{Bus, Waker};
    use iced::{Color, Point, Theme, time::Duration};

    // A different widget, style, and theme exercise the generic wrapper.
    struct ColorProbe {
        color: Color,
        binding: Binding<Color>,
    }

    impl Animatable for ColorProbe {
        type Value = Color;
        type Widget = Self;
        fn into_animated(self) -> Animated<Self, Color> {
            let binding = self.binding.clone();
            Animated::new(self, binding)
        }
    }
    impl Meta for ColorProbe {}

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
    fn style_changes_start_at_the_frame_that_draws_them() {
        let draw = |control: &Animated<ColorProbe, Color>, tree: &Tree| {
            control.draw(
                tree,
                &mut (),
                &(),
                &renderer::Style::default(),
                Layout::new(Size::new(20.0, 20.0)),
                mouse::Cursor::Unavailable,
                &Rectangle::with_size(Size::new(100.0, 100.0)),
            );
        };
        let frame = |control: &mut Animated<ColorProbe, Color>, tree: &mut Tree, now| {
            let mut messages = Bus::new();
            let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut messages);
            control.update(
                tree,
                &Event::Window(window::Event::RedrawRequested(now)),
                Layout::new(Size::new(20.0, 20.0)),
                mouse::Cursor::Unavailable,
                &(),
                &mut shell,
                &Rectangle::with_size(Size::new(100.0, 100.0)),
            );
        };
        let probe = |color| {
            Animated::bind(|animate| ColorProbe {
                color,
                binding: animate.0,
            })
        };

        let mut control = probe(Color::BLACK);
        let mut tree = Tree::new(&control);
        control.diff(&mut tree);
        draw(&control, &tree);

        // Frames run ahead of the wall clock here. Retargeting at the wall
        // clock would start the spring a second early and skip the transition.
        let start = Instant::now() + Duration::from_secs(1);
        let mut control = probe(Color::WHITE);
        tree.diff(&mut control);
        frame(&mut control, &mut tree, start);
        draw(&control, &tree);
        frame(&mut control, &mut tree, start + Duration::from_millis(40));

        let mut expected = AnimationState::default();
        let _ = expected.resolve(Color::BLACK, start);
        let _ = expected.resolve(Color::WHITE, start);
        expected.advance(start + Duration::from_millis(40));
        let displayed = control.binding.0.borrow().animation.current().unwrap();
        assert_ne!(displayed, Color::WHITE);
        assert_eq!(Some(displayed), expected.current());
    }

    #[test]
    #[should_panic(expected = "exactly once per draw")]
    fn debug_builds_reject_a_style_function_that_was_never_installed() {
        struct Unstyled;
        impl Meta for Unstyled {}
        impl Widget<u8, (), ()> for Unstyled {
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
            }
        }

        let mut control = Animated::<_, Color>::bind(|animate| {
            let _unused = animate.wrap_unary(|color: &Color| *color);
            Unstyled
        });
        let mut tree = Tree::new(&control);
        control.diff(&mut tree);
        control.draw(
            &tree,
            &mut (),
            &(),
            &renderer::Style::default(),
            Layout::new(Size::new(20.0, 20.0)),
            mouse::Cursor::Unavailable,
            &Rectangle::with_size(Size::new(100.0, 100.0)),
        );
    }

    #[test]
    fn repeated_diffs_and_rebuilds_preserve_an_in_flight_trajectory() {
        let mut first = animate(ColorProbe {
            color: Color::WHITE,
            binding: Binding::default(),
        });
        let mut tree = Tree::new(&first);
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
            binding: Binding::default(),
        });
        tree.diff(&mut rebuilt);
        // Re-diffing the same builder must neither borrow its cell twice nor
        // move its state out of that same cell.
        tree.diff(&mut rebuilt);
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
        let mut tree = new_tree::<_, Message, Theme, ()>(&*control);
        Widget::<Message, Theme, ()>::diff(control, &mut tree);
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
    fn clipped_buttons_do_not_schedule_frames_for_unobserved_styles() {
        let mut control = animate(
            Button::<(), _, ()>::new(iced::widget::Space::new().width(30).height(20)).on_press(()),
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

    fn appearance_style(color: Color) -> Style {
        Style {
            background: Some(color.into()),
            text_color: color,
            ..Style::default()
        }
    }

    struct Segments;

    impl<'a> iced::widget::Component<'a, u8, Theme, ()> for Segments {
        type State = ();
        type Event = u8;

        fn update(&self, _: &mut (), event: u8, _: &()) -> Option<u8> {
            Some(event)
        }

        fn view(&self, _: &()) -> impl Widget<u8, Theme, ()> + 'a {
            iced::widget::Row::with_children((0..3).map(|index| {
                Button::<_, _, ()>::new(iced::widget::Space::new().width(30).height(20))
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
            }))
            .spacing(1)
        }
    }

    #[test]
    fn holding_a_segment_settles_to_its_pressed_style() {
        use iced::widget::button::Status;

        for index in 0..3 {
            let mut control = iced::widget::component(Segments);
            let mut tree = Tree::new(&control);
            tree.diff(&mut control);
            control.layout(
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
                    control.update(
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
                    control.draw(
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
                .downcast_ref::<Binding<Style>>()
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
                control.update(
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
            Button::<(), _, ()>::new(iced::widget::Space::new().width(30).height(20))
                .style(|_, _| appearance_style(Color::BLACK)),
        );
        let mut tree = layout_button(&mut control);
        let mut messages = Bus::new();
        draw(&control, &tree);
        let mut rebuilt = animate(
            Button::<(), _, ()>::new(iced::widget::Space::new().width(30).height(20))
                .style(|_, _| appearance_style(Color::WHITE)),
        );
        diff_tree::<_, (), Theme, ()>(&mut tree, &mut rebuilt);
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
                .downcast_ref::<Binding<Style>>()
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
            .downcast_ref::<Binding<Style>>()
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
        let mut tree = new_tree::<_, usize, Theme, ()>(&control);
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
        diff_tree::<_, usize, Theme, ()>(&mut tree, &mut rebuilt);
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
        diff_tree::<_, usize, Theme, ()>(&mut tree, &mut rebuilt);
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
