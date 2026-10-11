use std::{fmt, io};

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug)]
pub enum AppError {
    InvalidInput { location: String, message: String },
    SizeLimit { limit: usize },
    Cancelled,
    Io(io::Error),
    Conflict(String),
    UnsupportedPublication(String),
    IdentityExhausted,
}

impl AppError {
    pub fn invalid(location: impl Into<String>, message: impl Into<String>) -> Self {
        Self::InvalidInput {
            location: location.into(),
            message: message.into(),
        }
    }
}
impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput { location, message } if !location.is_empty() => {
                write!(f, "{location}: {message}")
            }
            Self::InvalidInput { message, .. } => f.write_str(message),
            Self::SizeLimit { limit } => {
                write!(f, "The operation exceeds its {limit}-byte or cell limit.")
            }
            Self::Cancelled => f.write_str("Operation cancelled."),
            Self::Io(error) => write!(f, "File operation failed: {error}"),
            Self::Conflict(message) | Self::UnsupportedPublication(message) => f.write_str(message),
            Self::IdentityExhausted => f.write_str("No further identities are available."),
        }
    }
}
impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}
impl From<io::Error> for AppError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
