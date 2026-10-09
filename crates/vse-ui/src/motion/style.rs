//! Style transitions driven by one scalar [`Motion`].
//!
//! Widgets resolve their target style and schedule redraws. These types only
//! own interpolation, spring configuration, and animation time.

use super::{Motion, SpringConfig, mix_oklab, presets};
use iced::{Color, time::Instant};

/// The interpolation policy for a style.
///
/// `progress` is clamped to `[0, 1]`. The transition returns exact endpoint
/// styles itself; implementations only need to handle intermediate values.
pub trait TransitionStyle: Clone + PartialEq {
    fn mix(&self, to: &Self, progress: f32) -> Self;
}

impl TransitionStyle for Color {
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
pub struct Appearance<Style> {
    from: Style,
    to: Style,
    destination: Endpoint,
    progress: Motion,
    config: SpringConfig,
}

impl<S: TransitionStyle> Appearance<S> {
    /// Mount at the supplied style, already settled.
    pub fn new(style: S, now: Instant, config: SpringConfig) -> Self {
        Self {
            from: style.clone(),
            to: style,
            destination: Endpoint::To,
            progress: Motion::new_with_config(1.0, now, config),
            config,
        }
    }

    pub fn target(&self) -> &S {
        match self.destination {
            Endpoint::From => &self.from,
            Endpoint::To => &self.to,
        }
    }

    /// Reverse to an existing endpoint without resetting position or velocity.
    /// A third style starts a new blend from the currently displayed style.
    pub fn retarget(&mut self, target: S, now: Instant) {
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

    pub fn current(&self) -> S {
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
pub struct AnimationState<S> {
    appearance: Option<Appearance<S>>,
    config: SpringConfig,
}

impl<S> Default for AnimationState<S> {
    fn default() -> Self {
        Self::new(presets::snappy())
    }
}

impl<S> AnimationState<S> {
    pub fn new(config: SpringConfig) -> Self {
        Self {
            appearance: None,
            config,
        }
    }
}

impl<S: TransitionStyle> AnimationState<S> {
    /// Initialize or retarget to a resolved style, returning its displayed value.
    pub fn resolve(&mut self, target: S, now: Instant) -> S {
        match &mut self.appearance {
            Some(appearance) => appearance.retarget(target, now),
            None => self.appearance = Some(Appearance::new(target, now, self.config)),
        }
        self.appearance.as_ref().unwrap().current()
    }

    /// The displayed style, or `None` before the first style is resolved.
    pub fn current(&self) -> Option<S> {
        self.appearance.as_ref().map(Appearance::current)
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
        self.appearance
            .as_ref()
            .is_some_and(Appearance::is_animating)
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

    impl TransitionStyle for NamedStyle {
        fn mix(&self, to: &Self, progress: f32) -> Self {
            Self {
                brightness: self.brightness + (to.brightness - self.brightness) * progress,
                name: to.name.clone(),
            }
        }
    }

    #[test]
    fn owned_styles_mount_settled_and_transition_to_exact_endpoints() {
        let now = Instant::now();
        let from = NamedStyle::new(0.0, "off");
        let to = NamedStyle::new(1.0, "on");
        let mut state = AnimationState::default();
        assert!(state.current().is_none());
        assert!(!state.is_animating());
        assert_eq!(state.resolve(from.clone(), now), from);
        assert!(!state.is_animating());

        assert_eq!(state.resolve(to.clone(), now), from);
        state.advance(now + Duration::from_millis(40));
        let current = state.current().unwrap();
        assert!(current.brightness > 0.0 && current.brightness < 1.0);
        assert_eq!(current.name, "on");
        state.advance(now + Duration::from_secs(1));
        assert_eq!(state.current(), Some(to));
        assert!(!state.is_animating());
    }

    #[test]
    fn reversing_preserves_the_spring_and_uses_the_destination_policy() {
        let now = Instant::now();
        let from = NamedStyle::new(0.0, "off");
        let to = NamedStyle::new(1.0, "on");
        let mut appearance = Appearance::new(from.clone(), now, presets::snappy());
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
