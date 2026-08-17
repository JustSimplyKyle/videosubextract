use crate::config::Config;
use crate::extraction::{self, Event, OcrHandle, Request, Subtitle};
use crate::video_player::CropRect;
use clap::Parser;
use eyre::{Context, ContextCompat};
use futures::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Args {
    /// Video containing hard subtitles. Omit it to launch the graphical interface.
    #[arg(value_name = "VIDEO")]
    pub input: Option<PathBuf>,

    /// Destination SRT file. Defaults to the input path with an .srt extension.
    #[arg(short, long, value_name = "SRT", requires = "input")]
    pub output: Option<PathBuf>,

    /// Crop rectangle in source-video pixels, formatted as WIDTHxHEIGHT@X,Y.
    #[arg(short, long, value_name = "WIDTHxHEIGHT@X,Y", requires = "input")]
    pub crop: Option<Crop>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crop {
    width: u32,
    height: u32,
    x: u32,
    y: u32,
}

impl FromStr for Crop {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (dimensions, position) = value
            .split_once('@')
            .ok_or_else(|| "expected WIDTHxHEIGHT@X,Y".to_owned())?;
        let (width, height) = dimensions
            .split_once('x')
            .ok_or_else(|| "expected WIDTHxHEIGHT before '@'".to_owned())?;
        let (x, y) = position
            .split_once(',')
            .ok_or_else(|| "expected X,Y after '@'".to_owned())?;

        let parse = |field: &str, name: &str| {
            field
                .parse::<u32>()
                .map_err(|_| format!("{name} must be a non-negative integer"))
        };
        let crop = Self {
            width: parse(width, "width")?,
            height: parse(height, "height")?,
            x: parse(x, "x")?,
            y: parse(y, "y")?,
        };

        if crop.width == 0 || crop.height == 0 {
            return Err("width and height must be greater than zero".into());
        }

        Ok(crop)
    }
}

impl From<Crop> for CropRect {
    fn from(crop: Crop) -> Self {
        Self {
            x: crop.x as f32,
            y: crop.y as f32,
            width: crop.width as f32,
            height: crop.height as f32,
        }
    }
}

pub async fn run(input: PathBuf, output: PathBuf, crop: Option<Crop>) -> eyre::Result<()> {
    let config = Config::default();
    let request = Request {
        input,
        crop: crop.map(Into::into),
        ocr: OcrHandle::new(config.ocr_model),
        detector: config.subtitle_detector,
        native_search_params: config.native_search_params,
        post_ocr_processing: config.post_ocr_processing,
        processing_resolution: config.processing_resolution,
        progress_interval: 10,
        include_progress_preview: false,
    };
    let mut events = Box::pin(extraction::stream(request));
    let progress = ProgressBar::new_spinner();
    progress.set_style(
        ProgressStyle::with_template(
            "{spinner:.cyan} [{elapsed_precise}] {wide_bar:.cyan/blue} {pos}/{len} frames ({eta})",
        )
        .expect("valid progress bar template")
        .progress_chars("=>-"),
    );
    progress.set_message("Opening video");
    progress.enable_steady_tick(std::time::Duration::from_millis(100));

    let mut subtitles = Vec::<Subtitle>::new();
    let mut completed = false;
    while let Some(event) = events.next().await {
        match event {
            // Event::Started { total_frames } => {
            //     if let Some(total_frames) = total_frames {
            //         progress.set_length(total_frames as u64);
            //     }
            //     progress.set_message("Extracting subtitles");
            // }
            Event::Progress { timestamp, .. } => {
                progress.set_position(timestamp.as_millis() as u64);
            }
            Event::SubtitleFound {
                subtitle,
                replace_previous,
                ..
            } => {
                if replace_previous {
                    let previous = subtitles
                        .last_mut()
                        .context("extractor replaced a missing subtitle")?;
                    *previous = subtitle;
                } else {
                    subtitles.push(subtitle);
                }
            }
            Event::Finished => {
                completed = true;
                break;
            }
            Event::Error(error) => {
                progress.abandon_with_message("Extraction failed");
                return Err(eyre::eyre!(error));
            }
        }
    }

    if !completed {
        progress.abandon_with_message("Extraction stopped");
        return Err(eyre::eyre!("subtitle extraction stopped before completion"));
    }

    progress.finish_with_message(format!(
        "Found {} subtitle{}",
        subtitles.len(),
        if subtitles.len() == 1 { "" } else { "s" }
    ));
    let srt = extraction::to_srt(&subtitles);
    std::fs::write(&output, srt).wrap_err_with(|| format!("writing {}", output.display()))?;
    println!("{}", output.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_crop_rectangle() {
        assert_eq!(
            "1920x280@0,0".parse(),
            Ok(Crop {
                width: 1920,
                height: 280,
                x: 0,
                y: 0,
            })
        );
    }

    #[test]
    fn rejects_invalid_crop_rectangles() {
        assert!("1920x0@0,0".parse::<Crop>().is_err());
        assert!("1920x280".parse::<Crop>().is_err());
        assert!("1920x280@-1,0".parse::<Crop>().is_err());
    }
}
