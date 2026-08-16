#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Db(String),
    #[error("directory error: {0}")]
    Directory(String),
    #[error("identity error: {0}")]
    Identity(String),
    #[error("jikan error: {0}")]
    Jikan(String),
    #[error("keychain error: {0}")]
    Keychain(String),
    #[error("letterboxd error: {0}")]
    Letterboxd(String),
    #[error("network error: {0}")]
    Net(String),
    #[error("nyaa error: {0}")]
    Nyaa(String),
    #[error("stremio error: {0}")]
    Stremio(String),
}
