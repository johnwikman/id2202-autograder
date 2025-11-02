// Notification functionality

use crate::{error::Error, settings::Settings, utils::path_absolute_parent};
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};
use std::os::fd::AsRawFd;
use std::{fs::File, io::Write};

/// Verifies that the notification file, creating it and intermediate
/// directories if absent.
pub fn verify_path(settings: &Settings) -> Result<(), Error> {
    // Open the file and write to it, create intermediate directories if necessary.
    let s = &settings.notify.path;

    let notify_dir = path_absolute_parent(s)?;

    std::fs::create_dir_all(&notify_dir).map_err(|e| {
        let errmsg = format!(
            "Error creating directory {} for the notification file: {}",
            &notify_dir, e
        );
        log::error!("{}", errmsg);
        Error::from(errmsg)
    })?;

    ping(settings)
}

/// Ping everyone who is listening for notifications.
pub fn ping(settings: &Settings) -> Result<(), Error> {
    // Open the file and write to it, create intermediate directories if necessary.
    let s = &settings.notify.path;

    let mut f = File::create(s).map_err(|e| {
        let errmsg = format!("Error opening notification file \"{}\": {}", &s, e);
        log::error!("{}", errmsg);
        Error::from(errmsg)
    })?;

    let ping_value = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| {
            let errmsg = format!("Error getting duration: {}", e);
            log::error!("{}", errmsg);
            Error::from(errmsg)
        })?
        .as_nanos()
        .to_string()
        .into_bytes();

    let _written = f.write(&ping_value).map_err(|e| {
        let errmsg = format!("Error writing to notification file \"{}\": {}", &s, e);
        log::error!("{}", errmsg);
        Error::from(errmsg)
    })?;
    Ok(())
}

/// A notification listener instance
pub struct Listener {
    instance: Inotify,
    path: String,
    timeout_ms: i32,
}

pub struct NotificationResult {
    pub timedout: bool,
}

impl Listener {
    /// Creates a new listener instance from the settings
    pub fn from_settings(settings: &Settings) -> Result<Self, Error> {
        Self::new(
            &settings.notify.path,
            settings.notify.poll_timeout_millisec.into(),
        )
    }

    /// Creates a new listener instance
    pub fn new(path: &str, timeout_ms: i32) -> Result<Self, Error> {
        let l = Listener {
            instance: Inotify::init(InitFlags::empty().union(InitFlags::IN_NONBLOCK))
                .map_err(|e| Error::from(format!("Error initializing inotify instance: {e}")))?,
            path: String::from(path),
            timeout_ms: timeout_ms,
        };

        // This isn't used, everything is recorded in the instance
        let _wd = l
            .instance
            .add_watch(l.path.as_str(), AddWatchFlags::IN_MODIFY)
            .map_err(|e| {
                Error::from(format!(
                    "Cannot inotify watch on file \"{}\": {}",
                    l.path, e
                ))
            })?;

        Ok(l)
    }

    /// Listens for notifications, or until the timeout occurs.
    pub fn listen(&self) -> Result<NotificationResult, Error> {
        use nix::poll::{poll, PollFd, PollFlags};

        let mut pollfds = [PollFd::new(self.instance.as_raw_fd(), PollFlags::POLLIN)];

        match poll(&mut pollfds, self.timeout_ms) {
            Ok(nready) => {
                if nready > 0 {
                    // Try to read events
                    match self.instance.read_events() {
                        Ok(_events) => Ok(NotificationResult { timedout: false }),
                        Err(e) => Err(Error::from(format!(
                            "Received error while reading inotify events: {e}"
                        ))),
                    }
                } else {
                    Ok(NotificationResult { timedout: true })
                }
            }
            Err(e) => Err(Error::from(format!(
                "Received error while watching file: {e}"
            ))),
        }
    }
}
