#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("identity error: {0}")]
    Identity(String),
    #[error("noise error: {0}")]
    Noise(#[from] snow::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("frame too large: {0} bytes")]
    FrameTooLarge(u32),
    #[error("codec error: {0}")]
    Codec(String),
    #[error("peer presented an unexpected static key")]
    UnknownPeer,
    #[error("peer connection timed out")]
    TimedOut,
}
