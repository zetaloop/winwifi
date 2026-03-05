use std::convert::Infallible;

use thiserror::Error;
use winio::prelude::*;

use crate::wifi::native::WifiError;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("UI error: {0}")]
    Ui(#[from] winio::Error),
    #[error("Layout error: {0}")]
    Layout(#[from] TaffyError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("WiFi error: {0}")]
    Wifi(#[from] WifiError),
}

impl<E: Into<AppError> + std::fmt::Display> From<LayoutError<E>> for AppError {
    fn from(value: LayoutError<E>) -> Self {
        match value {
            LayoutError::Taffy(err) => Self::Layout(err),
            LayoutError::Child(err) => err.into(),
            _ => Self::Io(std::io::Error::other(value.to_string())),
        }
    }
}

impl From<Infallible> for AppError {
    fn from(value: Infallible) -> Self {
        match value {}
    }
}

pub type AppResult<T> = std::result::Result<T, AppError>;
