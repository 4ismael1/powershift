use std::fmt::{Display, Formatter};
use std::io;

pub type PowerResult<T> = Result<T, PowerError>;

#[derive(Debug)]
pub enum PowerError {
    Io(io::Error),
    CommandFailed {
        command: &'static str,
        code: Option<i32>,
        stderr: String,
    },
    Parse(String),
    InvalidGuid(String),
    WindowsApi {
        function: &'static str,
        code: u32,
    },
    NotSupported(&'static str),
}

impl Display for PowerError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            PowerError::Io(err) => write!(f, "I/O error: {err}"),
            PowerError::CommandFailed {
                command,
                code,
                stderr,
            } => write!(
                f,
                "{command} failed with code {:?}: {}",
                code,
                stderr.trim()
            ),
            PowerError::Parse(message) => write!(f, "Parse error: {message}"),
            PowerError::InvalidGuid(guid) => write!(f, "Invalid GUID: {guid}"),
            PowerError::WindowsApi { function, code } => {
                write!(f, "{function} failed with Win32 error {code}")
            }
            PowerError::NotSupported(feature) => write!(f, "{feature} is not supported here"),
        }
    }
}

impl std::error::Error for PowerError {}

impl From<io::Error> for PowerError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}
