use continuum_core::{ClipItem, CoreError};

#[derive(Debug, thiserror::Error)]
pub enum ClipboardError {
    #[error("clipboard backend is not available on this system")]
    Unavailable,
    #[error("failed to read clipboard: {0}")]
    Read(String),
    #[error("failed to write clipboard: {0}")]
    Write(String),
    #[error(transparent)]
    Core(#[from] CoreError),
}

pub trait ClipboardBackend: Send {
    fn read(&mut self) -> Result<Option<ClipItem>, ClipboardError>;
    fn write(&mut self, item: &ClipItem) -> Result<(), ClipboardError>;
}
