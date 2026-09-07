// SPDX-License-Identifier: MPL-2.0

use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};
use serde::{Deserialize, Serialize};

use crate::fl;
use crate::native_video_sub_finder::NativeSearchParams;
use crate::ocr::{self, OcrModel};

#[derive(Debug, Default, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
pub enum Language {
    #[default]
    English,
    ZhTw,
}

impl Language {
    pub const ALL: [Self; 2] = [Self::English, Self::ZhTw];

    pub fn labels() -> Vec<String> {
        vec![fl!("language-english"), fl!("language-zh-tw")]
    }

    pub const fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::ZhTw => "zh-TW",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum ProcessingResolution {
    Hd720,
    #[default]
    FullHd1080,
    UltraHd4k,
    None,
}

impl ProcessingResolution {
    pub const ALL: [Self; 4] = [Self::Hd720, Self::FullHd1080, Self::UltraHd4k, Self::None];
    pub fn labels() -> Vec<String> {
        vec![
            fl!("resolution-720p"),
            fl!("resolution-1080p"),
            fl!("resolution-4k"),
            fl!("resolution-none"),
        ]
    }

    pub const fn max_height(self) -> Option<u32> {
        match self {
            Self::Hd720 => Some(720),
            Self::FullHd1080 => Some(1080),
            Self::UltraHd4k => Some(2160),
            Self::None => None,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
pub enum SubtitleDetector {
    #[default]
    OriginalCpp,
    RustRewrite,
}

impl SubtitleDetector {
    pub const ALL: [Self; 2] = [Self::OriginalCpp, Self::RustRewrite];
    pub fn labels() -> Vec<String> {
        vec![fl!("original-cpp"), fl!("rust-rewrite-wip")]
    }
}

#[derive(Debug, Clone, CosmicConfigEntry, PartialEq)]
#[version = 7]
pub struct Config {
    pub language: Language,
    pub ocr_model: OcrModel,
    pub custom_ocrs: Vec<ocr::plugin_loader::DynamicLibrary>,
    pub subtitle_detector: SubtitleDetector,
    pub native_search_params: NativeSearchParams,
    pub post_ocr_processing: bool,
    pub processing_resolution: ProcessingResolution,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: Language::default(),
            ocr_model: OcrModel::default(),
            custom_ocrs: Vec::new(),
            subtitle_detector: SubtitleDetector::default(),
            native_search_params: NativeSearchParams::default(),
            post_ocr_processing: true,
            processing_resolution: ProcessingResolution::default(),
        }
    }
}
