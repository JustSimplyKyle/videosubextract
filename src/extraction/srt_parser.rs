use std::str::FromStr;

use pest::Parser;
use pest_derive::Parser;

use super::Subtitle;

use std::time::Duration;

pub struct Subtitles(Vec<Subtitle>);

struct SrtDuration(Duration);

#[derive(Parser)]
#[grammar = "extraction/srt.pest"]
struct SrtParser;

impl FromStr for SrtDuration {
    type Err = SrtTimestampParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let [hours, minutes, seconds_with_ms] = s.split(':').collect::<Vec<_>>()[..] else {
            return Err(SrtTimestampParseError::MissingColonSeparator);
        };
        let [seconds, milliseconds] = seconds_with_ms.split(',').collect::<Vec<_>>()[..] else {
            return Err(SrtTimestampParseError::MissingCommaSeparator);
        };

        let hours = hours.parse()?;
        let minutes = minutes.parse()?;
        let seconds = seconds.parse()?;
        let milliseconds = milliseconds.parse()?;

        if minutes >= 60 || seconds >= 60 || milliseconds >= 1_000 {
            return Err(SrtTimestampParseError::InvalidComponent);
        }

        let hours = Duration::from_hours(hours);
        let minutes = Duration::from_mins(minutes);
        let seconds = Duration::from_secs(seconds);
        let milliseconds = Duration::from_millis(milliseconds);

        let duration = || {
            hours
                .checked_add(minutes)?
                .checked_add(seconds)?
                .checked_add(milliseconds)
        };

        Ok(Self(duration().ok_or(SrtTimestampParseError::Overflow)?))
    }
}

error_set::error_set! {
    SubtitleParseError := SrtGrammarError || SrtBlockParseError

    SrtGrammarError := {
        #[display("failed to parse SRT input according to the `file` grammar rule")]
        InvalidFile(pest::error::Error<Rule>),
    }
    SrtBlockParseError := {
        #[display("SRT block is missing required rule {rule:?}")]
        MissingRule{rule: Rule},
        #[display("Invalid timing length, expected 2 got {got}")]
        TimingLengthMismatch{got: usize},
    } || SrtTimestampParseError

    SrtTimestampParseError := {
        #[display("SRT timestamp is missing `:` separators")]
        MissingColonSeparator,
        #[display("SRT timestamp is missing a `,` separator")]
        MissingCommaSeparator,
        #[display("SRT timestamp contains invalid components")]
        InvalidComponent,
        #[display("SRT timestamp can't be expressed in rust duration")]
        Overflow,
        InvalidNumber(std::num::ParseIntError),
    }
}

impl Subtitles {
    pub fn new(input: &str) -> Result<Self, SubtitleParseError> {
        let mut parser =
            SrtParser::parse(Rule::file, input).map_err(SrtGrammarError::InvalidFile)?;
        let subtitles = parser
            .next()
            .expect("the `file` rule always produces a pair")
            .into_inner()
            .filter(|pair| matches!(pair.as_rule(), Rule::block))
            .map(Self::parse_block)
            .collect::<Result<_, _>>();

        Ok(Self(subtitles?))
    }

    fn parse_block(block: pest::iterators::Pair<'_, Rule>) -> Result<Subtitle, SrtBlockParseError> {
        let mut cursor = block.into_inner();

        let _index = cursor
            .next()
            .ok_or(SrtBlockParseError::MissingRule { rule: Rule::index })?;

        let timing = cursor
            .next()
            .ok_or(SrtBlockParseError::MissingRule { rule: Rule::timing })?
            .into_inner()
            .map(|x| {
                let duration = x.as_str().parse::<SrtDuration>()?;
                Ok(duration.0)
            })
            .collect::<Result<Vec<_>, SrtBlockParseError>>()?;

        if timing.len() != 2 {
            return Err(SrtBlockParseError::TimingLengthMismatch { got: timing.len() });
        }

        let texts = cursor.map(|x| x.as_str()).collect::<Vec<_>>();

        if texts.is_empty() {
            return Err(SrtBlockParseError::MissingRule {
                rule: Rule::text_line,
            });
        }

        Ok(Subtitle {
            start_timestamp: timing[0],
            end_timestamp: timing[1],
            text: texts.join("\n"),
        })
    }
}

mod tests {
    use std::time::Duration;

    use crate::extraction::{Subtitle, srt_parser::Subtitles};

    #[test]
    fn parses_srt_subtitles() {
        let srt = "1\n00:00:01,234 --> 00:01:05,678\nFirst line\nSecond line\n\n2\n01:02:03,004 --> 01:02:05,006\nAnother subtitle\n";

        let Subtitles(subtitles) = Subtitles::new(srt).unwrap();

        assert_eq!(
            subtitles,
            vec![
                Subtitle::new(
                    Duration::from_millis(1_234),
                    Duration::from_millis(65_678),
                    "First line\nSecond line".into(),
                ),
                Subtitle::new(
                    Duration::from_millis(3_723_004),
                    Duration::from_millis(3_725_006),
                    "Another subtitle".into(),
                ),
            ]
        );
    }
}
