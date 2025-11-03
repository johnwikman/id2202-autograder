use crate::error::Error;
use chrono::DateTime;
use std::{
    ffi::OsString,
    fs::File,
    io::{Read, Write},
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use subprocess::{ExitStatus, Popen, PopenConfig, Redirection};
use tempfile;

/// Trims single newlines from the input string, returning a new string with
/// single newlines converted to spaces.
pub fn single_linefeed_to_space<S: AsRef<str>>(s: S) -> String {
    let s = s.as_ref().trim();
    if let (Some(offset_1), Some(offset_2), Some(lst)) = (
        s.split_at_checked(1),
        s.split_at_checked(2),
        s.chars().last(),
    ) {
        let mut ret = String::new();
        ret.extend(offset_1.0.chars());
        ret.extend(
            s.chars()
                .zip(offset_1.1.chars())
                .zip(offset_2.1.chars())
                .map(|((c_prev, c_mid), c_next)| {
                    if c_mid == '\n' && c_prev != '\n' && c_next != '\n' {
                        ' '
                    } else {
                        c_mid
                    }
                }),
        );
        ret.push(lst);
        ret
    } else {
        s.to_string()
    }
}

/// Returns a markdown preformatted block <pre> containing the provided text s
/// as verbatim, making sure to escape parts that could otherwise be
/// interpreted as HTML.
pub fn md_preformatted<S: AsRef<str>>(s: S) -> String {
    let mut ret_str = String::new();
    ret_str.push_str("<pre>\n");
    for c in s.as_ref().chars() {
        match c {
            '<' => ret_str.push_str("&lt;"),
            '>' => ret_str.push_str("&gt;"),
            '&' => ret_str.push_str("&amp;"),
            _ => ret_str.push(c),
        }
    }
    ret_str.push_str("\n</pre>");
    ret_str
}

/// Joins two file system paths together.
pub fn path_join<A: AsRef<Path>, B: AsRef<Path>>(a: A, b: B) -> Result<String, Error> {
    a.as_ref()
        .join(b.as_ref())
        .to_str()
        .map(String::from)
        .ok_or(Error::from("Could not convert path to a string."))
}

/// Joins two file system paths together and returns the absolute path of the
/// result.
pub fn path_absolute_join<A: AsRef<Path>, B: AsRef<Path>>(a: A, b: B) -> Result<String, Error> {
    std::path::absolute(a.as_ref().join(b.as_ref()))?
        .to_str()
        .map(String::from)
        .ok_or(Error::from("Could not convert path to a string."))
}

/// Returns the absolute parent path of the provided string, which can succeed
/// even if the path doesn't exist.
pub fn path_absolute_parent<P: AsRef<Path>>(path: P) -> Result<String, Error> {
    std::path::absolute(path.as_ref())?
        .parent()
        .map(|e| e.to_owned())
        .and_then(|p| p.to_str().map(String::from))
        .ok_or(Error::from("Internal error: Could not get parent of path."))
}

/// Creates a directory if it does not already exist.
pub fn create_dir_if_not_exists<P: AsRef<Path>>(path: P) -> Result<(), Error> {
    if !std::fs::exists(path.as_ref())? {
        std::fs::create_dir_all(path.as_ref())?;
    }
    Ok(())
}

/// Converts the provided system time to a string formatted as
/// YYYY-mm-dd HH:MM:SS in UTC time.
pub fn systemtime_to_utc_string(systime: &SystemTime) -> Option<String> {
    systime
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| {
            let secs: i64 = d.as_secs().try_into().ok()?;
            DateTime::from_timestamp(secs, d.subsec_nanos())
        })
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
}

/// Converts the provided system time to a string that is suitable for use in
/// file system entries, formatted as YYYY-mm-dd_HHMMSS.micros in UTC time.
pub fn systemtime_to_fsfriendly_utc_string(systime: &SystemTime) -> Option<String> {
    systime
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| {
            let secs: i64 = d.as_secs().try_into().ok()?;
            DateTime::from_timestamp(secs, d.subsec_nanos())
        })
        .map(|dt| dt.format("%Y-%m-%d_%H%M%S.%6f").to_string())
}

/// Returns a string containing the mimetype for the file located at the
/// specified path.
pub fn mimetype<P: AsRef<Path>>(path: P) -> Result<String, Error> {
    let res = syscommand_timeout(
        &[
            "file",
            "-b",
            "--mime",
            path.as_ref()
                .to_str()
                .ok_or_else(|| Error::from("Invalid UTF-8 path for entry {entry}"))?,
        ],
        SyscommandSettings {
            max_stdout_length: Some(10000),
            ..Default::default()
        },
    )?;
    let mime_string = res.stdout.trim_ascii().to_string();
    let mimetype = mime_string
        .split(" ")
        .next()
        .ok_or(Error::from("Internal error"))?;
    let mimetype = if mimetype.ends_with(";") {
        mimetype
            .split_at_checked(mimetype.len() - 1)
            .ok_or(Error::from("Internal error"))?
            .0
    } else {
        mimetype
    };
    Ok(mimetype.to_string())
}

#[derive(Debug, Clone)]
pub struct SyscommandSettings {
    pub timeout: Duration,
    pub expected_code: Option<i32>,
    pub stdin: Option<String>,
    pub max_stdout_length: Option<usize>,
    pub max_stderr_length: Option<usize>,
}

impl Default for SyscommandSettings {
    fn default() -> Self {
        SyscommandSettings {
            timeout: Duration::from_secs(60),
            expected_code: None,
            stdin: None,
            max_stdout_length: None,
            max_stderr_length: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SyscommandOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Runs a command with a timeout.
/// On success, returns code, stdout, and stderr.
///
/// Example, to run a simple command:
/// ```
/// use id2202_autograder::utils::{
///     syscommand_timeout, SyscommandSettings};
///
/// let ret = syscommand_timeout(
///     ["echo", "foo"],
///     SyscommandSettings::default()
/// ).unwrap();
///
/// println!("Returned {}", ret.code);
/// ```
pub fn syscommand_timeout<S: AsRef<str>, CmdList: AsRef<[S]>>(
    cmd: CmdList,
    cmd_settings: SyscommandSettings,
) -> Result<SyscommandOutput, Error> {
    let os_cmd: Vec<OsString> = cmd
        .as_ref()
        .iter()
        .map(|s| OsString::from(s.as_ref()))
        .collect();

    let stdin_filepath = match cmd_settings.stdin {
        Some(s) => {
            let mut f = tempfile::NamedTempFile::new()?;
            f.write(s.as_bytes())?;
            Some(
                f.keep()
                    .map_err(|e| {
                        Error::from(format!(
                            "Error marking the temp stdin file for keeping: {e}"
                        ))
                    })?
                    .1,
            )
        }
        None => None,
    };

    /// Convenience function for cleaning up the possibly dangling stdin tempfile.
    fn cleantemp(p: &Option<PathBuf>) -> () {
        p.as_ref()
            .inspect(|p| std::fs::remove_file(p).unwrap_or(()));
    }

    let mut handle = Popen::create(
        &os_cmd,
        PopenConfig {
            stdin: match &stdin_filepath {
                Some(path) => {
                    let f = File::open(path).inspect_err(|_| cleantemp(&stdin_filepath))?;
                    Redirection::File(f)
                }
                None => Redirection::None,
            },
            stdout: if cmd_settings.max_stdout_length.is_some() {
                Redirection::Pipe
            } else {
                Redirection::None
            },
            stderr: if cmd_settings.max_stderr_length.is_some() {
                Redirection::Pipe
            } else {
                Redirection::None
            },
            ..Default::default()
        },
    )
    .map_err(|e| {
        cleantemp(&stdin_filepath);
        Error::from(format!("Could not create Popen process: {e}"))
    })?;

    let mut buf_stdout: Vec<u8> = vec![];
    let mut buf_stderr: Vec<u8> = vec![];

    let end_time = SystemTime::now()
        .checked_add(cmd_settings.timeout)
        .unwrap_or_else(SystemTime::now);

    /// A wrapped function that reads stdout and stderr as the process is
    /// running. This is used to ensure that the process is killed even if
    /// something goes wrong with the IO.
    fn wrapped_read_and_wait(
        handle: &mut Popen,
        buf_stdout: &mut Vec<u8>,
        buf_stderr: &mut Vec<u8>,
        end_time: SystemTime,
        max_stdout_length: usize,
        max_stderr_length: usize,
    ) -> Result<Option<ExitStatus>, Error> {
        let mut read_buf: [u8; 4096] = [0u8; 4096];

        let mut poll = mio::Poll::new()
            .inspect_err(|e| log::error!("Received error when registering stderr: {e}"))?;
        let mut events = mio::Events::with_capacity(1024);

        if let Some(f) = &handle.stdout {
            poll.registry()
                .register(
                    &mut mio::unix::SourceFd(&f.as_raw_fd()),
                    mio::Token(1),
                    mio::Interest::READABLE,
                )
                .inspect_err(|e| log::error!("Received error when registering stdout: {e}"))?;
        }
        if let Some(f) = &handle.stderr {
            poll.registry()
                .register(
                    &mut mio::unix::SourceFd(&f.as_raw_fd()),
                    mio::Token(2),
                    mio::Interest::READABLE,
                )
                .inspect_err(|e| log::error!("Received error when registering stderr: {e}"))?;
        }

        let mut stat = None;

        while SystemTime::now() < end_time && stat.is_none() {
            poll.poll(&mut events, Some(Duration::from_millis(100)))?;

            for event in &events {
                if event.token() == mio::Token(1) {
                    if let Some(f) = handle.stdout.as_mut() {
                        let l = f.read(read_buf.as_mut_slice())?;
                        buf_stdout.extend_from_slice(read_buf.split_at(l).0);
                        if buf_stdout.len() > max_stdout_length {
                            return Err(Error::SyscommandOutputLimitExceededError(
                                max_stdout_length,
                            ));
                        }
                    }
                } else if event.token() == mio::Token(2) {
                    if let Some(f) = handle.stderr.as_mut() {
                        let l = f.read(read_buf.as_mut_slice())?;
                        buf_stderr.extend_from_slice(read_buf.split_at(l).0);
                        if buf_stderr.len() > max_stderr_length {
                            return Err(Error::SyscommandOutputLimitExceededError(
                                max_stderr_length,
                            ));
                        }
                    }
                }
            }

            stat = handle.poll();
        }

        // If we did not time out, make sure that we read the last data from
        // stdout and stderr
        if stat.is_some() {
            if let Some(f) = handle.stdout.as_mut() {
                loop {
                    let l = f.read(read_buf.as_mut_slice())?;
                    buf_stdout.extend_from_slice(read_buf.split_at(l).0);
                    if buf_stdout.len() > max_stdout_length {
                        return Err(Error::SyscommandOutputLimitExceededError(max_stdout_length));
                    }
                    if l < 4096 {
                        break;
                    }
                }
            }
            if let Some(f) = handle.stderr.as_mut() {
                loop {
                    let l = f.read(read_buf.as_mut_slice())?;
                    buf_stderr.extend_from_slice(read_buf.split_at(l).0);
                    if buf_stderr.len() > max_stderr_length {
                        return Err(Error::SyscommandOutputLimitExceededError(max_stderr_length));
                    }
                    if l < 4096 {
                        break;
                    }
                }
            }
        }

        Ok(stat)
    }

    let wait_result = wrapped_read_and_wait(
        &mut handle,
        &mut buf_stdout,
        &mut buf_stderr,
        end_time,
        cmd_settings.max_stdout_length.unwrap_or(0),
        cmd_settings.max_stderr_length.unwrap_or(0),
    )
    .inspect_err(|e| {
        log::warn!("(Terminating process) Runtime error when waiting for it to finish: {e}");
        handle
            .kill()
            .unwrap_or_else(|e| log::error!("Could not kill process: {e}"));
        cleantemp(&stdin_filepath);
    })?;

    cleantemp(&stdin_filepath);

    let stdout = String::from_utf8_lossy(buf_stdout.as_slice()).into_owned();
    let stderr = String::from_utf8_lossy(buf_stderr.as_slice()).into_owned();

    match wait_result {
        Some(stat) => match stat {
            ExitStatus::Exited(ucode) => {
                let code = ucode as i32;
                if cmd_settings.expected_code.map_or(true, |ec| ec == code) {
                    Ok(SyscommandOutput {
                        code: code,
                        stdout: stdout,
                        stderr: stderr,
                    })
                } else {
                    Err(format!("Exited with unexpected code {code}").into())
                }
            }
            ExitStatus::Signaled(sig) => Err(format!("Terminated by signal {sig}").into()),
            ExitStatus::Other(v) => Err(format!("Unknown exit status {v}").into()),
            ExitStatus::Undetermined => Err("Undetermined error".into()),
        },
        None => {
            handle
                .kill()
                .unwrap_or_else(|e| log::warn!("Could not killed timed out process: {e}"));
            Err(Error::SyscommandTimeoutError {
                stdout: Some(stdout),
                stderr: Some(stderr),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asserting::prelude::*;

    #[test]
    fn test_path_join() {
        assert_that!(path_join("foo", "bar")).has_value("foo/bar");
        assert_that!(path_join("/foo", "bar")).has_value("/foo/bar");
        assert_that!(path_join("/foo", "/bar")).has_value("/bar");
        assert_that!(path_join("foo/bar", "babar.txt")).has_value("foo/bar/babar.txt");
    }

    #[test]
    fn test_single_linefeed_to_space() {
        assert_that!(single_linefeed_to_space("bla bla")).is_equal_to("bla bla");
        assert_that!(single_linefeed_to_space(" ")).is_equal_to("");
        assert_that!(single_linefeed_to_space("a")).is_equal_to("a");
        assert_that!(single_linefeed_to_space("aA")).is_equal_to("aA");
        assert_that!(single_linefeed_to_space("aAb")).is_equal_to("aAb");
        assert_that!(single_linefeed_to_space("   foo\nbar  ")).is_equal_to("foo bar");
        assert_that!(single_linefeed_to_space("\n\nfoo\nbar\n\nbabar"))
            .is_equal_to("foo bar\n\nbabar");
        assert_that!(single_linefeed_to_space("\nfoo\nbar\n\n\nbabar  \n"))
            .is_equal_to("foo bar\n\n\nbabar");
    }

    #[test]
    fn test_md_preformatted() {
        assert_that!(md_preformatted("foo")).is_equal_to("<pre>\nfoo\n</pre>");
        assert_that!(md_preformatted("int foo() {return 1 < 2;}"))
            .is_equal_to("<pre>\nint foo() {return 1 &lt; 2;}\n</pre>");
        assert_that!(md_preformatted(
            "bool bar(int x) {\n  return x < 2 && x >= 2;\n}"
        ))
        .is_equal_to(
            "<pre>\nbool bar(int x) {\n  return x &lt; 2 &amp;&amp; x &gt;= 2;\n}\n</pre>",
        );
    }

    #[test]
    fn test_mimetype() {
        {
            let mut f = tempfile::NamedTempFile::new().unwrap();
            f.write("{\"foo\": 1, \"bar\": true}".as_bytes()).unwrap();
            assert_that!(mimetype(f.path()).unwrap()).contains("json");
        }
        {
            let mut f = tempfile::NamedTempFile::new().unwrap();
            f.write("foo bar\nI am a regular text file...".as_bytes())
                .unwrap();
            assert_that!(mimetype(f.path()).unwrap()).starts_with("text");
        }
    }

    #[test]
    fn test_syscommand_simple() {
        let ret = syscommand_timeout(
            ["echo", "foo"],
            SyscommandSettings {
                max_stdout_length: Some(10),
                ..Default::default()
            },
        );
        assert_that!(&ret).is_ok();
        assert_that!(&ret)
            .ok()
            .mapping(|s| &s.stdout)
            .is_equal_to("foo\n");
    }

    #[test]
    fn test_syscommand_with_timeout() {
        let ret = syscommand_timeout(
            ["sleep", "2"],
            SyscommandSettings {
                timeout: Duration::from_secs(1),
                ..Default::default()
            },
        );
        assert_that!(&ret).is_err();
        assert_that!(&ret).err().satisfies(|e| match e {
            Error::SyscommandTimeoutError { .. } => true,
            _ => false,
        });
    }
}
