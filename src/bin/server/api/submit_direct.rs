use std::time::{SystemTime, UNIX_EPOCH};

use actix_web::{
    post,
    web::{self, Buf},
    HttpRequest, Responder,
};
use base64::Engine;
use derive_more::derive::Debug;
use serde::{Deserialize, Serialize};

use id2202_autograder::{
    archive::Archive,
    config::{Settings, Tests, TestsLoadingOptions},
    db::{
        conn::DatabaseConnection,
        models::{origin::DirectInfoData, raw::NewSubmissionOriginDirectRow, SubmissionStatus},
    },
    origin::{
        direct::{self, Direct, DirectInfo},
        Origin,
    },
    reporting::MetaReport,
    utils::path_absolute_join,
};
use sha2::Digest;

use crate::api::{
    common::{acceptance_message, internal_error_report, report_superseded, resolve_jobs},
    response::{ErrorResponse, SubmitResponse},
};

/// A serializable direct submission. This is our own custom format, and serves
/// as the definition of the interface for direct submissions.
#[derive(Debug, Serialize, Deserialize)]
struct DirectSubmission {
    /// The domain of the submitter.
    domain: String,

    /// Submitter entity (e.g. a user on the system).
    entity: String,

    /// The requested grading tags.
    grading_tags: Vec<String>,

    /// Format of the archive. Allowed values are "zip" and "tar.gz".
    archive: String,

    /// The encoding of the data. Allowed values are "base64".
    encoding: String,

    /// The encoded data representing the archive.
    #[debug("[{} bytes long]", data.len())]
    data: String,

    /// Place to write messages back to. If not provided, the submitter has to
    /// poll the API to get status.
    sink: Option<DirectSubmissionSink>,
}

/// Sink for where to write back status about the submission.
#[derive(Debug, Serialize, Deserialize)]
struct DirectSubmissionSink {
    /// The URL to send reports to. See `origin/direct` for more information
    /// about the format.
    url: String,

    /// A secret key which will be used to HMAC the entire data blob. Useful if
    /// you don't have any other authentication at the endpoint specified by
    /// `url`.
    #[debug("[REDACTED]")]
    secret_key: String,
}

/// Direct submissions as an archive sent in the header.
#[utoipa::path(
    tag = "Submissions",
    params(
        ("X-Direct-Secret" = String, Header, description = "The secret key used for direct submissions"),
    ),
    security(("direct_submission" = [])),
    responses(
        (status = 201, description = "Submission created and registered in the database.", body = SubmitResponse),
        (status = 400, description = "Malformed payload or missing headers.", body = ErrorResponse),
        (status = 401, description = "Invalid submission secret or unrecognized domain.", body = ErrorResponse),
    ),
)]
#[post("/submit/direct")]
pub async fn direct_submission(
    data: web::Data<Settings>,
    req: HttpRequest,
    payload: web::Payload,
) -> Result<impl Responder, actix_web::Error> {
    let settings = data.get_ref();

    log::info!(
        "Direct submission request from {}",
        req.peer_addr().map(|addr| addr.to_string()).unwrap_or("unknown".to_string()),
    );

    let secret = req
        .headers()
        .get("X-Direct-Secret")
        .and_then(|hv| hv.to_str().ok())
        .ok_or(ErrorResponse::bad_request(&req, "missing secret key"))?;

    // Before attempting to deserialize the payload, check the secret key
    if settings.submission.direct.secret != secret {
        return Err(ErrorResponse::unauthorized(&req, "invalid secret key").into());
    }

    // Decode the payload as JSON
    let payload_bytes = payload
        .to_bytes_limited(settings.submission.max_payload)
        .await
        .map_err(|e| {
            log::warn!("Error reading payload: {e}");
            ErrorResponse::bad_request(&req, "bad payload")
        })?
        .map_err(|e| {
            log::warn!("Error reading payload: {e}");
            ErrorResponse::bad_request(&req, "bad payload")
        })?;
    let sub: DirectSubmission = serde_json::from_slice(payload_bytes.chunk()).map_err(|err| {
        log::warn!("Received invalid JSON payload: {err:?}");
        ErrorResponse::bad_request(&req, "invalid JSON format")
    })?;

    if !settings.submission.direct.allowed_domains.contains(&sub.domain) {
        return Err(ErrorResponse::unauthorized(&req, "unrecognized domain").into());
    }

    if !direct::is_valid_entity(&sub.entity) {
        return Err(
            ErrorResponse::bad_request(&req, "entity name contains invalid characters").into()
        );
    }

    if sub.grading_tags.is_empty() {
        log::info!("Direct submission from {} did not provide any grading tags", sub.domain);
        return Err(ErrorResponse::bad_request(&req, "no grading tags provided").into());
    }

    // Resolve the test cases before creating anything on the file system.
    let tests = match Tests::load(
        settings,
        &settings.runner.test_config,
        TestsLoadingOptions { taginfo_only: true },
    ) {
        Ok(tests) => Some(tests),
        Err(e) => {
            log::error!("FATAL: could not load test configuration: {e}");
            None
        }
    };
    let jobs = match &tests {
        Some(tests) => resolve_jobs(tests, &sub.grading_tags),
        None => Err(Box::new(internal_error_report())),
    };

    let data = match sub.encoding.as_str() {
        "base64" => base64::prelude::BASE64_STANDARD.decode(&sub.data).map_err(|e| {
            log::warn!("Recieved invalid base64 encoding: {e:?}");
            ErrorResponse::bad_request(&req, "data is not base64 encoded")
        })?,
        enc => {
            log::warn!("Recieved invalid encoding \"{enc}\"");
            return Err(ErrorResponse::bad_request(&req, "invalid data encoding").into());
        }
    };

    let limit = Some(settings.submission.direct.max_unpacked_size as u64);
    let archive = match sub.archive.as_str() {
        "zip" => Archive::from_zip(&data, limit),
        "tar.gz" | "targz" => Archive::from_targz(&data, limit),
        arc => {
            log::warn!("Recieved unsupported archive format \"{arc}\"");
            return Err(ErrorResponse::bad_request(&req, "unsupported archive method").into());
        }
    }
    .map_err(|e| {
        log::error!("Could not unpack provided archive: {e}");
        ErrorResponse::bad_request(&req, "invalid archive")
    })?;

    // Write archive in tar.gz format to where it needs to be.
    let mut hash = hex::encode(
        sha2::Sha256::new()
            .chain_update(data)
            .chain_update(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
                    .to_le_bytes(),
            )
            .finalize(),
    );
    hash.truncate(hash.len().min(16));
    let mut written_path = None;
    // Try to write the file 10 times. On the 10th hash collision we report an
    // internal error, because this is so unlikely to happen given that we also
    // digest the SystemTime.
    for i in 0..10 {
        let path = path_absolute_join(
            &settings.submission.direct.storage_dir,
            format!("direct-{hash}-{i}.tar.gz"),
        )
        .map_err(|e| {
            log::error!("could not convert to path: {e}");
            ErrorResponse::internal_server_error(&req)
        })?;
        if std::fs::exists(&path)? {
            log::warn!("hash collision on path: {path}");
            continue;
        }
        archive.to_targz(&path).map_err(|e| {
            log::error!("could not write archive to path: {e}");
            ErrorResponse::internal_server_error(&req)
        })?;
        written_path = Some(path);
        break;
    }

    let Some(local_path) = written_path else {
        log::error!("could not write submitted data to file system after 10 tries");
        return Err(ErrorResponse::internal_server_error(&req).into());
    };

    let origin = Origin::<Direct> {
        info: DirectInfo {
            domain: sub.domain.clone(),
            entity: sub.entity.clone(),
            local_path: local_path.clone(),
            sink_and_secret: sub.sink.as_ref().map(|ss| (ss.url.clone(), ss.secret_key.clone())),
        },
    };

    let source =
        NewSubmissionOriginDirectRow { domain: sub.domain.clone(), entity: sub.entity.clone() };

    // Connect to database and insert the submission request
    let mut dbconn = DatabaseConnection::connect(settings).map_err(|err| {
        log::error!("Could not connect to database: {err}");
        ErrorResponse::internal_server_error(&req)
    })?;

    let jobs = match jobs {
        Ok(jobs) => jobs,
        Err(report) => {
            let submission = dbconn
                .register_ungradable_submission::<Direct>(
                    &sub.grading_tags.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                    &report,
                    &source,
                    &DirectInfoData {
                        local_path: &local_path,
                        sink_and_secret: origin
                            .info
                            .sink_and_secret
                            .as_ref()
                            .map(|(u, k)| (u.as_str(), k.as_str())),
                    },
                )
                .map_err(|e| {
                    log::error!("Could not register submission with database: {e}");
                    ErrorResponse::internal_server_error(&req)
                })?;

            origin
                .set_state_and_report(
                    settings,
                    &report.as_ref().into(),
                    &direct::DirectState::Failed,
                    Some("Submission Error"),
                    Some(submission.id),
                )
                .await
                .unwrap_or_else(|e| log::warn!("Could not submit commit info: {e}."));

            log::info!("Submission {} recorded, but nothing can be graded", submission.id);
            return Ok(
                SubmitResponse::new(&req, "submission cannot be graded", submission.id).to_http()
            );
        }
    };

    let registered = dbconn
        .register_submission::<Direct>(
            &sub.grading_tags.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            jobs,
            &source,
            &DirectInfoData {
                local_path: &local_path,
                sink_and_secret: origin
                    .info
                    .sink_and_secret
                    .as_ref()
                    .map(|(u, k)| (u.as_str(), k.as_str())),
            },
        )
        .map_err(|e| {
            log::error!("Could not register submission with database: {e}");
            ErrorResponse::internal_server_error(&req)
        })?;
    let submission_id = registered.submission.id;

    report_superseded(settings, &registered)
        .await
        .unwrap_or_else(|e| log::warn!("Could not report superseded jobs: {e}"));

    // A submission whose every tag was rejected has no job left for a runner
    // to pick up, so nothing else would ever move its commit off pending.
    let (state, label) = match registered.submission.status() {
        SubmissionStatus::Aborted => (direct::DirectState::Aborted, "Nothing To Grade"),
        _ => (direct::DirectState::Waiting, "Waiting In Queue"),
    };

    // Respond to the commit message and set the commit status
    origin
        .set_state_and_report(
            settings,
            &MetaReport::Structured(acceptance_message(&registered.submission)),
            &state,
            Some(label),
            Some(submission_id),
        )
        .await
        .unwrap_or_else(|e| log::warn!("Could not send info to submitter: {e}. Will not reject this submission since it is already created."));

    // Notifying the other runners (TODO: make this name configurable)
    dbconn.notify("submission").unwrap_or_else(|e| {
        log::warn!("Could not notify the runners about the new submission: {}", e)
    });

    log::info!("Submission {sub:?} successfully inserted with id {submission_id}");
    Ok(SubmitResponse::new(&req, "submission received", submission_id).to_http())
}
