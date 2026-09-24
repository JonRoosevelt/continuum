use continuum_core::ClipItem;

#[derive(Debug, thiserror::Error)]
pub enum ClipboardError {
    #[error("clipboard backend is not available on this system")]
    Unavailable,
    #[error("failed to read clipboard: {0}")]
    Read(String),
    #[error("failed to write clipboard: {0}")]
    Write(String),
    #[error(transparent)]
    Core(#[from] continuum_core::CoreError),
}

pub trait ClipboardBackend: Send {
    /// Returns `Some` only when the clipboard changed since the previous call.
    fn read(&mut self) -> Result<Option<Vec<ClipItem>>, ClipboardError>;
    fn write(&mut self, items: &[ClipItem]) -> Result<(), ClipboardError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessBehavior {
    Default,
    Ask,
    AlwaysAllow,
    AlwaysDeny,
}
