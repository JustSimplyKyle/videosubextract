// SPDX-License-Identifier: MPL-2.0

use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};
use serde::{Deserialize, Serialize};

use crate::native_video_sub_finder::NativeSearchParams;
use crate::ocr::OcrModel;

#[derive(Debug, Default, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
pub enum SubtitleDetector {
    #[default]
    OriginalCpp,
    RustRewrite,
}

impl SubtitleDetector {
    pub const ALL: [Self; 2] = [Self::OriginalCpp, Self::RustRewrite];
    pub const LABELS: [&'static str; 2] = ["Original C++", "Rust rewrite"];
}

#[derive(Debug, Clone, CosmicConfigEntry, PartialEq)]
#[version = 3]
pub struct Config {
    pub ocr_model: OcrModel,
    pub subtitle_detector: SubtitleDetector,
    pub native_search_params: NativeSearchParams,
    pub post_ocr_processing: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ocr_model: OcrModel::default(),
            subtitle_detector: SubtitleDetector::default(),
            native_search_params: NativeSearchParams::default(),
            post_ocr_processing: true,
        }
    }
}
