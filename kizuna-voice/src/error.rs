use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Gateway error: {0}")]
    Gateway(String),
    
    #[error("Transport error: {0}")]
    Transport(String),
    
    #[error("Crypto/DAVE error: {0}")]
    Crypto(String),
    
    #[error("Connection error: {0}")]
    Connection(String),
}

pub type Result<T> = std::result::Result<T, Error>;
