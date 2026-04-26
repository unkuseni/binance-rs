use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Deserialize)]
pub struct BinanceContentError {
    pub code: i32,
    pub msg: String,
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Binance error: {0:?}")]
    BinanceError(BinanceContentError),

    #[error("Kline value missing: {name} at index {index}")]
    KlineValueMissingError { index: usize, name: &'static str },

    #[error("{0}")]
    Msg(String),

    #[error("Request error: {0}")]
    ReqError(#[from] reqwest::Error),

    #[error("Invalid header: {0}")]
    InvalidHeaderError(#[from] reqwest::header::InvalidHeaderValue),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Parse float error: {0}")]
    ParseFloatError(#[from] std::num::ParseFloatError),

    #[error("URL parse error: {0}")]
    UrlParserError(#[from] url::ParseError),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("WebSocket error: {0}")]
    Tungstenite(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("Timestamp error: {0}")]
    TimestampError(#[from] std::time::SystemTimeError),
}
