#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("clipboard item contains no representations")]
    EmptyItem,
    #[error("clipboard payload contains no items")]
    EmptyPayload,
    #[error("failed to generate random device id: {0}")]
    DeviceId(#[from] getrandom::Error),
}
