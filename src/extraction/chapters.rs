//! Turn detected title-card runs into contiguous chapters.
//!
//! A run is one stretch of time where the monitored region showed the same
//! text. Chapters are derived from where that text *changes*: identical
//! neighbours are merged, short flashes (transitions, a title briefly
//! obscured) are absorbed into the chapter before them, and every chapter
//! ends where the next one starts so the result covers the whole video.

use std::time::Duration;

use super::Subtitle;
use super::chapter_writer::{ChapterLocation, ChapterLocations};

#[derive(Debug, Clone)]
pub struct ChapterOptions {
    /// Chapters shorter than this are absorbed into the previous one.
    pub min_length: Duration,
    /// Title for the chapter inserted at 0 when the first title appears later
    /// than `min_length` into the video.
    pub leading_title: String,
}

/// Collapse a recognized title onto one line: chapter titles are single-line
/// metadata, but OCR of a two-line title card yields a line break.
fn normalize_title(text: &str) -> String {
    text.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" / ")
}

/// `runs` must be ordered by start time. `total` is the video's duration; the
/// last chapter ends there (or at the last run's end, if that is later).
pub fn runs_to_chapters(
    runs: &[Subtitle],
    total: Duration,
    options: &ChapterOptions,
) -> ChapterLocations {
    let candidates = runs
        .iter()
        .map(|run| (run.start_timestamp, normalize_title(run.text())))
        .filter(|(_, title)| !title.is_empty())
        .collect::<Vec<_>>();
    let total = runs
        .iter()
        .map(|run| run.end_timestamp)
        .fold(total, Duration::max);

    let mut kept: Vec<(Duration, String)> = Vec::with_capacity(candidates.len());
    for (index, (start, title)) in candidates.iter().enumerate() {
        let next_start = candidates.get(index + 1).map_or(total, |(next, _)| *next);
        let too_short = next_start.saturating_sub(*start) < options.min_length;
        match kept.last() {
            Some(_) if too_short => {}
            Some((_, previous)) if previous == title => {}
            _ => kept.push((*start, title.clone())),
        }
    }

    if let Some((first_start, _)) = kept.first_mut() {
        if *first_start < options.min_length {
            *first_start = Duration::ZERO;
        } else {
            kept.insert(0, (Duration::ZERO, options.leading_title.clone()));
        }
    }

    let ends = kept
        .iter()
        .skip(1)
        .map(|(start, _)| *start)
        .chain(std::iter::once(total))
        .collect::<Vec<_>>();

    ChapterLocations(
        kept.into_iter()
            .zip(ends)
            .map(|((start, name), end)| ChapterLocation { start, end, name })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(start: u64, end: u64, text: &str) -> Subtitle {
        Subtitle::new(
            Duration::from_secs(start),
            Duration::from_secs(end),
            text.to_owned(),
        )
    }

    fn options() -> ChapterOptions {
        ChapterOptions {
            min_length: Duration::from_secs(10),
            leading_title: "Intro".into(),
        }
    }

    fn summary(chapters: &ChapterLocations) -> Vec<(u64, u64, &str)> {
        chapters
            .0
            .iter()
            .map(|c| (c.start.as_secs(), c.end.as_secs(), c.name.as_str()))
            .collect()
    }

    #[test]
    fn empty_input_has_no_chapters() {
        let chapters = runs_to_chapters(&[], Duration::from_secs(60), &options());
        assert!(chapters.0.is_empty());
    }

    #[test]
    fn chapters_are_contiguous_and_end_at_the_video_end() {
        let runs = [run(1, 170, "One"), run(180, 400, "Two")];
        let chapters = runs_to_chapters(&runs, Duration::from_secs(600), &options());
        assert_eq!(summary(&chapters), [(0, 180, "One"), (180, 600, "Two")]);
    }

    #[test]
    fn identical_neighbours_merge_even_across_gaps() {
        let runs = [
            run(0, 50, "One"),
            run(90, 170, " One "),
            run(180, 300, "Two"),
        ];
        let chapters = runs_to_chapters(&runs, Duration::from_secs(300), &options());
        assert_eq!(summary(&chapters), [(0, 180, "One"), (180, 300, "Two")]);
    }

    #[test]
    fn short_flashes_are_absorbed_into_the_previous_chapter() {
        let runs = [
            run(0, 99, "One"),
            run(100, 102, "glitch"),
            run(102, 170, "One"),
            run(180, 300, "Two"),
        ];
        let chapters = runs_to_chapters(&runs, Duration::from_secs(300), &options());
        assert_eq!(summary(&chapters), [(0, 180, "One"), (180, 300, "Two")]);
    }

    #[test]
    fn late_first_title_gets_a_leading_chapter() {
        let runs = [run(45, 300, "One")];
        let chapters = runs_to_chapters(&runs, Duration::from_secs(300), &options());
        assert_eq!(summary(&chapters), [(0, 45, "Intro"), (45, 300, "One")]);
    }

    #[test]
    fn runs_past_the_reported_duration_extend_the_last_chapter() {
        let runs = [run(0, 320, "One")];
        let chapters = runs_to_chapters(&runs, Duration::ZERO, &options());
        assert_eq!(summary(&chapters), [(0, 320, "One")]);
    }

    #[test]
    fn multi_line_titles_are_flattened_and_blank_runs_skipped() {
        let runs = [
            run(0, 100, "  Track 1 \n\n  Song   name "),
            run(100, 200, " \n "),
        ];
        let chapters = runs_to_chapters(&runs, Duration::from_secs(200), &options());
        assert_eq!(summary(&chapters), [(0, 200, "Track 1 / Song name")]);
    }
}
