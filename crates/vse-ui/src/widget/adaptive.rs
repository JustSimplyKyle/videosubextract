//! Breakpoint-driven layouts built on [`iced::widget::responsive`].

use iced::Size;

use iced::widget::Widget;

/// Width classes, following Material's window size classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Breakpoint {
    /// Narrower than [`Breakpoint::MEDIUM`]; single column.
    Compact,
    /// Between [`Breakpoint::MEDIUM`] and [`Breakpoint::EXPANDED`].
    Medium,
    /// At least [`Breakpoint::EXPANDED`]; room for side-by-side panes.
    Expanded,
}

impl Breakpoint {
    pub const MEDIUM: f32 = 600.0;
    pub const EXPANDED: f32 = 840.0;

    pub fn from_width(width: f32) -> Self {
        if width >= Self::EXPANDED {
            Self::Expanded
        } else if width >= Self::MEDIUM {
            Self::Medium
        } else {
            Self::Compact
        }
    }
}

/// The space handed to an [`adaptive`] view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub size: Size,
    pub breakpoint: Breakpoint,
}

impl Viewport {
    pub fn new(size: Size) -> Self {
        Self {
            size,
            breakpoint: Breakpoint::from_width(size.width),
        }
    }

    pub fn at_least(self, breakpoint: Breakpoint) -> bool {
        self.breakpoint >= breakpoint
    }

    pub fn is_expanded(self) -> bool {
        matches!(self.breakpoint, Breakpoint::Expanded)
    }

    /// Choose a value per breakpoint, e.g. `viewport.pick(space_s, space_m, space_l)`.
    pub fn pick<T>(self, compact: T, medium: T, expanded: T) -> T {
        match self.breakpoint {
            Breakpoint::Compact => compact,
            Breakpoint::Medium => medium,
            Breakpoint::Expanded => expanded,
        }
    }
}

/// Builds content from the available space, classified into a [`Breakpoint`].
pub fn adaptive<'a, Message: 'a, W: Widget<Message> + 'a>(
    view: impl Fn(Viewport) -> W + 'a,
) -> impl Widget<Message> + 'a {
    iced::widget::responsive(move |size| view(Viewport::new(size)))
}
