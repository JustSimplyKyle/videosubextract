//! A minimal animated switch driven by Iced's `transition` widget.
//!
//! The parent owns only the boolean; the widget tree owns the spring and its
//! frames.
//! Run the demo with `cargo run -p vse-ui --example switch`.
//! This example handles pointer and touch input; keyboard focus is not implemented.

use crate::Apply;
use crate::motion::{Motion, mix_oklab};
use iced::advanced::{mouse, renderer};
use iced::time::Instant;
use iced::widget::{Container, Space, container, mouse_area, row, transition};
use iced::{Border, Color, Element, Fill, Theme};
use std::rc::Rc;

#[derive(Default, Clone, Copy)]
pub struct Size {
    width: f32 = 46.0,
    height: f32 = 28.0,
    inset: f32 = 3.0,
}

impl Size {
    pub const fn small() -> Self {
        Self {
            width: 46.0 * 0.75,
            height: 28.0 * 0.75,
            inset: 3.0 * 0.75,
        }
    }
    pub const fn large() -> Self {
        Self {
            width: 46.0 * 1.25,
            height: 28.0 * 1.25,
            inset: 3.0 * 1.25,
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
    /// Choose a spring preset. Applies on mount; the transition keeps its
    /// motion afterwards.
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
        let on_toggle: Option<Rc<dyn Fn(bool) -> Message + 'a>> = on_toggle.map(Rc::from);

        transition(
            target,
            // Mount at the incoming value rather than animating from zero.
            move || Motion::new_with_config(target, Instant::now(), spring_config),
            move |motion: &Motion, _| -> Element<'a, Message, Theme, Renderer> {
                let progress = motion.value().clamp(0.0, 1.0);
                let Some(on_toggle) = on_toggle.clone() else {
                    return track(size, progress, false).into();
                };

                let on_toggle = on_toggle.clone();
                mouse_area(track(size, progress, true))
                    .on_press(())
                    .interaction(mouse::Interaction::Pointer)
                    .apply(Element::from)
                    .map(move |()| on_toggle(!is_enabled))
            },
        )
        .into()
    }
}

/// The track and thumb at `progress`, with the track color blended in Oklab.
fn track<'a, Message: 'a, Renderer: renderer::Renderer + 'a>(
    size: Size,
    progress: f32,
    interactive: bool,
) -> Container<'a, Message, Theme, Renderer> {
    let alpha = if interactive { 1.0 } else { 0.45 };

    let thumb = container(Space::new())
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

    let content = row![Space::new().width(size.track_width() * progress), thumb]
        .width(Fill)
        .height(Fill)
        .align_y(iced::Alignment::Center);
    container(content)
        .padding(size.inset)
        .width(size.width)
        .height(size.height)
        .style(move |theme: &Theme| container::Style {
            background: Some(
                mix_oklab(
                    theme.palette().background.strong.color,
                    theme.palette().primary.base.color,
                    progress,
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

#[cfg(test)]
mod tests {
    use super::*;
    use iced::advanced::shell::{Bus, Waker};
    use iced::advanced::widget::Tree;
    use iced::advanced::{Layout, Shell, layout};
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

    #[test]
    fn mounts_settled_animates_changes_and_redraws_stop() {
        let now = Instant::now();
        let mut control: Control = switch(true).on_toggle(|value| value).into();
        let mut tree = Tree::new(control.as_widget());
        reconcile(&mut control, &mut tree);
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
        send(&mut control, &mut tree, frame(now + Duration::from_secs(5)));
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
