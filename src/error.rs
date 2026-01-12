use paste::paste;

#[derive(Debug)]
pub struct Error {
    pub kind: ErrorKind,
    pub cause: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl Error {
    /// Associates a cause with the error, overwriting any existing cause.
    pub fn with_cause(self, cause: Box<dyn std::error::Error + Send + Sync>) -> Self {
        Self {
            kind: self.kind,
            cause: Some(cause),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause.as_ref().map(|e| e.as_ref() as _)
    }
}

/// Error type for system command execution failures with metadata.
#[derive(Debug, Clone)]
pub struct SyscommandError {
    pub cmd: Vec<String>,
    pub msg: Option<String>,
    pub timeout: Option<std::time::Duration>,

    /// `(received, expected)`
    pub code_mismatch: Option<(i32, i32)>,
    pub output_limit_exceeded: Option<usize>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
}

impl SyscommandError {
    pub fn new(cmd: Vec<String>) -> Self {
        Self {
            cmd: cmd,
            msg: None,
            timeout: None,
            code_mismatch: None,
            output_limit_exceeded: None,
            stdout: None,
            stderr: None,
        }
    }

    pub fn msg(mut self, msg: impl Into<String>) -> Self {
        self.msg = Some(msg.into());
        self
    }

    pub fn timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn code_mismatch(mut self, received: i32, expected: i32) -> Self {
        self.code_mismatch = Some((received, expected));
        self
    }

    pub fn limit_exceeded(mut self, limit: usize) -> Self {
        self.output_limit_exceeded = Some(limit);
        self
    }

    pub fn stdout(mut self, stdout: impl Into<String>) -> Self {
        self.stdout = Some(stdout.into());
        self
    }

    pub fn stderr(mut self, stderr: impl Into<String>) -> Self {
        self.stderr = Some(stderr.into());
        self
    }

    /// Used for when `.into()` is not powerful enough to infer the type.
    pub fn as_error(self) -> Error {
        self.into()
    }
}

/// Error type for test configuration parsing with traceability metadata.
#[derive(Debug, Clone)]
pub struct TestConfigError {
    pub msg: String,
    pub path: Option<String>,
    pub title: Option<String>,
    pub key: Option<String>,
    pub kind: Option<String>,
    pub tag: Option<String>,
}

impl TestConfigError {
    pub fn new() -> Self {
        Self {
            msg: "".to_string(),
            path: None,
            title: None,
            key: None,
            kind: None,
            tag: None,
        }
    }
    pub fn new_msg(msg: impl Into<String>) -> Self {
        Self::new().msg(msg)
    }

    pub fn msg(mut self, msg: impl Into<String>) -> Self {
        self.msg = msg.into();
        self
    }

    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn kind(mut self, kind: impl Into<String>) -> Self {
        self.kind = Some(kind.into());
        self
    }

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// Used for when `.into()` is not powerful enough to infer the type.
    pub fn as_error(self) -> Error {
        self.into()
    }
}

/// Macro used to define error kinds and their auxiliary functions.
macro_rules! define_error_kinds {
    (
        $(
            $kind:ident { $($field:ident : $ty:ty),* $(,)? }
        ),* $(,)?
    ) => {
        #[derive(Debug)]
        pub enum ErrorKind {
            $(
                $kind { $($field: $ty),* },
            )*
            TestConfig(TestConfigError),
            Syscommand(SyscommandError),
        }

        paste! {
            impl Error {
                $(
                    pub fn [<$kind:snake>]($($field: impl Into<$ty>),*) -> Self {
                        Self {
                            kind: ErrorKind::$kind { $($field: $field.into()),* },
                            cause: None,
                        }
                    }
                    pub fn [<err_ $kind:snake>]<T>($($field: impl Into<$ty>),*) -> Result<T, Self> {
                        Err(Self {
                            kind: ErrorKind::$kind { $($field: $field.into()),* },
                            cause: None,
                        })
                    }
                    pub fn [<errcause_ $kind:snake>]<T>($($field: impl Into<$ty>),*, cause: Box<dyn std::error::Error + Send + Sync>) -> Result<T, Self> {
                        Err(Self {
                            kind: ErrorKind::$kind { $($field: $field.into()),* },
                            cause: Some(cause),
                        })
                    }
                )*
            }
        }
    };
}

define_error_kinds! {
    LoadConfig {file: String},
    ParseType { type_name: String, value: String },
    Fs { msg: String, path: String },
    Auto { msg: String },
    Convert { msg: String },
    Runtime { msg: String },
    Format { msg: String, value: String },
    Identifier { got: String, expected: Vec<String> },
    HttpResponse { msg: String, code: u16, text: String },
}

// Add these manually for the predefined TestConfigError and SyscommandError in the macro
impl Error {
    pub fn test_config() -> TestConfigError {
        TestConfigError::new()
    }
    pub fn test_config_msg(s: impl Into<String>) -> TestConfigError {
        TestConfigError::new_msg(s)
    }

    pub fn syscommand(cmd: Vec<String>) -> SyscommandError {
        SyscommandError::new(cmd)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.alternate() {
            write!(f, "{}", self)?;
            let mut source = std::error::Error::source(self);

            let mut i = 0;
            while let Some(cause) = source {
                // Avoid recursing too deep
                i += 1;
                if i > 10 {
                    break;
                }
                write!(f, "\nCaused by: {cause}")?;
                source = cause.source();
            }
            return Ok(());
        }
        match &self.kind {
            ErrorKind::LoadConfig { file } => {
                write!(f, "error loading config from file {}", file)
            }
            ErrorKind::ParseType { type_name, value } => {
                write!(f, "error parsing \"{}\" as type {}", value, type_name)
            }
            ErrorKind::Fs { msg, path } => {
                write!(f, "{}: {}", msg, path)
            }
            ErrorKind::Auto { msg } => {
                write!(f, "auto converted: {}", msg)
            }
            ErrorKind::Convert { msg } => {
                write!(f, "conversion error: {}", msg)
            }
            ErrorKind::Runtime { msg } => {
                write!(f, "runtime (internal) error: {}", msg)
            }
            ErrorKind::Format { msg, value } => {
                write!(f, "format error \"{}\" for value \"{}\"", msg, value)
            }
            ErrorKind::Identifier { got, expected } => {
                write!(
                    f,
                    "got identifier {}, expected one of: {}",
                    got,
                    expected.join(", ")
                )
            }
            ErrorKind::HttpResponse { msg, code, text } => {
                write!(
                    f,
                    "http response error {} with code {}: {}",
                    msg, code, text
                )
            }
            ErrorKind::Syscommand(e) => {
                write!(f, "syscommand error on {:?}", e.cmd)?;
                if let Some(msg) = &e.msg {
                    write!(f, "\n  with message: {}", msg)?;
                }
                if let Some(timeout) = &e.timeout {
                    write!(f, "\n  timeout: {:?}", timeout)?;
                }
                if let Some((received, expected)) = &e.code_mismatch {
                    write!(
                        f,
                        "\n  expected code {}, received code {}",
                        expected, received
                    )?;
                }
                if let Some(limit) = &e.output_limit_exceeded {
                    write!(f, "\n  exceeded output limit of {} bytes", limit)?;
                }
                if let Some(stdout) = &e.stdout {
                    write!(f, "\n  stdout: \"\"\"\n{}\n\"\"\"", stdout)?;
                }
                if let Some(stderr) = &e.stderr {
                    write!(f, "\n  stderr: \"\"\"\n{}\n\"\"\"", stderr)?;
                }
                Ok(())
            }
            ErrorKind::TestConfig(e) => {
                write!(f, "test config error: {}", e.msg)?;
                if let Some(path) = &e.path {
                    write!(f, "\n  path: {}", path)?;
                }
                if let Some(title) = &e.title {
                    write!(f, "\n  title: {}", title)?;
                }
                if let Some(key) = &e.key {
                    write!(f, "\n  key: {}", key)?;
                }
                if let Some(kind) = &e.kind {
                    write!(f, "\n  kind: {}", kind)?;
                }
                if let Some(tag) = &e.tag {
                    write!(f, "\n  tag: {}", tag)?;
                }
                Ok(())
            }
        }
    }
}

/// Define auto conversions from other error types into this error type, with a
/// default message.
macro_rules! auto_convert_error {
    ($ty:ty, $msg:literal) => {
        impl From<$ty> for Error {
            fn from(e: $ty) -> Self {
                Error::auto($msg).with_cause(Box::new(e))
            }
        }
    };
}

auto_convert_error!(diesel::ConnectionError, "diesel connection error");
auto_convert_error!(diesel::result::Error, "diesel result error");
auto_convert_error!(log::SetLoggerError, "set logger error");
auto_convert_error!(postgres::Error, "postgres error");
auto_convert_error!(reqwest::Error, "reqwest error");
auto_convert_error!(serde_json::Error, "serde JSON error");
auto_convert_error!(std::io::Error, "IO error");
auto_convert_error!(std::time::SystemTimeError, "system time error");
auto_convert_error!(toml::ser::Error, "toml serialization error");
auto_convert_error!(toml::de::Error, "toml deserialization error");
auto_convert_error!(subprocess::PopenError, "subprocess popen error");

impl Error {
    /// Converts the error `e` into this error kind and specifies a custom
    /// message `msg`.
    pub fn auto_msg(msg: impl Into<String>, e: impl Into<Error>) -> Self {
        let mut err: Error = e.into();
        match err.kind {
            ErrorKind::Auto { .. } => err.kind = ErrorKind::Auto { msg: msg.into() },
            _ => {}
        }
        err
    }
}

impl From<TestConfigError> for Error {
    fn from(e: TestConfigError) -> Self {
        Error {
            kind: ErrorKind::TestConfig(e),
            cause: None,
        }
    }
}

impl From<SyscommandError> for Error {
    fn from(e: SyscommandError) -> Self {
        Error {
            kind: ErrorKind::Syscommand(e),
            cause: None,
        }
    }
}
