use std::fmt::Write;
use std::path::Path;
use std::{path::PathBuf, time::Duration};

use ffmpeg_sidecar::command::FfmpegCommand;
use ffmpeg_the_third::{Rational, Rescale, media};

const CHAPTER_TIME_BASE: Rational = Rational(1, 1000);

error_set::error_set! {
    EmbedWriterError := {
        #[display("failed to write {path:?}: {source}")]
        WriteTemporaryFile {
            source: std::io::Error,
            path: PathBuf,
        },
        #[display("failed to read the streams of {path:?}: {source}")]
        ProbeInput {
            source: ffmpeg_the_third::Error,
            path: PathBuf,
        },
    } || ChapterMetadataError

    ChapterMetadataError := {
        #[display("failed to format chapter metadata: {0}")]
        FormatMetadata(std::fmt::Error),
    } || ChapterTimestampError

    ChapterTimestampError := {
        #[display("chapter duration {duration:?} exceeds the FFmpeg timestamp range")]
        TimestampOutOfRange { duration: Duration },
    }
}

/// Copies a video into a new file with chapters and/or an extra subtitle
/// stream added. Every existing stream is copied as-is; nothing is re-encoded
/// except the new subtitles, which are converted to a codec the container takes.
#[derive(Default)]
pub struct EmbedWriter {
    pub chapters: Option<ChapterLocations>,
    /// SRT text for the new subtitle stream.
    pub subtitles: Option<String>,
}

impl EmbedWriter {
    pub fn into_sidecar(
        self,
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
    ) -> Result<FfmpegCommand, EmbedWriterError> {
        let input = input.as_ref();
        let output = output.as_ref();
        let chapters = self
            .chapters
            .map(|chapters| write_temporary("chapters.ffmetadata", &chapters.into_ffmetadata()?))
            .transpose()?;
        let subtitles = self
            .subtitles
            .map(|subtitles| write_temporary("subtitles.srt", &subtitles))
            .transpose()?;

        // FFmpeg applies options to the file that follows them, so every input
        // goes first and every output option after.
        let mut command = FfmpegCommand::new();
        command.hide_banner().overwrite().input(input);
        for extra_input in chapters.iter().chain(&subtitles) {
            command.input(extra_input);
        }
        command
            // keep every stream, not just FFmpeg's default pick of one video and one audio
            .map("0")
            .args(["-map_metadata", "0"]) // copy the existing metadata information
            .args(["-c", "copy"]);

        let mut next_input = 1;
        if chapters.is_some() {
            // take chapters from the metadata file even if the input already has some
            command.args(["-map_chapters", &next_input.to_string()]);
            next_input += 1;
        }
        if subtitles.is_some() {
            // The new stream comes after the input's own subtitle streams.
            let index = count_subtitle_streams(input)?;
            command
                .map(format!("{next_input}:s"))
                .arg(format!("-c:s:{index}"))
                .arg(subtitle_codec(output))
                // mov_text otherwise gets a 1/1000000 time base, and the MP4
                // muxer gives its chapter track the LCM of all track time bases
                // (1/6000000 next to 1/16000 video and 1/48000 audio). Chapter
                // durations then overflow 32 bits after ~716s and the chapters
                // come out scrambled. SRT is millisecond-precise anyway.
                .arg(format!("-time_base:s:{index}"))
                .arg("1/1000");
        }

        command.output(output);
        Ok(command)
    }
}

/// A file in the temp directory, named per process so concurrent runs don't collide.
fn write_temporary(name: &str, contents: &str) -> Result<PathBuf, EmbedWriterError> {
    let path = std::env::temp_dir().join(format!("videosubextract-{}-{name}", std::process::id()));
    std::fs::write(&path, contents).map_err(|source| EmbedWriterError::WriteTemporaryFile {
        source,
        path: path.clone(),
    })?;
    Ok(path)
}

fn count_subtitle_streams(input: &Path) -> Result<usize, EmbedWriterError> {
    let probe = |source| EmbedWriterError::ProbeInput {
        source,
        path: input.to_path_buf(),
    };
    let context = ffmpeg_the_third::format::input(input).map_err(probe)?;
    Ok(context
        .streams()
        .filter(|stream| stream.parameters().medium() == media::Type::Subtitle)
        .count())
}

/// The text subtitle codec the output container accepts.
fn subtitle_codec(output: &Path) -> &'static str {
    let extension = output
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase());
    match extension.as_deref() {
        Some("mp4" | "m4v" | "mov") => "mov_text",
        Some("webm") => "webvtt",
        _ => "srt",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterLocations(pub Vec<ChapterLocation>);

impl ChapterLocations {
    pub fn into_ffmetadata(self) -> Result<String, ChapterMetadataError> {
        let mut output = String::new();
        writeln!(&mut output, ";FFMETADATA1")?;
        for chapter in self.0 {
            let start = duration_to_timestamp(chapter.start)?;
            let end = duration_to_timestamp(chapter.end)?;
            writeln!(&mut output, "[CHAPTER]")?;
            writeln!(&mut output, "TIMEBASE={CHAPTER_TIME_BASE}")?;
            writeln!(&mut output, "START={start}")?;
            writeln!(&mut output, "END={end}")?;
            writeln!(&mut output, "title={}", escape_metadata(&chapter.name))?;
        }
        Ok(output)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterLocation {
    pub start: Duration,
    pub end: Duration,
    pub name: String,
}

/// FFmetadata treats `=`, `;`, `#`, `\` and newlines as syntax; they must be backslash-escaped.
fn escape_metadata(value: &str) -> String {
    value
        .chars()
        .fold(String::with_capacity(value.len()), |mut out, c| {
            if matches!(c, '=' | ';' | '#' | '\\' | '\n') {
                out.push('\\');
            }
            out.push(c);
            out
        })
}

/// Convert to chapter ticks, rounding to the nearest millisecond (ties up).
fn duration_to_timestamp(duration: Duration) -> Result<i64, ChapterTimestampError> {
    // Rescale only the fractional second so total nanoseconds don't have to
    // fit in i64; this preserves the full range of millisecond timestamps.
    let fractional_ms = duration
        .subsec_nanos()
        .rescale(Rational(1, 1_000_000_000), CHAPTER_TIME_BASE);

    i64::try_from(duration.as_secs())
        .ok()
        .and_then(|seconds| seconds.checked_mul(1000))
        .and_then(|whole_ms| whole_ms.checked_add(fractional_ms))
        .ok_or(ChapterTimestampError::TimestampOutOfRange { duration })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_durations_using_ffmpeg_rounding() {
        for (duration, expected) in [
            (Duration::ZERO, 0),
            (Duration::from_millis(1_234), 1_234),
            (Duration::from_nanos(499_999), 0),
            (Duration::from_nanos(500_000), 1),
            (Duration::from_nanos(1_234_499_999), 1_234),
            (Duration::from_nanos(1_234_500_000), 1_235),
            (Duration::from_nanos(999_500_000), 1_000),
        ] {
            assert_eq!(duration_to_timestamp(duration).unwrap(), expected);
        }
    }

    #[test]
    fn checks_the_full_millisecond_timestamp_range() {
        let maximum = Duration::from_millis(i64::MAX as u64);
        assert_eq!(duration_to_timestamp(maximum).unwrap(), i64::MAX);
        assert_eq!(
            duration_to_timestamp(maximum + Duration::from_nanos(499_999)).unwrap(),
            i64::MAX,
        );

        for duration in [
            maximum + Duration::from_nanos(500_000),
            maximum + Duration::from_millis(1),
            Duration::from_secs(i64::MAX as u64),
            Duration::MAX,
        ] {
            assert!(matches!(
                duration_to_timestamp(duration),
                Err(ChapterTimestampError::TimestampOutOfRange { duration: actual })
                    if actual == duration,
            ));
        }
    }

    #[test]
    fn writes_chapter_timestamps_in_the_declared_time_base() {
        let metadata = ChapterLocations(vec![ChapterLocation {
            start: Duration::from_nanos(1_234_500_000),
            end: Duration::from_millis(5_678),
            name: "First chapter".into(),
        }])
        .into_ffmetadata()
        .unwrap();

        assert_eq!(
            metadata,
            ";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=1235\nEND=5678\ntitle=First chapter\n",
        );
    }

    #[test]
    fn escapes_metadata_syntax_in_titles() {
        assert_eq!(escape_metadata("a=b;c#d\\e\nf"), "a\\=b\\;c\\#d\\\\e\\\nf");
    }
}
