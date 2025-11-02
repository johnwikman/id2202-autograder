use std::convert::From;
use toml;

#[derive(Debug)]
pub enum Error {
    IOError(std::io::Error),
    SetLoggerError(log::SetLoggerError),
    TOMLDeError(toml::de::Error),
    TOMLSerError(toml::ser::Error),
    DieselConnectionError(diesel::ConnectionError),
    DieselError(diesel::result::Error),
    ReqwestError(reqwest::Error),
    PopenError(subprocess::PopenError),
    SyscommandTimeoutError {
        stdout: Option<String>,
        stderr: Option<String>,
    },
    SyscommandOutputLimitExceededError(usize),
    RawError(String),
}

impl std::error::Error for Error {}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::IOError(e) => {
                write!(f, "[IOError] {e}")
            }
            Error::SetLoggerError(e) => {
                write!(f, "[SetLoggerError] {e}")
            }
            Error::TOMLDeError(e) => {
                write!(f, "[TOMLDeError] {e}")
            }
            Error::TOMLSerError(e) => {
                write!(f, "[TOMLSerError] {e}")
            }
            Error::DieselConnectionError(e) => {
                write!(f, "[DieselConnectionError] {e}")
            }
            Error::DieselError(e) => {
                write!(f, "[DieselError] {e}")
            }
            Error::ReqwestError(e) => {
                write!(f, "[ReqwestError] {e}")
            }
            Error::PopenError(e) => {
                write!(f, "[PopenError] {e}")
            }
            Error::SyscommandTimeoutError { .. } => {
                write!(f, "[SyscommandTimeoutError]")
            }
            Error::SyscommandOutputLimitExceededError(s) => {
                write!(f, "[SyscommandOutputLimitExceededError] limit: {s}")
            }
            Error::RawError(s) => {
                write!(f, "{s}")
            }
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::IOError(e)
    }
}

impl From<log::SetLoggerError> for Error {
    fn from(e: log::SetLoggerError) -> Self {
        Error::SetLoggerError(e)
    }
}

impl From<fern::InitError> for Error {
    fn from(e: fern::InitError) -> Self {
        match e {
            fern::InitError::Io(e) => Self::from(e),
            fern::InitError::SetLoggerError(e) => Self::from(e),
        }
    }
}

impl From<toml::de::Error> for Error {
    fn from(e: toml::de::Error) -> Self {
        Error::TOMLDeError(e)
    }
}

impl From<toml::ser::Error> for Error {
    fn from(e: toml::ser::Error) -> Self {
        Error::TOMLSerError(e)
    }
}

impl From<diesel::ConnectionError> for Error {
    fn from(e: diesel::ConnectionError) -> Self {
        Error::DieselConnectionError(e)
    }
}

impl From<diesel::result::Error> for Error {
    fn from(e: diesel::result::Error) -> Self {
        Error::DieselError(e)
    }
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::ReqwestError(e)
    }
}

impl From<subprocess::PopenError> for Error {
    fn from(e: subprocess::PopenError) -> Self {
        Error::PopenError(e)
    }
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error::RawError(s)
    }
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error::RawError(String::from(s))
    }
}

impl Error {
    pub fn err_string<T>(s: String) -> Result<T, Self> {
        Err(Self::from(s))
    }
    pub fn err_str<T>(s: &str) -> Result<T, Self> {
        Err(Self::from(s))
    }
}
