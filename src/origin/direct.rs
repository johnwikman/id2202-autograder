//! Direct submission origin
// Using this library for HTTP: https://docs.rs/reqwest/latest/reqwest/

use std::{io::Read, time::Duration};

use crate::{
    config::Settings,
    error::Error,
    origin::{FetchSpec, OriginKind},
    reporting::MetaReport,
};
use reqwest::{
    self,
    header::{HeaderMap, HeaderValue},
    Client as ReqwestClient,
};
use serde::Serialize;

#[derive(Clone, Copy, Debug)]
pub struct Direct;

#[derive(Clone, Debug)]
pub struct DirectInfo {
    pub domain: String,
    pub entity: String,
    pub local_path: String,
    pub sink_and_secret: Option<(String, String)>,
}

#[derive(Clone, Copy, Debug)]
pub enum DirectState {
    Waiting,
    InProgress,
    Success,
    Failed,
    Aborted,
    Unknown,
}

impl DirectState {
    fn as_str(&self) -> &'static str {
        match self {
            DirectState::Waiting => "waiting",
            DirectState::InProgress => "in progress",
            DirectState::Success => "success",
            DirectState::Failed => "failed",
            DirectState::Aborted => "aborted",
            DirectState::Unknown => "unknown",
        }
    }
}

/// Checks that the entity name only contains accepted characters, and that it
/// does not start or end with a special character.
pub fn is_valid_entity(entity: &str) -> bool {
    if !(1..=60).contains(&entity.len()) {
        return false;
    }

    entity.chars().all(|c| matches!(c, '0'..='9' | 'a'..='z' | 'A'..='Z' | '-' | '_' | '@' | '.'))
        && (!entity.starts_with(|c| matches!(c, '-' | '_' | '@' | '.')))
        && (!entity.ends_with(|c| matches!(c, '-' | '_' | '@' | '.')))
}

/// Prepares the sink message and the headers that needs to go along with it.
fn prepare_sink_message(
    msg: impl Serialize,
    secret_key: &str,
) -> Result<(String, HeaderMap), Error> {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let data = serde_json::to_string(&msg)?;

    // compute the hmac
    let mut mac = HmacSha256::new_from_slice(secret_key.as_bytes())
        .map_err(|e| Error::convert("could not create HMAC").with_cause(e))?;
    mac.update(data.as_bytes());

    let hmac256_value = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", HeaderValue::from_static("application/json"));
    headers.insert(
        "X-HMAC-Signature",
        HeaderValue::from_str(&hmac256_value)
            .map_err(|e| Error::format("invalid header value", &hmac256_value).with_cause(e))?,
    );

    Ok((data, headers))
}

impl OriginKind for Direct {
    type Info = DirectInfo;
    type SubmissionState = DirectState;
    type Fetch = DirectFetch;

    fn fetch_spec(_settings: &Settings, info: &Self::Info) -> Self::Fetch {
        DirectFetch { local_path: info.local_path.clone() }
    }

    /// Sends a "state" message back to the sink if this origin has specified a
    /// sink.
    async fn set_state(
        settings: &Settings,
        info: &Self::Info,
        state: &Self::SubmissionState,
        description: Option<&str>,
        submission_id: Option<i64>,
    ) -> Result<(), Error> {
        let Some((sink_url, sink_secret)) = &info.sink_and_secret else {
            log::info!("No sink specified for domain {} on entity {}", info.domain, info.entity);
            return Ok(());
        };

        #[derive(Debug, Clone, Serialize)]
        struct DirectStatus<'a> {
            submission_id: Option<i64>,
            domain: &'a str,
            entity: &'a str,
            message_type: &'a str,
            state: &'a str,
            description: Option<&'a str>,
        }

        let (data, headers) = prepare_sink_message(
            DirectStatus {
                submission_id,
                domain: &info.domain,
                entity: &info.entity,
                message_type: "state",
                state: state.as_str(),
                description,
            },
            sink_secret,
        )?;

        let c = ReqwestClient::new();
        let response = c
            .post(sink_url)
            .headers(headers)
            .body(data)
            .timeout(Duration::from_millis(settings.timeout.http_send_millisec.into()))
            .send()
            .await
            .map_err(|e| {
                log::error!("Error with GitHub commit status: {e}");
                e
            })?;

        if response.status().is_success() {
            log::debug!(
                "Successfully sent status to domain {} on entity {}",
                info.domain,
                info.entity
            );
            Ok(())
        } else {
            Error::err_http_response(
                "when sending direct status".to_string(),
                response.status().as_u16(),
                response.text().await.unwrap_or("no text received".to_string()),
            )
        }
    }

    /// Sends a "report" message back to the sink if this origin has specified
    /// a sink.
    async fn send_report<'a>(
        settings: &Settings,
        info: &Self::Info,
        report: &MetaReport<'a>,
        submission_id: Option<i64>,
    ) -> Result<(), Error> {
        let Some((sink_url, sink_secret)) = &info.sink_and_secret else {
            log::info!("No sink specified for domain {} on entity {}", info.domain, info.entity);
            return Ok(());
        };

        #[derive(Debug, Clone, Serialize)]
        struct DirectReport<'a> {
            submission_id: Option<i64>,
            domain: &'a str,
            entity: &'a str,
            message_type: &'a str,
            report: serde_json::Value,
        }

        let (data, headers) = prepare_sink_message(
            DirectReport {
                submission_id,
                domain: &info.domain,
                entity: &info.entity,
                message_type: "report",
                report: report.to_json(&settings.reporting)?,
            },
            sink_secret,
        )?;

        let c = ReqwestClient::new();
        let response = c
            .post(sink_url)
            .headers(headers)
            .body(data)
            .timeout(Duration::from_millis(settings.timeout.http_send_millisec.into()))
            .send()
            .await
            .map_err(|e| {
                log::error!("Error with GitHub commit status: {e}");
                e
            })?;

        if response.status().is_success() {
            log::debug!(
                "Successfully sent report to domain {} on origin {}",
                info.domain,
                info.entity
            );
            Ok(())
        } else {
            Error::err_http_response(
                "when sending direct report".to_string(),
                response.status().as_u16(),
                response.text().await.unwrap_or("no text received".to_string()),
            )
        }
    }
}

pub struct DirectFetch {
    local_path: String,
}

impl FetchSpec for DirectFetch {
    fn fetch_into(&self, settings: &Settings, dir: &str) -> Result<(), Error> {
        use crate::{archive::Archive, utils::write_all_timeout};
        use std::{fs::File, path::PathBuf, time::Duration};

        let mut f = File::open(&self.local_path)
            .map_err(|e| Error::fs("could not open local_path", &self.local_path).with_cause(e))?;

        let mut buf: Vec<u8> = Vec::new();
        f.read_to_end(&mut buf).map_err(|e| {
            Error::fs("error reading from local_path", &self.local_path).with_cause(e)
        })?;

        let archive = Archive::from_targz(&buf, None).map_err(|e| {
            Error::fs("error converting data from local_path to .tar.gz", &self.local_path)
                .with_cause(e)
        })?;

        let dir = PathBuf::from(dir);
        for (path, data) in archive.tree {
            let out_path = dir.join(path);
            if let Some(parent) = out_path.parent() {
                if !parent.exists() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        Error::fs("could not create parent directory", parent.to_string_lossy())
                            .with_cause(e)
                    })?;
                }
            }
            let mut out_file = File::create_new(&out_path).map_err(|e| {
                Error::fs("could not create archive path file", out_path.to_string_lossy())
                    .with_cause(e)
            })?;
            write_all_timeout(
                &mut out_file,
                &data,
                Duration::from_secs(settings.timeout.fs_write_seconds as u64),
            )
            .map_err(|e| {
                Error::fs("could not write to archive path", out_path.to_string_lossy())
                    .with_cause(e)
            })?;
        }

        Ok(())
    }
}
