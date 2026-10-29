//! Transitions between values driven by one scalar [`Motion`].
//!
//! The values are usually widget styles, but any [`Mix`] type works. Widgets
//! resolve their target and schedule redraws. These types only own
//! interpolation, spring configuration, and animation time.

use super::{Motion, SpringConfig, mix_oklab, presets};
use iced::{Color, time::Instant, widget::transition};

/// How to interpolate between two values of a type.
///
/// `progress` is clamped to `[0, 1]`. The transition returns exact endpoint
/// values itself; implementations only need to handle intermediate values.
pub trait Mix: Clone + PartialEq {
    fn mix(&self, to: &Self, progress: f32) -> Self;
}

impl Mix for Color {
    fn mix(&self, to: &Self, progress: f32) -> Self {
        mix_oklab(*self, *to, progress)
    }
}

#[derive(Clone, Copy)]
enum Endpoint {
    From,
    To,
}

/// Two style endpoints and a single spring for their blend progress.
#[derive(Clone)]
pub struct Blend<Style> {
    from: Style,
    to: Style,
    destination: Endpoint,
    progress: Motion,
    config: SpringConfig,
}

impl<Value: Mix> Blend<Value> {
    /// Mount at the supplied style, already settled.
    pub fn new(style: Value, now: Instant, config: SpringConfig) -> Self {
        Self {
            from: style.clone(),
            to: style,
            destination: Endpoint::To,
            progress: Motion::new_with_config(1.0, now, config),
            config,
        }
    }

    pub fn target(&self) -> &Value {
        match self.destination {
            Endpoint::From => &self.from,
            Endpoint::To => &self.to,
        }
    }

    /// Reverse to an existing endpoint without resetting position or velocity.
    /// A third style starts a new blend from the currently displayed style.
    pub fn retarget(&mut self, target: Value, now: Instant) {
        if &target == self.target() {
            return;
        }
        if target == self.from {
            self.destination = Endpoint::From;
            self.progress.retarget(0.0, now);
        } else if target == self.to {
            self.destination = Endpoint::To;
            self.progress.retarget(1.0, now);
        } else {
            self.progress.advance(now);
            self.from = self.current();
            self.to = target;
            self.destination = Endpoint::To;
            self.progress = Motion::new_with_config(0.0, now, self.config);
            self.progress.retarget(1.0, now);
        }
    }

    pub fn current(&self) -> Value {
        let progress = self.progress.value().clamp(0.0, 1.0);
        if progress <= 0.0 {
            self.from.clone()
        } else if progress >= 1.0 {
            self.to.clone()
        } else {
            match self.destination {
                Endpoint::From => self.to.mix(&self.from, 1.0 - progress),
                Endpoint::To => self.from.mix(&self.to, progress),
            }
        }
    }

    pub fn advance(&mut self, now: Instant) {
        self.progress.advance(now);
    }

    pub fn set_config(&mut self, config: SpringConfig, now: Instant) {
        self.config = config;
        self.progress.set_config(config, now);
    }

    pub fn is_animating(&self) -> bool {
        self.progress.is_animating()
    }
}

/// only deals with spring movement, parent decides when to "update" the spring
#[derive(Clone)]
pub struct AnimationState<Value> {
    appearance: Option<Blend<Value>>,
    config: SpringConfig,
}

impl<Value: Mix + Copy + 'static> transition::Program for AnimationState<Value> {
    type Value = Value;

    fn go(&mut self, value: Self::Value, now: Instant) {
        self.resolve(value, now);
        self.advance(now);
    }

    fn is_animating(&self, _now: Instant) -> bool {
        self.is_animating()
    }
}

impl<Value> Default for AnimationState<Value> {
    fn default() -> Self {
        Self::new(presets::snappy())
    }
}

impl<Value> AnimationState<Value> {
    pub fn new(config: SpringConfig) -> Self {
        Self {
            appearance: None,
            config,
        }
    }
}

impl<Value: Mix> AnimationState<Value> {
    /// Initialize or retarget to a resolved style, returning its displayed value.
    pub fn resolve(&mut self, target: Value, now: Instant) -> Value {
        match &mut self.appearance {
            Some(appearance) => appearance.retarget(target, now),
            None => self.appearance = Some(Blend::new(target, now, self.config)),
        }
        self.appearance.as_ref().unwrap().current()
    }

    /// The displayed style, or `None` before the first style is resolved.
    pub fn current(&self) -> Option<Value> {
        self.appearance.as_ref().map(Blend::current)
    }

    pub fn set_config(&mut self, config: SpringConfig, now: Instant) {
        self.config = config;
        if let Some(appearance) = &mut self.appearance {
            appearance.set_config(config, now);
        }
    }

    pub fn advance(&mut self, now: Instant) {
        if let Some(appearance) = &mut self.appearance {
            appearance.advance(now);
        }
    }

    pub fn is_animating(&self) -> bool {
        self.appearance.as_ref().is_some_and(Blend::is_animating)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::time::Duration;

    // Neither Copy nor Default: styles may own data and choose which
    // properties interpolate and which follow the destination immediately.
    #[derive(Clone, Debug, PartialEq)]
    struct NamedStyle {
        brightness: f32,
        name: String,
    }

    impl NamedStyle {
        fn new(brightness: f32, name: &str) -> Self {
            Self {
                brightness,
                name: name.into(),
            }
        }
    }

    impl Mix for NamedStyle {
        fn mix(&self, to: &Self, progress: f32) -> Self {
            Self {
                brightness: self.brightness + (to.brightness - self.brightness) * progress,
                name: to.name.clone(),
            }
        }
    }

    #[test]
    fn transition_program_repeats_enter_and_exit() {
        use transition::Program;

        let now = Instant::now();
        let off = Color::WHITE.scale_alpha(0.0);
        let on = Color::WHITE.scale_alpha(0.35);
        let mut state = AnimationState::default();
        state.go(off, now);

        for cycle in 0..2 {
            let enter = now + Duration::from_secs(cycle * 4 + 1);
            state.go(on, enter);
            assert!(
                state.is_animating(),
                "hover {cycle} must retarget the spring"
            );
            state.go(on, enter + Duration::from_millis(40));
            let alpha = state.current().unwrap().a;
            assert!(alpha > off.a && alpha < on.a);
            state.go(on, enter + Duration::from_secs(1));
            assert_eq!(state.current(), Some(on));
            assert!(!state.is_animating());

            let exit = enter + Duration::from_secs(2);
            state.go(off, exit);
            assert!(state.is_animating());
            state.go(off, exit + Duration::from_millis(40));
            let alpha = state.current().unwrap().a;
            assert!(alpha > off.a && alpha < on.a);
            state.go(off, exit + Duration::from_secs(1));
            assert_eq!(state.current(), Some(off));
            assert!(!state.is_animating());
        }
    }

    #[test]
    fn reversing_preserves_the_spring_and_uses_the_destination_policy() {
        let now = Instant::now();
        let from = NamedStyle::new(0.0, "off");
        let to = NamedStyle::new(1.0, "on");
        let mut appearance = Blend::new(from.clone(), now, presets::snappy());
        appearance.retarget(to, now);
        let later = now + Duration::from_millis(40);
        appearance.advance(later);
        let progress = appearance.progress.value();
        let velocity = appearance.progress.velocity();
        let brightness = appearance.current().brightness;
        assert!(velocity > 0.0);

        appearance.retarget(from.clone(), later);
        assert_eq!(appearance.progress.value(), progress);
        assert_eq!(appearance.progress.velocity(), velocity);
        assert!((appearance.current().brightness - brightness).abs() < 0.000001);
        assert_eq!(appearance.current().name, "off");
        appearance.advance(now + Duration::from_secs(1));
        assert_eq!(appearance.current(), from);
    }

    #[test]
    fn a_third_style_starts_from_the_displayed_style() {
        let now = Instant::now();
        let mut state = AnimationState::new(presets::snappy());
        state.resolve(NamedStyle::new(0.0, "off"), now);
        state.resolve(NamedStyle::new(1.0, "on"), now);
        let later = now + Duration::from_millis(40);
        state.advance(later);
        let displayed = state.current().unwrap();
        let third = NamedStyle::new(0.5, "hover");
        assert_eq!(state.resolve(third.clone(), later), displayed);
        state.advance(now + Duration::from_secs(1));
        assert_eq!(state.current(), Some(third));
        assert!(!state.is_animating());
    }
}
