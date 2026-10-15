//! Spring motion and color blending for stateful UI components.
//!
//! Keep [`Motion`] in `Component::State`, rather than in the short-lived component
//! builder: Iced reconstructs builders when the application's view changes, while
//! the widget tree preserves component state. The parent owns the logical value;
//! the component owns the displayed value and animation clock.
//!
//! A component normally uses these helpers in four places:
//!
//! 1. **`diff`**: initialize motion at the incoming value on first mount. On later
//!    changes, call [`Motion::retarget`] with the new value and `Instant::now()`.
//!    Initializing at the incoming value avoids animating an already-enabled
//!    control from zero. Recreating motion on every change would discard velocity.
//! 2. **`listen`**: call [`frame_action`] with whether any of the component's motions
//!    are active, and a constructor for an internal `Frame(Instant)` event.
//! 3. **`update`**: handle that internal frame event by calling [`Motion::advance`]
//!    on each motion. Return `None` for frames; only meaningful user interactions
//!    should emit messages to the parent application.
//! 4. **`view`**: read [`Motion::value`] for spacer widths, offsets, scale, or color
//!    blend weights. Clamp bounded properties such as opacity to `[0, 1]`; geometry
//!    may intentionally overshoot when using a bouncy configuration.
//!
//! No application-level frame subscription is needed. These helpers assume the
//! component remains mounted during the animation. For content that appears and
//! disappears, such as a dialog or toast, prefer `transition` with an
//! [`EarlyExitMotion`], below.
//!
//! # With `transition`
//!
//! Without a component, Iced's [`transition`](iced::widget::transition()) widget
//! can own the spring instead. [`Motion`] implements its `Program`: the
//! transition's value is the target, the widget tree keeps the motion across
//! view rebuilds, and the widget schedules its own frames. The `view` closure
//! runs every frame while animating, so keep it cheap.
//!
//! ```
//! use iced::time::Instant;
//! use iced::widget::{progress_bar, transition};
//! use vse_ui::motion::{Motion, presets};
//!
//! fn smooth_progress<'a, Message: 'a>(progress: f32) -> iced::Element<'a, Message> {
//!     transition(
//!         progress,
//!         || Motion::new_with_config(0.0, Instant::now(), presets::gentle()),
//!         |motion: &Motion, _| progress_bar(0.0..=1.0, motion.value()),
//!     )
//!     .into()
//! }
//! ```
//!
//! For content that mounts and unmounts, drive an [`EarlyExitMotion`] between
//! `1.0` and `0.0`. Keep the transition mounted, and render the content while
//! the parent shows it or [`EarlyExitMotion::is_present`] holds:
//!
//! ```
//! use iced::time::Instant;
//! use iced::widget::{Space, text, transition};
//! use iced::{Color, Element};
//! use vse_ui::motion::{EarlyExitMotion, Motion, presets};
//!
//! fn toast<'a, Message: 'a>(visible: bool) -> Element<'a, Message> {
//!     transition(
//!         if visible { 1.0 } else { 0.0 },
//!         || EarlyExitMotion::new(Motion::new_with_config(0.0, Instant::now(), presets::quick()), 0.01),
//!         move |exit: &EarlyExitMotion, _| -> Element<'a, Message> {
//!             if visible || exit.is_present() {
//!                 let opacity = exit.motion().value().clamp(0.0, 1.0);
//!                 text("Saved").color(Color::BLACK.scale_alpha(opacity)).into()
//!             } else {
//!                 Space::new().into()
//!             }
//!         },
//!     )
//!     .into()
//! }
//! ```
//!
//! # Example
//!
//! ```
//! use iced::time::{Duration, Instant};
//! use vse_ui::motion::{Motion, SpringConfig, mix_oklab};
//!
//! let now = Instant::now();
//! let mut progress = Motion::new(0.0, now)
//!     .with_config(SpringConfig::new().duration(0.3).bounce(0.0));
//! progress.retarget(1.0, now);
//! progress.advance(now + Duration::from_millis(80));
//! let t = progress.value().clamp(0.0, 1.0);
//! let track = mix_oklab(iced::Color::BLACK, iced::Color::WHITE, t);
//! assert!(track.r > 0.0 && track.r < 1.0);
//! ```

use iced::time::Instant;
use iced::widget::Action;
use iced::{Color, Event, window};
use springs::Spring;

mod blend;
pub use blend::{AnimationState, Blend, Mix};

/// Configuration for the underlying analytical spring solver.
///
/// `duration` controls natural response time, not a hard completion deadline.
/// `bounce(0.0)` gives critical damping; positive bounce allows overshoot.
pub use springs::SpringConfig;

/// Named physical spring configurations shared by UI components.
///
/// | Preset | Mass | Stiffness | Damping | Damping ratio |
/// | --- | ---: | ---: | ---: | ---: |
/// | [`gentle`] | 1 | 100 | 15 | 0.750 |
/// | [`quick`] | 1 | 300 | 20 | 0.577 |
/// | [`snappy`] | 1 | 1600 | 80 | 1.000 |
/// | [`bouncy`] | 1 | 600 | 15 | 0.306 |
/// | [`slow`] | 1 | 80 | 20 | 1.118 |
///
/// Configurations use the exact physical values, not the rounded damping ratios
/// in the table. The solver derives `omega = sqrt(stiffness / mass)` and
/// `ratio = damping / (2 * sqrt(stiffness * mass))`.
///
/// New motions start with zero velocity, matching the fourth value in the supplied
/// `[mass, stiffness, damping, initial_velocity]` presets. Retargeting an existing
/// motion preserves its velocity so interruptions remain smooth.
///
/// ```
/// use iced::time::Instant;
/// use vse_ui::motion::{Motion, presets};
///
/// let now = Instant::now();
/// let mut offset = Motion::new_with_config(0.0, now, presets::quick());
/// offset.retarget(100.0, now);
///
/// // The builder form is equivalent:
/// let opacity = Motion::new(0.0, now).with_config(presets::slow());
/// ```
///
/// Gentle, quick, and bouncy are underdamped and can overshoot. Clamp their output
/// when driving bounded properties such as opacity, color weights, or progress.
/// Snappy is critically damped; slow is overdamped. The default [`Motion::new`] configuration remains critically
/// damped; choose one of these presets explicitly when initializing motion.
pub mod presets {
    use super::SpringConfig;

    /// Gentle motion with slight overshoot: mass 1, stiffness 100, damping 15.
    pub fn gentle() -> SpringConfig {
        SpringConfig::from_physical(1.0, 100.0, 15.0)
    }

    /// A quicker response with overshoot: mass 1, stiffness 300, damping 20.
    pub fn quick() -> SpringConfig {
        SpringConfig::from_physical(1.0, 300.0, 20.0)
    }

    /// Fast color feedback without overshoot: mass 1, stiffness 1600, damping 80.
    /// Reaches about 91% of the target in 100 ms and 99.7% in 200 ms.
    pub fn snappy() -> SpringConfig {
        SpringConfig::from_physical(1.0, 1600.0, 80.0)
    }

    /// A brisk, oscillating response: mass 1, stiffness 600, damping 15.
    pub fn bouncy() -> SpringConfig {
        SpringConfig::from_physical(1.0, 600.0, 15.0)
    }

    /// A slow, overdamped response: mass 1, stiffness 80, damping 20.
    pub fn slow() -> SpringConfig {
        SpringConfig::from_physical(1.0, 80.0, 20.0)
    }
}

/// A scalar spring and its clock, suitable for persistent component state.
///
/// Values can represent normalized progress, pixels, scale, or another continuous
/// scalar. Time advancement uses the analytical solver in `springs`, so a long
/// frame interval does not require numerical integration substeps.
pub struct Motion {
    spring: Spring<f32>,
    now: Instant,
}

impl Motion {
    /// Start at rest at `initial`, with a 0.3-second response and no bounce.
    ///
    /// The default epsilon is `0.0001`, suitable for normalized UI progress.
    /// Initialize once on first mount, using the actual incoming property value.
    pub fn new(initial: f32, now: Instant) -> Self {
        Self::new_with_config(initial, now, SpringConfig::new().duration(0.3).bounce(0.0))
    }

    /// Start at rest using an explicit configuration, such as [`presets::quick`].
    ///
    /// Equivalent to `Motion::new(initial, now).with_config(config)`. Prefer this
    /// constructor when choosing a preset on first mount in `Component::diff`.
    pub fn new_with_config(initial: f32, now: Instant, config: SpringConfig) -> Self {
        Self {
            spring: Spring::new(initial)
                .with_config(config)
                .with_epsilon(0.0001),
            now,
        }
    }

    /// Select a motion preset when initializing the component state.
    ///
    /// For example, `.with_config(SpringConfig::new().duration(0.4).bounce(0.2))`
    /// gives geometry a small bounce. Changing this does not reset position or
    /// velocity, but can change the trajectory of an already-running spring.
    pub fn with_config(mut self, config: SpringConfig) -> Self {
        self.spring = self.spring.with_config(config);
        self
    }

    /// Change configuration on an existing motion without resetting its state.
    ///
    /// Call from `Component::diff` when a parent supplies a configurable preset.
    /// The old spring advances to `now` before applying the new configuration,
    /// preserving position and velocity even when changed during an animation.
    pub fn set_config(&mut self, config: SpringConfig, now: Instant) {
        self.advance(now);
        self.spring = self.spring.with_config(config);
    }

    /// Set the settlement tolerance in the same units as the animated value.
    ///
    /// For pixel motion, a tolerance such as `0.01` may be sufficient. The solver
    /// also checks velocity, with its threshold derived from this tolerance and
    /// the spring's natural frequency. `epsilon` must be finite and positive.
    pub fn with_epsilon(mut self, epsilon: f64) -> Self {
        self.spring = self.spring.with_epsilon(epsilon);
        self
    }

    /// Whether more animation frames are needed.
    ///
    /// The solver snaps to the exact target and zero velocity when it settles.
    pub fn is_animating(&self) -> bool {
        !self.spring.is_settled()
    }

    /// Advance to the timestamp from an internal redraw event.
    ///
    /// Equal or older timestamps are ignored: a queued redraw may predate a
    /// property change handled in `diff`. Never move the animation clock backwards.
    pub fn advance(&mut self, now: Instant) {
        if now <= self.now {
            return;
        }
        self.spring
            .advance(now.duration_since(self.now).as_secs_f64());
        self.now = now;
    }

    /// Read the displayed value, without advancing time or clamping overshoot.
    pub fn value(&self) -> f32 {
        self.spring.value()
    }

    /// Read the destination of the current trajectory.
    pub fn target(&self) -> f32 {
        self.spring.target()
    }

    /// Read velocity in animated-value units per second.
    pub fn velocity(&self) -> f32 {
        self.spring.velocity()
    }

    /// Whether a spring heading to zero is visually gone: at or below
    /// `threshold` and not moving back up. Underdamped springs reach this
    /// before they settle.
    pub fn is_exit_complete(&self, threshold: f32) -> bool {
        self.value() <= threshold && self.velocity() <= 0.0
    }

    /// Come to rest at `value` immediately, discarding velocity.
    ///
    /// Use to end an animation once it is visually complete, so an underdamped
    /// spring cannot keep oscillating around its target.
    pub fn snap_to(&mut self, value: f32) {
        self.spring.snap_to(value);
    }

    /// Change the destination while preserving current position and velocity.
    ///
    /// First advances the old trajectory to `now`, then changes its target. This
    /// makes rapid reversals continuous. An unchanged target does nothing, so
    /// unrelated application updates cannot keep restarting an animation.
    pub fn retarget(&mut self, target: f32, now: Instant) {
        if target != self.spring.target() {
            self.advance(now);
            self.spring.set_target(target);
        }
    }
}

/// Drives a [`transition`](iced::widget::transition()) with a spring. The
/// transition's value is the target; retargeting keeps position and velocity.
/// See the [module docs](self#with-transition).
impl iced::widget::transition::Program for Motion {
    type Value = f32;

    fn go(&mut self, target: f32, now: Instant) {
        self.retarget(target, now);
        self.advance(now);
    }

    fn is_animating(&self, _: Instant) -> bool {
        Motion::is_animating(self)
    }
}

/// A [`Motion`] that comes to rest at zero once it falls to `threshold`,
/// instead of oscillating there. Other targets keep their full motion,
/// including overshoot.
///
/// Use it for content that mounts and unmounts: it ends the exit as soon as
/// the content is visually gone, and cannot bounce it back into view.
pub struct EarlyExitMotion {
    motion: Motion,
    threshold: f32,
}

impl EarlyExitMotion {
    /// `threshold` is in the motion's units; `0.01` suits normalized progress.
    pub fn new(motion: Motion, threshold: f32) -> Self {
        Self { motion, threshold }
    }

    pub fn motion(&self) -> &Motion {
        &self.motion
    }

    /// Whether content should stay mounted: heading anywhere but zero, or
    /// still exiting.
    ///
    /// This reads the motion's target rather than the parent's visibility. A
    /// rebuild right after hiding runs before the next frame retargets the
    /// motion, and must not unmount content whose exit has not started yet.
    pub fn is_present(&self) -> bool {
        self.motion.target() != 0.0 || self.motion.is_animating()
    }
}

impl iced::widget::transition::Program for EarlyExitMotion {
    type Value = f32;

    fn go(&mut self, target: f32, now: Instant) {
        self.motion.go(target, now);
        if target == 0.0
            && self.motion.is_animating()
            && self.motion.is_exit_complete(self.threshold)
        {
            self.motion.snap_to(0.0);
        }
    }

    fn is_animating(&self, _: Instant) -> bool {
        self.motion.is_animating()
    }
}

/// Build the runtime action for an animating component's `listen` method.
///
/// `frame_event` constructs an **internal component event**, not a parent message.
/// Use one frame event to advance all of a component's springs with the same time:
///
/// ```
/// use iced::{Event, time::Instant};
/// use vse_ui::motion::frame_action;
///
/// enum InternalEvent { Frame(Instant) }
/// fn listen(event: &Event, is_animating: bool) -> iced::widget::Action<InternalEvent> {
///     frame_action(event, is_animating, InternalEvent::Frame)
/// }
/// ```
///
/// While active, a redraw publishes a frame event and schedules another redraw.
/// Other runtime events request the first/next redraw without publishing a frame.
/// Once all motions settle, pass `false` to stop requesting frames. There may be
/// one already-scheduled trailing redraw, which then produces no action.
///
/// Scheduling happens here because `Component::update` can return only an optional
/// parent message. Publishing an internal event alone is insufficient to sustain
/// the redraw loop. This action does not capture input; handle clicks or touches
/// separately before calling it.
pub fn frame_action<FrameEvent>(
    event: &Event,
    is_animating: bool,
    frame_event: impl FnOnce(Instant) -> FrameEvent,
) -> Action<FrameEvent> {
    if !is_animating {
        return Action::none();
    }
    if let &Event::Window(window::Event::RedrawRequested(now)) = event {
        return Action::publish(frame_event(now)).and_request_redraw_at(now);
    }
    Action::request_redraw()
}

/// Blend opaque UI colors along a straight line in Cartesian Oklab.
///
/// Supply normalized progress, typically `motion.value().clamp(0.0, 1.0)`.
/// Weights outside that range return the respective endpoint exactly. Oklab gives
/// more perceptually even changes than blending gamma-encoded sRGB channels.
///
/// Iced exposes Oklch (lightness, chroma, hue), so this converts chroma/hue to
/// Cartesian `a`/`b`, blends them with lightness, and converts back. Interpolating
/// hue directly would take a different path in Oklch. Iced's conversion back to
/// sRGB clips out-of-gamut channels; this is not a gamut-mapping algorithm.
///
/// Alpha is blended separately. This helper is intended for opaque theme colors;
/// blending arbitrary transparent colors needs a premultiplied-alpha policy.
/// Changing opacity alone can use `color.scale_alpha(opacity)` instead.
pub fn mix_oklab(from: Color, to: Color, t: f32) -> Color {
    if t <= 0.0 {
        return from;
    }
    if t >= 1.0 {
        return to;
    }
    let from = from.into_oklch();
    let to = to.into_oklch();
    let lerp = |a: f32, b: f32| a + (b - a) * t;
    let a = lerp(from.c * from.h.cos(), to.c * to.h.cos());
    let b = lerp(from.c * from.h.sin(), to.c * to.h.sin());
    Color::from_oklch(iced::color::Oklch {
        l: lerp(from.l, to.l),
        c: a.hypot(b),
        h: b.atan2(a),
        a: lerp(from.a, to.a),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::time::Duration;
    use iced::widget::transition::Program;

    #[test]
    fn reversal_preserves_position_and_velocity() {
        let now = Instant::now();
        let mut motion = Motion::new(0.0, now);
        motion.retarget(1.0, now);
        let halfway = now + Duration::from_millis(70);
        motion.advance(halfway);
        let position = motion.value();
        let velocity = motion.velocity();
        assert!(position > 0.0 && position < 1.0);
        assert!(velocity > 0.0);

        motion.retarget(0.0, halfway);
        assert_eq!(motion.value(), position);
        assert_eq!(motion.velocity(), velocity);
        motion.advance(now + Duration::from_secs(5));
        assert_eq!(motion.value(), 0.0);
        assert!(!motion.is_animating());
    }

    #[test]
    fn quick_exit_snaps_to_zero_but_entry_keeps_its_overshoot() {
        let now = Instant::now();
        let mut exit =
            EarlyExitMotion::new(Motion::new_with_config(0.0, now, presets::quick()), 0.01);
        let frames =
            |start: Instant| (1..=60).map(move |frame| start + Duration::from_millis(frame * 16));

        exit.go(1.0, now);
        let peak = frames(now)
            .map(|now| {
                exit.go(1.0, now);
                exit.motion().value()
            })
            .fold(0.0, f32::max);
        assert!(peak > 1.0, "entry is not clipped");

        let closing = now + Duration::from_secs(5);
        exit.go(1.0, closing);
        exit.go(0.0, closing);
        let finished = frames(closing)
            .position(|now| {
                exit.go(0.0, now);
                !Program::is_animating(&exit, now)
            })
            .expect("the exit finishes");
        assert!(finished * 16 <= 200, "finished after {} ms", finished * 16);
        for now in frames(closing + Duration::from_secs(1)) {
            exit.go(0.0, now);
            assert!(!Program::is_animating(&exit, now));
            assert_eq!(exit.motion().value(), 0.0);
        }
    }

    #[test]
    fn oklab_blend_keeps_endpoints_and_interpolates_lightness() {
        let from = Color::from_rgb8(40, 45, 55);
        let to = Color::from_rgb8(45, 195, 110);
        assert_eq!(mix_oklab(from, to, 0.0), from);
        assert_eq!(mix_oklab(from, to, 1.0), to);
        let midpoint = mix_oklab(from, to, 0.5).into_oklch();
        let expected = (from.into_oklch().l + to.into_oklch().l) / 2.0;
        assert!((midpoint.l - expected).abs() < 0.0001);
    }
}
