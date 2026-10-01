use crate::app::ReportLike;
use crate::impl_report_residual;

use super::*;
use cosmic::iced::widget::Stack;
use cosmic::{Apply, Element};
use iced::advanced::graphics::core::length::Constraint;
use iced::alignment::Horizontal;
use std::time::Duration;
use vse_ui as cosmic;

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

impl Model {
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
            Message::StartSubtitleDisplay => {
                if let Some(path) = self.video_player.path() {
                    Event::StartSubtitleSearch(path.clone(), self.screenshot_selection_scaled)
                } else {
                    Event::None
                }
            }
        };

        if needs_recompute {
            self.recompute_scaled_selection();
        }
        task
    }

    fn view(&self) -> Element<'_, Message> {
        let space_s = cosmic::theme::spacing().space_s;

        let full_img_handle = self.video_player.frame_handle();

        let full_img = widget::image(&full_img_handle)
            .content_fit(iced::ContentFit::Contain)
            .expand(true)
            .width(Length::Shrink)
            .height(Length::Shrink);

        let canvas_widget = widget::canvas(selection_canvas::SelectionProgram {
            reset_generation: self.canvas_generation,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .apply(Element::from)
        .map(Message::Canvas);

        let cropped_img = self
            .screenshot_selection_scaled
            .unwrap_or_default()
            .apply(|ele| {
                let region = iced::Rectangle {
                    x: ele.x as u32,
                    y: ele.y as u32,
                    width: ele.width as u32,
                    height: ele.height as u32,
                };

                widget::image(full_img_handle)
                    .crop(region)
                    .width(Length::Shrink)
                    .height(Length::Shrink)
            });

        let full_img = Stack::new().push(full_img).push(canvas_widget);
        // Keep the selection canvas fitted to the image while reserving space
        // for the preview and controls before laying out the full frame.
        let full_img = widget::container(full_img)
            .center_x(Length::Fill)
            .height(Length::Fluid(Constraint::Max));

        let reset_btn = widget::button(widget::text(fl!("reset-selection")))
            .on_press(Message::ResetSelection)
            .style(cosmic::theme::button::destructive);

        let load_video = widget::button(widget::text(if self.video_player.path().is_none() {
            fl!("load-video")
        } else {
            fl!("change-video")
        }))
        .on_press(Message::VideoPlayer(
            video_player_widget::Message::PickVideo,
        ));

        let load_video = if self.video_player.path().is_none() {
            load_video.style(cosmic::theme::button::suggested)
        } else {
            load_video
        };

        let skip_backward = widget::button(icon::from_name("media-seek-backward-symbolic"))
            .on_press(Message::VideoPlayer(
                video_player_widget::Message::SeekBackward(Duration::from_secs(5)),
            ))
            .style(cosmic::theme::button::nav_toggle);

        let skip_forward = widget::button(icon::from_name("media-seek-forward-symbolic"))
            .on_press(Message::VideoPlayer(
                video_player_widget::Message::SeekForward(Duration::from_secs(5)),
            ))
            .style(cosmic::theme::button::nav_toggle);
        let selection_label: Element<'_, Message> = self.screenshot_selection_scaled.map_or_else(
            || {
                widget::text(fl!("select-region"))
                    .style(cosmic::theme::text::accent)
                    .into()
            },
            |rectangle| {
                let dimensions = format!(
                    "{:.0}×{:.0}@{:.0},{:.0}",
                    rectangle.width, rectangle.height, rectangle.x, rectangle.y
                );
                let label = widget::text(fl!("selection", dimensions = dimensions.clone()))
                    .style(cosmic::theme::text::accent);

                widget::mouse_area(label)
                    .on_press(Message::CopySelectionDimensions(dimensions))
                    .interaction(iced::mouse::Interaction::Pointer)
                    .into()
            },
        );

        let find_subs = widget::button(widget::text(fl!("find-subtitles")));
        let find_subs = if self.video_player.path().is_some() {
            find_subs
                .on_press(Message::StartSubtitleDisplay)
                .style(cosmic::theme::button::suggested)
        } else {
            find_subs
        };

        let slider = self
            .video_player
            .controller()
            .map(|x| x.inner.info.video_time.as_secs_f64())
            .map(|video_time| {
                widget::slider(
                    0.0..=video_time,
                    self.video_player.current_time().as_secs_f64(),
                    |x| {
                        Message::VideoPlayer(video_player_widget::Message::SeekAbsolute(
                            Duration::from_secs_f64(x),
                        ))
                    },
                )
            });

        let current_time = widget::text(self.video_player.current_time().apply(format_duration))
            .width(Length::Fill)
            .align_x(Horizontal::Right);

        widget::column! {
            full_img,
            cropped_img,
            slider,
            current_time,
            widget::row! {
                load_video,
                reset_btn,
                selection_label,
                skip_backward,
                skip_forward,
                find_subs,
            }
            .spacing(space_s)
            .align_y(Alignment::Center)
        }
        .spacing(space_s)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        self.video_player.subscription(()).map(Message::VideoPlayer)
    }

    fn recompute_scaled_selection(&mut self) {
        let Some((_, size)) = self.video_player.allocation() else {
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
            Message::VideoPlayer(video_player_widget::Message::VideoFrameAllocated(_))
                | Message::Canvas(_)
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
