//! A minimal animated switch driven by Iced's `transition` widget.
//!
//! The parent owns only the boolean; the widget tree owns the spring and its
//! frames.
//! Run the demo with `cargo run -p vse-ui --example switch`.
//! This example handles pointer and touch input; keyboard focus is not implemented.

use crate::Apply;
use crate::motion::{Motion, Motions, mix_oklab, presets};
use crate::widget::Animated;
use iced::advanced::renderer;
use iced::time::Instant;
use iced::widget::{Container, Space, button, container, row, stack, transition};
use iced::{Border, Color, Element, Fill, Theme};
use std::cell::Cell;
use std::rc::Rc;

#[derive(Default, Clone, Copy)]
pub struct Size {
    width: f32 = 46.0,
    height: f32 = 28.0,
    inset: f32 = 3.0,
}

impl Size {
    pub fn small() -> Self {
        let default = Self::default();
        Self {
            width: default.width * 0.75,
            height: default.height * 0.75,
            inset: default.inset * 0.75,
        }
    }
    pub fn large() -> Self {
        let default = Self::default();
        Self {
            width: default.width * 1.25,
            height: default.height * 1.25,
            inset: default.inset * 1.25,
        }
    }
    const fn thumb_size(&self) -> f32 {
        2.0_f32.mul_add(-self.inset, self.height)
    }
    const fn track_width(&self) -> f32 {
        2.0_f32.mul_add(-self.inset, self.width) - self.thumb_size()
    }
}

/// Creates a switch. Without an `on_toggle` callback it is disabled.
pub fn switch<'a, Message>(is_enabled: bool) -> Switch<'a, Message> {
    Switch {
        is_enabled,
        on_toggle: None,
        spring_config: crate::motion::presets::gentle(),
        size: Default::default(),
    }
}

/// An interruptible switch whose track color interpolates in Oklab.
pub struct Switch<'a, Message> {
    is_enabled: bool,
    on_toggle: Option<crate::components::MessageEmitter<'a, bool, Message>>,
    spring_config: springs::SpringConfig,
    size: Size,
}

impl<'a, Message> Switch<'a, Message> {
    /// Emits the new logical value immediately when clicked or touched.
    #[must_use]
    pub fn on_toggle(mut self, callback: impl Fn(bool) -> Message + 'a) -> Self {
        self.on_toggle = Some(Rc::new(callback));
        self
    }
    /// Choose the thumb's spring preset; bouncy presets overshoot into the
    /// track's padding. The track color always uses a critically damped
    /// spring. Changes apply on the next frame, keeping the thumb's motion.
    #[must_use]
    pub const fn with_config(mut self, config: springs::SpringConfig) -> Self {
        self.spring_config = config;
        self
    }
    #[must_use]
    pub const fn with_size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }
}

impl<'a, Message: 'a, Renderer: renderer::Renderer + 'a> From<Switch<'a, Message>>
    for Element<'a, Message, Theme, Renderer>
{
    fn from(switch: Switch<'a, Message>) -> Self {
        let Switch {
            is_enabled,
            on_toggle,
            spring_config,
            size,
        } = switch;
        let target = if is_enabled { 1.0 } else { 0.0 };

        transition(
            [(target, spring_config), (target, presets::snappy())],
            // Mount at the incoming value rather than animating from zero.
            move || {
                let now = Instant::now();
                Motions([Motion::new(target, now), Motion::new(target, now)])
            },
            move |Motions([thumb, color]): &Motions<2>,
                  _|
                  -> Element<'a, Message, Theme, Renderer> {
                let thumb = thumb.value();
                let color = color.value();
                let Some(on_toggle) = on_toggle.clone() else {
                    return track(size, thumb, color, false, None).into();
                };

                // The button reports its status only to its style function, which
                // runs before its content draws, so the hint can read it from here.
                let hovered = Rc::new(Cell::new(false));
                let status = hovered.clone();
                button(track(
                    size,
                    thumb,
                    color,
                    true,
                    Some(hint(size, thumb, hovered)),
                ))
                .padding(0)
                .on_press(())
                .style(move |_, current| {
                    status.set(matches!(
                        current,
                        button::Status::Hovered | button::Status::Pressed
                    ));
                    button::Style::default()
                })
                .apply(Element::from)
                .map(move |()| on_toggle(!is_enabled))
            },
        )
        .into()
    }
}

/// The track with its thumb at `thumb` progress and its color blended in Oklab
/// at `color` progress. The thumb may overshoot into the padding, but never
/// past the track's edge. An optional `hint` is layered between the track and
/// the thumb.
fn track<'a, Message: 'a, Renderer: renderer::Renderer + 'a>(
    size: Size,
    thumb: f32,
    color: f32,
    interactive: bool,
    hint: Option<Element<'a, Message, Theme, Renderer>>,
) -> Container<'a, Message, Theme, Renderer> {
    let alpha = if interactive { 1.0 } else { 0.45 };

    let knob = container(Space::new())
        .width(size.thumb_size())
        .height(size.thumb_size())
        .style(move |_| container::Style {
            background: Some(Color::WHITE.scale_alpha(alpha).into()),
            border: Border {
                radius: (size.thumb_size() / 2.0).into(),
                ..Border::default()
            },
            ..container::Style::default()
        });

    let offset = size
        .track_width()
        .mul_add(thumb, size.inset)
        .clamp(0.0, 2.0f32.mul_add(size.inset, size.track_width()));
    let content = row![Space::new().width(offset), knob]
        .width(Fill)
        .height(Fill)
        .align_y(iced::Alignment::Center);
    container(stack(hint.into_iter().chain([content.into()])))
        .width(size.width)
        .height(size.height)
        .style(move |theme: &Theme| container::Style {
            background: Some(
                mix_oklab(
                    theme.palette().background.strong.color,
                    theme.palette().primary.base.color,
                    color.clamp(0.0, 1.0),
                )
                .scale_alpha(alpha)
                .into(),
            ),
            border: Border {
                radius: (size.height / 2.0).into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
}

/// A translucent circle behind the thumb that fades in while `hovered`.
fn hint<'a, Message: 'a, Renderer: renderer::Renderer + 'a>(
    size: Size,
    thumb: f32,
    hovered: Rc<Cell<bool>>,
) -> Element<'a, Message, Theme, Renderer> {
    let circle = Animated::bind(|animate| {
        let color = animate.wrap_unary(move |_: &Theme| {
            Color::WHITE.scale_alpha(if hovered.get() { 0.35 } else { 0.0 })
        });
        container(Space::new())
            .width(size.height)
            .height(size.height)
            .style(move |theme| container::Style {
                background: Some(color(theme).into()),
                border: Border {
                    radius: (size.height / 2.0).into(),
                    ..Border::default()
                },
                ..container::Style::default()
            })
    });

    let offset = (size.track_width() * thumb).clamp(0.0, size.track_width());
    row![Space::new().width(offset), circle]
        .width(Fill)
        .height(Fill)
        .align_y(iced::Alignment::Center)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::advanced::shell::{Bus, Waker};
    use iced::advanced::widget::Tree;
    use iced::advanced::{Layout, Shell, layout, mouse};
    use iced::{Event, Point, Rectangle, time::Duration, window};

    type Control = Element<'static, bool, Theme, ()>;

    fn send(
        control: &mut Control,
        tree: &mut Tree,
        event: Event,
    ) -> (window::RedrawRequest, Vec<bool>) {
        let mut messages = Bus::new();
        let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut messages);
        control.as_widget_mut().update(
            tree,
            &event,
            Layout::new(tree.size),
            mouse::Cursor::Available(Point::new(5.0, 5.0)),
            &(),
            &mut shell,
            &Rectangle::with_size(iced::Size::new(100.0, 100.0)),
        );
        let redraw = shell.redraw_request();
        drop(shell);
        (
            redraw,
            messages.drain().map(|(message, _)| message).collect(),
        )
    }

    fn reconcile(control: &mut Control, tree: &mut Tree) {
        tree.diff(control.as_widget_mut());
        control.as_widget_mut().layout(
            tree,
            &(),
            &layout::Limits::new(iced::Size::ZERO, iced::Size::new(100.0, 100.0)),
        );
    }

    fn frame(now: Instant) -> Event {
        Event::Window(window::Event::RedrawRequested(now))
    }

    /// Animated styles resolve while drawing, as after each frame of a real
    /// event loop.
    fn draw(control: &Control, tree: &Tree) {
        control.as_widget().draw(
            tree,
            &mut (),
            &Theme::Light,
            &renderer::Style::default(),
            Layout::new(tree.size),
            mouse::Cursor::Unavailable,
            &Rectangle::with_size(iced::Size::new(100.0, 100.0)),
        );
    }

    #[test]
    fn mounts_settled_animates_changes_and_redraws_stop() {
        let now = Instant::now();
        let mut control: Control = switch(true).on_toggle(|value| value).into();
        let mut tree = Tree::new(control.as_widget());
        reconcile(&mut control, &mut tree);
        draw(&control, &tree);
        // Initially mounted switches start at their supplied value.
        let (redraw, _) = send(&mut control, &mut tree, frame(now));
        assert!(matches!(redraw, window::RedrawRequest::Wait));

        control = switch(false).on_toggle(|value| value).into();
        reconcile(&mut control, &mut tree);
        let (redraw, _) = send(
            &mut control,
            &mut tree,
            frame(now + Duration::from_millis(16)),
        );
        assert!(!matches!(redraw, window::RedrawRequest::Wait));
        draw(&control, &tree);
        send(&mut control, &mut tree, frame(now + Duration::from_secs(5)));
        draw(&control, &tree);
        let (redraw, _) = send(&mut control, &mut tree, frame(now + Duration::from_secs(6)));
        assert!(matches!(redraw, window::RedrawRequest::Wait));
    }

    #[test]
    fn presses_emit_the_toggled_value_only_when_interactive() {
        for (control, expected) in [
            (switch(false).on_toggle(|value| value), vec![true]),
            (switch(false), vec![]),
        ] {
            let mut sim: iced_test::Simulator<'_, bool> = iced_test::Simulator::new(control);
            sim.point_at(Point::new(10.0, 10.0));
            sim.simulate(iced_test::simulator::click());
            assert_eq!(sim.drain().collect::<Vec<_>>(), expected);
        }
    }
}
