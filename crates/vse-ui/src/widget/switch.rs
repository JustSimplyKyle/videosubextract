//! A minimal animated switch with component-owned spring state.
//!
//! The parent owns only the boolean. Redraws and animation events stay internal.
//! Run the demo with `cargo run -p vse-ui --example switch`.
//! This example handles pointer and touch input; keyboard focus is not implemented.

use crate::motion::{Motion, frame_action, mix_oklab};
use iced::advanced::mouse;
use iced::time::Instant;
use iced::widget::{Action, Component, Space, component, container, mouse_area, row};
use iced::{Border, Color, Element, Event, Fill, Rectangle, Renderer, Theme};

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
        self.on_toggle = Some(Box::new(callback));
        self
    }
    /// Choose a spring preset. Changes apply to the existing motion, including
    /// during an animation, while preserving its current position and velocity.
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

#[doc(hidden)]
#[derive(Default)]
pub struct SwitchState {
    motion: Option<Motion>,
}

#[doc(hidden)]
#[derive(Clone)]
pub enum SwitchEvent {
    Toggle,
    Frame(Instant),
}

impl<'a, Message: 'a> Component<'a, Message> for Switch<'a, Message> {
    type State = SwitchState;
    type Event = SwitchEvent;

    fn diff(&mut self, state: &mut SwitchState) {
        let now = Instant::now();
        let target = if self.is_enabled { 1.0 } else { 0.0 };
        match &mut state.motion {
            Some(motion) => {
                motion.retarget(target, now);
                motion.set_config(self.spring_config, now);
            }
            None => state.motion = Some(Motion::new_with_config(target, now, self.spring_config)),
        }
    }

    fn listen(
        &self,
        state: &SwitchState,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Action<SwitchEvent> {
        let pressed = match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                cursor.is_over(bounds)
            }
            Event::Touch(iced::touch::Event::FingerPressed { position, .. }) => {
                bounds.contains(*position)
            }
            _ => false,
        };
        if pressed && self.on_toggle.is_some() {
            return Action::publish(SwitchEvent::Toggle).and_capture();
        }

        frame_action(
            event,
            state.motion.as_ref().is_some_and(Motion::is_animating),
            SwitchEvent::Frame,
        )
    }

    fn update(&self, state: &mut SwitchState, event: SwitchEvent, _: &Renderer) -> Option<Message> {
        match event {
            SwitchEvent::Toggle => self.on_toggle.as_ref().map(|f| f(!self.is_enabled)),
            SwitchEvent::Frame(now) => {
                state.motion.as_mut()?.advance(now);
                None
            }
        }
    }

    fn view(&self, state: &SwitchState) -> Element<'a, SwitchEvent> {
        let progress = state
            .motion
            .as_ref()
            .map_or(if self.is_enabled { 1.0 } else { 0.0 }, Motion::value)
            .clamp(0.0, 1.0);
        let interactive = self.on_toggle.is_some();
        let alpha = if interactive { 1.0 } else { 0.45 };

        let size = self.size;

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
        let track = container(content)
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
            });
        mouse_area(track)
            .interaction(if interactive {
                mouse::Interaction::Pointer
            } else {
                mouse::Interaction::None
            })
            .into()
    }
}

impl<'a, Message: 'a> From<Switch<'a, Message>> for Element<'a, Message> {
    fn from(switch: Switch<'a, Message>) -> Self {
        component(switch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::{time::Duration, window};

    #[test]
    fn redraws_stop_after_settling() {
        let now = Instant::now();
        let mut control = switch(true).on_toggle(|value| value);
        let mut state = SwitchState::default();
        control.diff(&mut state);
        // Initially mounted switches start at their supplied value.
        assert_eq!(state.motion.as_ref().unwrap().value(), 1.0);
        assert!(!state.motion.as_ref().unwrap().is_animating());

        control.is_enabled = false;
        control.diff(&mut state);
        let size = Size::default();
        let bounds = Rectangle::with_size(iced::Size::new(size.width, size.height));
        let frame_time = now + Duration::from_secs(5);
        let event = Event::Window(window::Event::RedrawRequested(frame_time));
        let (published, redraw, _) = control
            .listen(&state, &event, bounds, mouse::Cursor::Unavailable)
            .into_inner();
        assert!(matches!(redraw, window::RedrawRequest::At(_)));
        let Some(SwitchEvent::Frame(timestamp)) = published else {
            panic!("an active spring must receive an internal frame event");
        };
        state.motion.as_mut().unwrap().advance(timestamp);
        assert_eq!(state.motion.as_ref().unwrap().value(), 0.0);

        let (published, redraw, _) = control
            .listen(&state, &event, bounds, mouse::Cursor::Unavailable)
            .into_inner();
        assert!(published.is_none());
        assert!(matches!(redraw, window::RedrawRequest::Wait));
    }
}
