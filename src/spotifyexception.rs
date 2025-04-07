use thiserror::Error;

#[derive(Error, Debug)]
pub enum SpotifyException {
    #[error("Spotify API error: {0}")]
    ApiError(String),
    
    #[error("HTTP request error: {0}")]
    RequestError(#[from] reqwest::Error),
    
    #[error("JSON parsing error: {0}")]
    JsonError(#[from] serde_json::Error),
    
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    
    #[error("URL encoding error: {0}")]
    UrlEncodedError(#[from] serde_urlencoded::ser::Error),
    
    #[error("{0}")]
    Generic(String),
}

impl SpotifyException {
    pub fn new<S: Into<String>>(message: S) -> Self {
        SpotifyException::Generic(message.into())
    }
}