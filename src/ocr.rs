use cosmic::Apply;
use eyre::Result;
use image::DynamicImage;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

pub mod plugin_loader;

use crate::config::Config;

static PADDLE_OCR_V6_MEDIUM: OnceLock<Result<ocr_rs::OcrEngine>> = OnceLock::new();
static PADDLE_OCR_V5_MOBILE: OnceLock<Result<ocr_rs::OcrEngine>> = OnceLock::new();

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub enum OcrModel {
    PaddleOcr(PaddleOcr),
    Custom(plugin_loader::DynamicLibrary),
}

impl OcrModel {
    pub fn all(config: &Config) -> Vec<Self> {
        vec![
            Self::PaddleOcr(PaddleOcr::V6Medium),
            Self::PaddleOcr(PaddleOcr::V5Mobile),
        ]
        .apply(|mut models| {
            let custom_models = config.custom_ocrs.iter().map(|x| Self::Custom(x.clone()));

            models.extend(custom_models);

            models
        })
    }
    pub fn labels(config: &Config) -> Vec<String> {
        vec!["PaddleOCR v6 Medium".into(), "PaddleOCR v5 Mobile".into()].apply(|mut models| {
            let custom_model_names = config.custom_ocrs.iter().map(|x| x.name.clone());

            models.extend(custom_model_names);

            models
        })
    }
}

impl Default for OcrModel {
    fn default() -> Self {
        Self::PaddleOcr(PaddleOcr::V6Medium)
    }
}

#[derive(Debug, Default, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
pub enum PaddleOcr {
    #[default]
    V6Medium,
    V5Mobile,
}

impl PaddleOcr {
    fn engine(self) -> Result<&'static ocr_rs::OcrEngine> {
        let create_engine = |detection_model, recognition_model, charset| {
            ocr_rs::OcrEngine::from_bytes(
                detection_model,
                recognition_model,
                charset,
                Some(ocr_rs::OcrEngineConfig {
                    det_options: ocr_rs::DetOptions::default(),
                    rec_options: ocr_rs::RecOptions::default(),
                    enable_parallel: true,
                    backend: ocr_rs::Backend::CPU,
                    ..Default::default()
                }),
            )
            .map_err(Into::into)
        };

        let engine = match self {
            Self::V6Medium => PADDLE_OCR_V6_MEDIUM.get_or_init(|| {
                create_engine(
                    include_bytes!("../models/PP-OCRv6_medium_det.mnn"),
                    include_bytes!("../models/PP-OCRv6_medium_rec.mnn"),
                    include_bytes!("../models/ppocr_keys_v6_medium.txt"),
                )
            }),
            Self::V5Mobile => PADDLE_OCR_V5_MOBILE.get_or_init(|| {
                create_engine(
                    include_bytes!("../models/PP-OCRv5_mobile_det.mnn"),
                    include_bytes!("../models/PP-OCRv5_mobile_rec.mnn"),
                    include_bytes!("../models/ppocr_keys_v5.txt"),
                )
            }),
        };

        engine.as_ref().map_err(|error| eyre::eyre!(error))
    }
}

pub trait OcrProvider: Send + Sync {
    fn recognize_text(&self, image: &DynamicImage) -> Result<String>;
}

impl OcrProvider for OcrModel {
    fn recognize_text(&self, image: &DynamicImage) -> Result<String> {
        match self {
            Self::PaddleOcr(paddle_ocr) => Ok(paddle_ocr
                .engine()?
                .recognize(image)?
                .iter()
                .map(|result| result.text.as_str())
                .collect::<Vec<_>>()
                .join("\n")),
            Self::Custom(library) => library.recognize_text(image),
        }
    }
}
