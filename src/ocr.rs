use eyre::Result;
use image::DynamicImage;
use manganis::{Asset, asset};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use vse_ui::Apply;

pub mod plugin_loader;

use crate::config::Config;
use crate::fl;

static PADDLE_OCR_V6_MEDIUM: OnceLock<Result<ocr_rs::OcrEngine>> = OnceLock::new();
static PADDLE_OCR_V5_MOBILE: OnceLock<Result<ocr_rs::OcrEngine>> = OnceLock::new();

const PADDLE_OCR_V6_MEDIUM_DET: Asset = asset!("/models/PP-OCRv6_medium_det.mnn");
const PADDLE_OCR_V6_MEDIUM_REC: Asset = asset!("/models/PP-OCRv6_medium_rec.mnn");
const PADDLE_OCR_V6_MEDIUM_KEYS: Asset = asset!("/models/ppocr_keys_v6_medium.txt");
const PADDLE_OCR_V5_MOBILE_DET: Asset = asset!("/models/PP-OCRv5_mobile_det.mnn");
const PADDLE_OCR_V5_MOBILE_REC: Asset = asset!("/models/PP-OCRv5_mobile_rec.mnn");
const PADDLE_OCR_V5_MOBILE_KEYS: Asset = asset!("/models/ppocr_keys_v5.txt");

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
        vec![fl!("paddleocr-v6-medium"), fl!("paddleocr-v5-mobile")].apply(|mut models| {
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
        let create_engine = |detection_model: Asset, recognition_model: Asset, charset: Asset| {
            let detection_model =
                std::fs::read(dioxus_asset_resolver::asset_path(detection_model)?)?;
            let recognition_model =
                std::fs::read(dioxus_asset_resolver::asset_path(recognition_model)?)?;
            let charset = std::fs::read(dioxus_asset_resolver::asset_path(charset)?)?;

            Ok(ocr_rs::OcrEngine::from_bytes(
                &detection_model,
                &recognition_model,
                &charset,
                Some(ocr_rs::OcrEngineConfig {
                    det_options: ocr_rs::DetOptions::default(),
                    rec_options: ocr_rs::RecOptions::default(),
                    enable_parallel: true,
                    backend: ocr_rs::Backend::CPU,
                    ..Default::default()
                }),
            )?)
        };

        let engine = match self {
            Self::V6Medium => PADDLE_OCR_V6_MEDIUM.get_or_init(|| {
                create_engine(
                    PADDLE_OCR_V6_MEDIUM_DET,
                    PADDLE_OCR_V6_MEDIUM_REC,
                    PADDLE_OCR_V6_MEDIUM_KEYS,
                )
            }),
            Self::V5Mobile => PADDLE_OCR_V5_MOBILE.get_or_init(|| {
                create_engine(
                    PADDLE_OCR_V5_MOBILE_DET,
                    PADDLE_OCR_V5_MOBILE_REC,
                    PADDLE_OCR_V5_MOBILE_KEYS,
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
