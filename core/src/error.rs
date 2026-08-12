#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Db(String),
    #[error("identity error: {0}")]
    Identity(String),
}
