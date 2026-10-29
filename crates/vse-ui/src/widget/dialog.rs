//! Modal presentation driven by Iced's `transition` widget. Keep the host
//! mounted and change `open`; the widget tree owns the spring.
//!
//! The content factory stays concrete and is called through the exit
//! animation, once per frame while animating. The rendered child is removed
//! when the exit becomes visually complete, without waiting for the spring's
//! settling tail.

use std::marker::PhantomData;

use super::auto_widget::{AutoWidget, IntoWidget};
use crate::{
    Apply,
    motion::{EarlyExitMotion, Motion, SpringConfig, presets},
};
use iced::advanced::{Widget, renderer};
use iced::time::Instant;
use iced::widget::{Space, Widget as _, container, float, opaque};
use iced::{Color, Element, Fill, Theme, Vector};

mod gate;

/// Exits below this progress are visually complete.
const EXIT_THRESHOLD: f32 = 0.01;

/// Build a persistent host from requested visibility and a concrete content
/// factory. Rebuild the host on every application view, including while closed.
pub fn dialog<'a, Message, W, C, Renderer>(
    open: bool,
    content: C,
) -> Dialog<'a, Message, C, Renderer>
where
    Message: 'a,
    C: Fn() -> W + 'a,
    W: Widget<Message, Theme, Renderer> + 'a,
    Renderer: renderer::Renderer + 'a,
{
    AutoWidget::from_builder(Builder {
        open,
        content,
        config: presets::quick(),
        types: PhantomData,
    })
}

/// The parent owns requested visibility; the transition's tree state owns
/// displayed visibility.
pub type Dialog<'a, Message, C, Renderer = iced::Renderer> =
    AutoWidget<Builder<'a, Message, C, Renderer>>;

pub struct Builder<'a, Message, C, Renderer> {
    open: bool,
    content: C,
    config: SpringConfig,
    types: PhantomData<&'a (Message, Renderer)>,
}

impl<'a, Message, W, C, Renderer> Dialog<'a, Message, C, Renderer>
where
    Message: 'a,
    C: Fn() -> W + 'a,
    W: Widget<Message, Theme, Renderer> + 'a,
    Renderer: renderer::Renderer + 'a,
{
    /// Changes apply on the next frame, keeping the current motion.
    #[must_use]
    pub fn with_config(self, config: SpringConfig) -> Self {
        self.map_builder(|mut dialog| {
            dialog.config = config;
            dialog
        })
    }
}

impl<'a, Message, W, C, Renderer> IntoWidget for Builder<'a, Message, C, Renderer>
where
    Message: 'a,
    C: Fn() -> W + 'a,
    W: Widget<Message, Theme, Renderer> + 'a,
    Renderer: renderer::Renderer + 'a,
{
    type Message = Message;
    type Theme = Theme;
    type Renderer = Renderer;
    type Widget = impl Widget<Message, Theme, Renderer> + 'a;

    fn into_widget(self) -> Self::Widget {
        let Builder {
            open,
            content,
            config,
            ..
        } = self;

        iced::widget::transition(
            (if open { 1.0 } else { 0.0 }, config),
            || EarlyExitMotion::new(Motion::new(0.0, Instant::now()), EXIT_THRESHOLD),
            move |exit: &EarlyExitMotion, _| -> Element<'a, Message, Theme, Renderer> {
                if open || exit.is_present() {
                    present(content(), exit.motion().value(), open).boxed()
                } else {
                    Space::new().width(Fill).height(Fill).boxed()
                }
            },
        )
    }
}

/// The modal backdrop and panel at `progress`. Closing content is muted so it
/// cannot emit messages during the exit.
fn present<'a, Message: 'a, Renderer: renderer::Renderer + 'a>(
    content: impl Widget<Message, Theme, Renderer> + 'a,
    progress: f32,
    open: bool,
) -> impl Widget<Message, Theme, Renderer> + 'a {
    let offset = crate::theme::spacing().space_s * (1.0 - progress);

    gate::gate(open, content)
        .apply(opaque)
        .apply(float)
        .scale(0.05f32.mul_add(progress, 0.95).max(0.01))
        .translate(move |_, _| Vector::new(0.0, offset))
        .apply(container)
        .center(Fill)
        .style(move |_| {
            // clamping because overshooting in color looks weird
            container::background(Color::BLACK.scale_alpha(0.55 * progress.clamp(0.0, 1.0)))
        })
        .apply(opaque)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::{button, column, container, stack, text_input};
    use iced::{Event, time::Duration, window};
    use iced_test::{Simulator, selector};

    #[derive(Debug, Clone, PartialEq)]
    enum Message {
        Page,
        Inside,
        Edit(String),
    }

    fn view(open: bool) -> impl iced::widget::Widget<Message> {
        // A stack sizes its layers to the first one, so the page fills the window.
        stack![
            container(button("Page").on_press(Message::Page))
                .width(Fill)
                .height(Fill),
            dialog(open, || {
                column![
                    text_input("Name", "").id("name").on_input(Message::Edit),
                    button("Inside").on_press(Message::Inside),
                ]
            }),
        ]
    }

    fn frame(sim: &mut Simulator<'_, Message>, now: Instant) {
        sim.simulate([Event::Window(window::Event::RedrawRequested(now))]);
    }

    fn drain(sim: &mut Simulator<'_, Message>) -> Vec<Message> {
        sim.drain().collect()
    }

    #[test]
    fn mounts_lazily_mutes_while_closing_and_keeps_child_state_on_reversal() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);

        let mut sim = Simulator::new(view(false));
        frame(&mut sim, at(0));
        assert!(sim.find("Inside").is_err(), "closed dialogs build nothing");
        sim.click("Page").unwrap();
        assert_eq!(drain(&mut sim), [Message::Page]);

        let mut sim = sim.rebuild(view(true));
        for ms in (16..=1000).step_by(16) {
            frame(&mut sim, at(ms));
        }
        sim.click("Inside").unwrap();
        sim.click(selector::id("name")).unwrap();
        sim.typewrite("a");
        assert_eq!(
            drain(&mut sim),
            [Message::Inside, Message::Edit("a".into())]
        );

        // Closing content still receives input but emits nothing.
        let mut sim = sim.rebuild(view(false));
        frame(&mut sim, at(1016));
        frame(&mut sim, at(1032));
        sim.typewrite("b");
        assert_eq!(drain(&mut sim), []);

        // Reopening mid-exit keeps the input's focus, so its state survived.
        let mut sim = sim.rebuild(view(true));
        frame(&mut sim, at(1048));
        sim.typewrite("c");
        assert_eq!(drain(&mut sim), [Message::Edit("c".into())]);

        let mut sim = sim.rebuild(view(false));
        frame(&mut sim, at(2000));
        frame(&mut sim, at(2016));
        sim.click("Page").unwrap();
        assert_eq!(drain(&mut sim), [], "the closing dialog shields the page");

        for ms in (2032..=2400).step_by(16) {
            frame(&mut sim, at(ms));
        }
        assert!(
            sim.find("Inside").is_err(),
            "finished exits unmount content"
        );
        sim.click("Page").unwrap();
        assert_eq!(drain(&mut sim), [Message::Page]);
    }
}
