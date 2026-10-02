use crate::impl_report_residual;
use crate::{app::ReportLike, apply_traits::ApplyConditional};
use iced::Color;
use iced::font::Weight;
use vse_ui::theme::WithRadius;

use super::*;
use cosmic::Element;
use iced::{Font, alignment::Vertical};
use vse_ui::{
    self as cosmic, Apply,
    theme::{self, COSMIC},
};

#[derive(Default)]
pub struct Model {
    pub video_player: video_player_widget::Model,
    pub screenshot_selection: Option<iced::Rectangle>,
    pub screenshot_selection_scaled: Option<iced::Rectangle>,
    pub canvas_dimensions: iced::Rectangle,
    pub canvas_generation: u32,
}

#[derive(derive_more::Debug, Clone)]
pub enum Message {
    ResetSelection,
    Canvas(selection_canvas::Message),
    VideoPlayer(video_player_widget::Message),
    CopySelectionDimensions(String),
    StartSubtitleDisplay,
}

pub enum Event {
    StartSubtitleSearch(std::path::PathBuf, Option<iced::Rectangle>),
    Run(Task<Message>),
    CopySelectionDimensions(String),
    Error(eyre::Report),
    None,
}

impl ReportLike for Event {
    fn err(e: eyre::Report) -> Self {
        Self::Error(e)
    }
    fn none() -> Self {
        Self::None
    }
}

impl_report_residual!(Event);

const fn playback_map(x: video_player_widget::PlayerMessage) -> Message {
    Message::VideoPlayer(video_player_widget::Message::Playback(x))
}

fn selection_details_translation(
    selection: iced::Rectangle,
    details: iced::Rectangle,
    viewport: iced::Rectangle,
    canvas_size: iced::Size,
) -> iced::Vector {
    // The selection is canvas-local; the float's bounds and viewport are absolute.
    let canvas = iced::Rectangle::new(details.position(), canvas_size);
    let visible = canvas.intersection(&viewport).unwrap_or(viewport);
    let gap = 8.0;
    let anchor = details.position() + iced::Vector::new(selection.x, selection.y);
    let above = anchor.y - details.height - gap;
    let y = if above >= visible.y {
        above
    } else {
        anchor.y + gap
    };
    let x = anchor.x.clamp(
        visible.x,
        (visible.x + visible.width - details.width).max(visible.x),
    );
    let y = y.clamp(
        visible.y,
        (visible.y + visible.height - details.height).max(visible.y),
    );

    iced::Vector::new(x - details.x, y - details.y)
}

impl Model {
    /// Other pages can be open before a video has been loaded.
    pub fn video_path(&self) -> Option<&std::path::PathBuf> {
        match &self.video_player {
            Some(player) => Some(player.path()),
            None => None,
        }
    }

    fn update(&mut self, message: Message) -> Event {
        let needs_recompute = Self::scaled_selection_needs_recomputation(&message);

        let task = match message {
            Message::ResetSelection => {
                self.screenshot_selection = None;
                self.canvas_generation = self.canvas_generation.wrapping_add(1);
                Event::None
            }
            Message::Canvas(x) => match x {
                selection_canvas::Message::ScreenshotRegion(rectangle) => {
                    self.screenshot_selection = rectangle;
                    Event::None
                }
                selection_canvas::Message::CanvasSize(rectangle) => {
                    self.screenshot_selection = self.screenshot_selection.map(|selection| {
                        selection_canvas::rescale_rectangle(
                            selection,
                            self.canvas_dimensions.size(),
                            rectangle.size(),
                        )
                    });
                    self.canvas_dimensions = rectangle;
                    Event::None
                }
            },
            Message::VideoPlayer(message) => match self.video_player.update(message, ()) {
                video_player_widget::Event::Run(task) => Event::Run(task.map(Message::VideoPlayer)),
                video_player_widget::Event::Error(error) => Event::Error(error),
                video_player_widget::Event::None => Event::None,
            },
            Message::CopySelectionDimensions(dimensions) => {
                Event::CopySelectionDimensions(dimensions)
            }
            Message::StartSubtitleDisplay => match &self.video_player {
                Some(player) => Event::StartSubtitleSearch(
                    player.path().clone(),
                    self.screenshot_selection_scaled,
                ),
                None => Event::None,
            },
        };

        if needs_recompute {
            self.recompute_scaled_selection();
        }
        task
    }

    fn view(&self) -> Element<'_, Message> {
        let Some(player) = &self.video_player else {
            return widget::space().into();
        };
        let img = player
            .image(COSMIC.corner_radii.radius_l[0])
            .map(playback_map);

        let selection_canvas = widget::canvas(selection_canvas::SelectionProgram::default())
            .width(Length::Fill)
            .height(Length::Fill)
            .apply(Element::from)
            .map(Message::Canvas);

        let canvas_size = self.canvas_dimensions.size();
        let s = self
            .screenshot_selection
            .and_then(|x| Some((x, self.screenshot_selection_scaled?)))
            .map(|(s, scaled)| {
                let format_dimensions = |s: iced::Rectangle| {
                    format!("{:.0} x {:.0} @ {:.0}, {:.0}", s.width, s.height, s.x, s.y)
                };
                widget::text(format_dimensions(scaled))
                    .font(Font::MONOSPACE.weight(Weight::Bold))
                    .color(Color::BLACK)
                    .apply(widget::button)
                    .on_press(
                        scaled
                            .apply(format_dimensions)
                            .apply(|x| x.chars().filter(|x| !x.is_whitespace()).collect::<String>())
                            .apply(Message::CopySelectionDimensions),
                    )
                    .style(|x, y| theme::button::suggested(x, y).with_radius(COSMIC.radius_s()[0]))
                    .apply(widget::float)
                    .translate(move |original, viewport| {
                        selection_details_translation(s, original, viewport, canvas_size)
                    })
                    .apply(Element::from)
            });

        let img_with_selection = widget::stack![img, selection_canvas, s];

        widget::column![img_with_selection, Self::video_controls(player)]
            .spacing(theme::spacing().space_s)
            .into()
    }

    pub fn title_actions(&self) -> Element<'_, Message> {
        let load_video = if self.video_player.is_some() {
            widget::button(widget::text(fl!("change-video")))
        } else {
            widget::button(widget::text(fl!("load-video"))).style(theme::button::suggested)
        }
        .on_press(Message::VideoPlayer(video_player_widget::Message::Loading(
            video_player_widget::LoadingMessage::PickVideo,
        )))
        .padding(theme::spacing().space_xs);

        let start_subtitle = widget::button(widget::text(fl!("find-subtitles")))
            .apply_if(self.video_player.is_some(), |x| {
                x.on_press(Message::StartSubtitleDisplay)
                    .style(theme::button::suggested)
            })
            .padding(theme::spacing().space_xs);

        widget::row![load_video, start_subtitle]
            .spacing(theme::spacing().space_xs)
            .into()
    }

    fn settings_view(&self) -> Element<'_, Message> {
        todo!()
    }

    fn video_controls(player: &video_player_widget::Player) -> Element<'_, Message> {
        let icon = |name| {
            widget::icon::from_name(name)
                .apply(widget::icon_button)
                .padding(theme::spacing().space_xs)
        };
        let seek_duration = Duration::from_secs(5);
        let pause = icon("media-playback-pause-symbolic");
        let start = icon("media-playback-start-symbolic");
        let forward = icon("media-seek-forward-symbolic").on_press(Message::VideoPlayer(
            video_player_widget::Message::Playback(
                video_player_widget::PlayerMessage::SeekForward(seek_duration),
            ),
        ));
        let backward = icon("media-seek-backward-symbolic").on_press(Message::VideoPlayer(
            video_player_widget::Message::Playback(
                video_player_widget::PlayerMessage::SeekBackward(seek_duration),
            ),
        ));
        let play_button = if player.is_paused() { start } else { pause }.on_press(
            Message::VideoPlayer(video_player_widget::Message::Playback(
                video_player_widget::PlayerMessage::PauseToggle,
            )),
        );

        let slider = player.slider().map(playback_map);
        let current = player
            .current_time()
            .apply(format_duration)
            .apply(widget::text)
            .font(Font::MONOSPACE);

        let total = player
            .video_duration()
            .apply(format_duration)
            .apply(widget::text)
            .font(Font::MONOSPACE);

        let row = widget::row!(backward, play_button, forward, current, slider, total)
            .spacing(theme::spacing().space_s)
            .align_y(Vertical::Center)
            .apply(widget::container)
            .padding(theme::spacing().space_m)
            .width(Length::Fill)
            .style(vse_ui::theme::container::card);
        row.into()
    }

    fn subscription(&self) -> Subscription<Message> {
        self.video_player.subscription(()).map(Message::VideoPlayer)
    }

    fn recompute_scaled_selection(&mut self) {
        let Some(player) = &self.video_player else {
            self.screenshot_selection_scaled = None;
            return;
        };
        let Some((_, size)) = player.allocation() else {
            self.screenshot_selection_scaled = None;
            return;
        };
        let Some(ele) = self.screenshot_selection else {
            self.screenshot_selection_scaled = None;
            return;
        };

        let canvas_size = self.canvas_dimensions.size();

        if canvas_size.width <= 0.0 || canvas_size.height <= 0.0 {
            self.screenshot_selection_scaled = None;
            return;
        }

        let scaled = selection_canvas::rescale_rectangle(ele, canvas_size, *size);

        self.screenshot_selection_scaled = Some(scaled);
    }

    const fn scaled_selection_needs_recomputation(message: &Message) -> bool {
        matches!(
            message,
            Message::VideoPlayer(video_player_widget::Message::Playback(
                video_player_widget::PlayerMessage::VideoFrameAllocated(_)
            )) | Message::Canvas(_)
                | Message::ResetSelection
        )
    }
}

impl Composition for Model {
    type Message = Message;
    type Event = Event;
    type ViewContext<'a> = ();
    type UpdateContext<'a> = ();
    type SubscriptionContext<'a> = ();

    fn view(&self, (): Self::ViewContext<'_>) -> Element<'_, Self::Message> {
        Self::view(self)
    }

    fn update(&mut self, message: Self::Message, (): Self::UpdateContext<'_>) -> Self::Event {
        Self::update(self, message)
    }

    fn subscription(&self, (): Self::SubscriptionContext<'_>) -> Subscription<Self::Message> {
        Self::subscription(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_details_stay_visible_at_every_canvas_corner() {
        let details =
            iced::Rectangle::new(iced::Point::new(120.0, 80.0), iced::Size::new(240.0, 36.0));
        let viewport = iced::Rectangle::with_size(iced::Size::new(1200.0, 900.0));
        for canvas_size in [iced::Size::new(800.0, 450.0), iced::Size::new(320.0, 180.0)] {
            let canvas = iced::Rectangle::new(details.position(), canvas_size);
            for x in [0.0, canvas_size.width - 20.0] {
                for y in [0.0, canvas_size.height - 20.0] {
                    let selection =
                        iced::Rectangle::new(iced::Point::new(x, y), iced::Size::new(20.0, 20.0));
                    let translation =
                        selection_details_translation(selection, details, viewport, canvas_size);
                    let placed =
                        iced::Rectangle::new(details.position() + translation, details.size());
                    assert!(placed.x >= canvas.x && placed.y >= canvas.y);
                    assert!(placed.x + placed.width <= canvas.x + canvas.width);
                    assert!(placed.y + placed.height <= canvas.y + canvas.height);
                }
            }
        }
    }

    #[test]
    fn selection_details_follow_the_selection_above_when_there_is_room() {
        let details =
            iced::Rectangle::new(iced::Point::new(120.0, 80.0), iced::Size::new(240.0, 36.0));
        let selection = iced::Rectangle::new(
            iced::Point::new(100.0, 100.0),
            iced::Size::new(200.0, 100.0),
        );
        let canvas_size = iced::Size::new(800.0, 450.0);
        let viewport = iced::Rectangle::with_size(iced::Size::new(1200.0, 900.0));
        assert_eq!(
            selection_details_translation(selection, details, viewport, canvas_size),
            iced::Vector::new(100.0, 56.0),
        );
    }

    #[test]
    fn selection_details_respect_a_clipped_viewport() {
        let details =
            iced::Rectangle::new(iced::Point::new(120.0, 80.0), iced::Size::new(240.0, 36.0));
        let viewport = iced::Rectangle::new(
            iced::Point::new(150.0, 100.0),
            iced::Size::new(400.0, 200.0),
        );
        let canvas_size = iced::Size::new(800.0, 450.0);
        for position in [iced::Point::ORIGIN, iced::Point::new(780.0, 430.0)] {
            let selection = iced::Rectangle::new(position, iced::Size::new(20.0, 20.0));
            let translation =
                selection_details_translation(selection, details, viewport, canvas_size);
            let placed = iced::Rectangle::new(details.position() + translation, details.size());
            assert!(placed.x >= viewport.x && placed.y >= viewport.y);
            assert!(placed.x + placed.width <= viewport.x + viewport.width);
            assert!(placed.y + placed.height <= viewport.y + viewport.height);
        }
    }

    #[test]
    fn canvas_resize_keeps_the_crop_in_sync_with_the_overlay() {
        let selection =
            iced::Rectangle::new(iced::Point::new(80.0, 90.0), iced::Size::new(640.0, 270.0));
        let original_bounds =
            iced::Rectangle::new(iced::Point::new(20.0, 30.0), iced::Size::new(800.0, 450.0));
        let mut model = Model {
            screenshot_selection: Some(selection),
            canvas_dimensions: original_bounds,
            ..Model::default()
        };
        model.update(Message::Canvas(selection_canvas::Message::CanvasSize(
            iced::Rectangle::new(iced::Point::new(40.0, 50.0), iced::Size::new(400.0, 225.0)),
        )));
        assert_eq!(
            model.screenshot_selection,
            Some(iced::Rectangle::new(
                iced::Point::new(40.0, 45.0),
                iced::Size::new(320.0, 135.0)
            )),
        );
        model.update(Message::Canvas(selection_canvas::Message::CanvasSize(
            original_bounds,
        )));
        assert_eq!(model.screenshot_selection, Some(selection));
    }
}
