//! Listen to notifications in postgres.
//!
//! This does not use diesel since we need the timeout_iter function.

use std::time::Duration;

use postgres::fallible_iterator::FallibleIterator;
use postgres::{Client, NoTls};

use crate::config::Settings;
use crate::error::Error;

/// Listens to a notification on the channel `ch` on the postgres database
/// configured in the settings `s`.
///
/// See this link for more information:
/// https://www.postgresql.org/docs/current/sql-listen.html
///
/// The `NOTIFY` command is handled in `conn.rs`.
///
/// Warning: The value for `ch` can never come from a user as that will be
/// hardcoded into the query.
///
/// Note: This will open up a new connection to the database.
pub fn listen<S: AsRef<str>>(s: &Settings, ch: S) -> Result<bool, Error> {
    // Check that the channel is only ASCII alphabet chars
    if !ch.as_ref().bytes().all(|c| c.is_ascii_alphabetic()) {
        return Error::err_format("notify channel", ch.as_ref());
    }

    let conn_string: String = format!(
        "host={} port={} user={} password={} dbname=autograder connect_timeout=10",
        s.postgres.host, s.postgres.port, s.postgres.user, s.postgres.password
    );
    let mut client = Client::connect(&conn_string, NoTls)
        .map_err(|e| Error::auto_msg("could not connect to database for listen()", e))?;

    client
        .execute(&format!("LISTEN {};", ch.as_ref()), &[])
        .map_err(|e| {
            Error::auto_msg(
                format!("could not LISTEN on channel \"{}\"", ch.as_ref()),
                e,
            )
        })?;

    // https://docs.rs/postgres/0.19.12/postgres/struct.Notifications.html
    // https://docs.rs/postgres/0.19.12/postgres/notifications/struct.TimeoutIter.html
    let mut notifications = client.notifications();
    let mut to_iter =
        notifications.timeout_iter(Duration::from_millis(s.notify.poll_timeout_millisec as u64));

    match to_iter.next()? {
        Some(_) => Ok(true),
        None => Ok(false),
    }
}
