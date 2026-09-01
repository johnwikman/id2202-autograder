use actix_web::{
    body::BoxBody,
    error::InternalError,
    http::StatusCode,
    web::{self},
    HttpMessage, HttpRequest, HttpResponse,
};
use sailfish::TemplateSimple;

use id2202_autograder::{
    config::Settings,
    db::models::{origin::StoredOriginEnum, SubmissionStatus, SubmissionWithReports},
    reporting::MetaReport,
    utils::utc_string,
};

use crate::{
    auth::AuthorizationInfo,
    route::{
        common::{CommonInformation, RenderOptionString, RenderReport},
        error_msg,
    },
};

/// Template for showing job information
#[derive(TemplateSimple)]
#[template(path = "route/submission.stpl")]
struct SubmissionTemplate<'a> {
    common: CommonInformation,
    // For this template only
    submission_id: i64,
    status_lists: Vec<SubmissionStatusList<'a>>,
    report: RenderReport<'a>,
}

struct SubmissionStatusList<'a> {
    title: Option<&'a str>,
    title_href: Option<String>,
    items: Vec<SubmissionStatusListItem<'a>>,
}

#[derive(Default)]
struct SubmissionStatusListItem<'a> {
    li_class: RenderOptionString,
    label: &'a str,
    value: String,
    value_span_class: RenderOptionString,
    svg_icon: Option<&'a str>,
}

/// Helper function for authenticating and fetching the submission and the
/// report.
///
/// This can be authenticated using the auth_key parameter
fn fetch_submission_and_report(
    settings: &Settings,
    req: &HttpRequest,
    submission_id_string: &str,
) -> Result<SubmissionWithReports, Result<HttpResponse, actix_web::Error>> {
    use id2202_autograder::db::conn::DatabaseConnection;

    let auth_info = req
        .extensions()
        .get::<AuthorizationInfo>()
        .ok_or_else(|| error_msg::unauthorized(settings))?
        .clone();
    if !auth_info.any_provided() {
        // No authentication provided, no point in proceeding
        return Err(error_msg::unauthorized(settings));
    }

    let parsed_id: i64 = match submission_id_string.parse() {
        Ok(v) => v,
        Err(_) => {
            log::error!("Bad submission id: {submission_id_string}");
            return Err(error_msg::not_found(settings));
        }
    };

    let mut conn = match DatabaseConnection::connect(settings) {
        Ok(conn) => conn,
        Err(e) => {
            log::error!("Could not open database connection: {e}");
            return Err(error_msg::internal_server_error(settings));
        }
    };

    let sub = match SubmissionWithReports::by_id_opt(&mut conn, parsed_id) {
        Ok(Some(sub)) => sub,
        Ok(None) => {
            log::error!("Submission id not found: {parsed_id}");
            return Err(error_msg::not_found(settings));
        }
        Err(e) => {
            log::error!("Could not get submission {parsed_id}: {e}");
            return Err(error_msg::internal_server_error(settings));
        }
    };

    if auth_info.api_auth_ok {
        // OK, this counts as a valid authentication for all submissions
    } else if let Some(provided_auth_key) = &auth_info.auth_key {
        if &sub.origin.src_row.auth_key != provided_auth_key {
            return Err(error_msg::unauthorized(settings));
        }
    } else {
        // This shouldn't normally happen, but including this here for safety's sake.
        return Err(error_msg::unauthorized(settings));
    }

    Ok(sub)
}

/// Display information about a single submission
pub async fn get_submission(
    data: web::Data<Settings>,
    req: HttpRequest,
    submission_id: web::Path<String>,
) -> Result<HttpResponse, actix_web::Error> {
    let settings = data.get_ref();

    let sub = match fetch_submission_and_report(settings, &req, submission_id.as_str()) {
        Ok(sub) => sub,
        Err(e) => {
            return e;
        }
    };

    let mut status_lists: Vec<SubmissionStatusList> = vec![];

    // First submission runtime metadata
    let mut statlist_general =
        SubmissionStatusList { title: None, title_href: None, items: vec![] };

    let status = sub.status();
    let (li_class, rhs_symbol) = match status {
        SubmissionStatus::Waiting | SubmissionStatus::InProgress => {
            (None, settings.reporting.markdown.symbol_waiting.clone())
        }
        SubmissionStatus::Success => (
            Some("list-group-item-success".to_string()),
            settings.reporting.markdown.symbol_ok.clone(),
        ),
        SubmissionStatus::Failed | SubmissionStatus::Aborted | SubmissionStatus::Unknown => (
            Some("list-group-item-danger".to_string()),
            settings.reporting.markdown.symbol_failed.clone(),
        ),
    };
    statlist_general.items.push(SubmissionStatusListItem {
        li_class: li_class.into(),
        label: "Status",
        value: format!("{status} {rhs_symbol}"),
        ..Default::default()
    });

    statlist_general.items.push(SubmissionStatusListItem {
        label: "Submitted At",
        value: utc_string(&sub.submitted_at),
        ..Default::default()
    });

    statlist_general.items.push(SubmissionStatusListItem {
        label: "Requested Tags",
        value: sub.requested_tags.join(", "),
        ..Default::default()
    });

    if let Some(started_at) = sub.started_at() {
        statlist_general.items.push(SubmissionStatusListItem {
            label: "Started At",
            value: utc_string(&started_at),
            ..Default::default()
        });
    }
    if let Some(finished_at) = sub.finished_at() {
        statlist_general.items.push(SubmissionStatusListItem {
            label: "Finished At",
            value: utc_string(&finished_at),
            ..Default::default()
        });
    }

    status_lists.push(statlist_general);

    // Collect submission source information
    let mut statlist_source =
        SubmissionStatusList { title: Some("Submission Source"), title_href: None, items: vec![] };

    match &sub.origin.origin {
        StoredOriginEnum::GitHub(gh) => {
            statlist_source.title_href = Some(format!(
                "https://{}/{}/{}/commit/{}",
                gh.src.domain, gh.src.org, gh.src.repo, gh.info.commit
            ));
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Origin",
                value: "GitHub".to_string(),
                svg_icon: Some("source-github"),
                ..Default::default()
            });
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Domain",
                value: gh.src.domain.clone(),
                ..Default::default()
            });
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Organization",
                value: gh.src.org.clone(),
                ..Default::default()
            });
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Repository",
                value: gh.src.repo.clone(),
                ..Default::default()
            });
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Commit",
                value: gh.info.commit.clone(),
                ..Default::default()
            });
        }
        StoredOriginEnum::GitLab(gl) => {
            let protocol = if settings
                .submission
                .gitlab
                .known_instances
                .iter()
                .find(|ki| ki.domain == gl.src.domain)
                .map(|ki| ki.use_https)
                .unwrap_or(true)
            {
                "https"
            } else {
                "http"
            };
            statlist_source.title_href = Some(format!(
                "{}://{}/{}/{}/-/commit/{}",
                protocol, gl.src.domain, gl.src.namespace, gl.src.repo, gl.info.commit
            ));

            statlist_source.items.push(SubmissionStatusListItem {
                label: "Origin",
                value: "GitLab".to_string(),
                svg_icon: Some("source-gitlab"),
                ..Default::default()
            });
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Domain",
                value: gl.src.domain.to_string(),
                ..Default::default()
            });
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Namespace",
                value: gl.src.namespace.to_string(),
                ..Default::default()
            });
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Repository",
                value: gl.src.repo.to_string(),
                ..Default::default()
            });
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Commit",
                value: gl.info.commit.to_string(),
                ..Default::default()
            });
        }
    }
    status_lists.push(statlist_source);

    let mut tpl = SubmissionTemplate {
        common: CommonInformation::from_title(settings, &format!("Submission {}", sub.id)),
        submission_id: sub.id,
        status_lists,
        report: RenderReport { v: MetaReport::of_submission(&sub), settings },
    };
    tpl.common.include_syntax_highlighting = false;

    let body: String =
        tpl.render_once().map_err(|e| InternalError::new(e, StatusCode::INTERNAL_SERVER_ERROR))?;

    Ok(HttpResponse::Ok().body(body))
}

/// Display information about a single submission
pub async fn get_submission_markdown(
    data: web::Data<Settings>,
    req: HttpRequest,
    submission_id: web::Path<String>,
) -> Result<HttpResponse, actix_web::Error> {
    use actix_web::http::header;

    let settings = data.get_ref();

    let md_text = match fetch_submission_and_report(settings, &req, submission_id.as_str()) {
        Ok(sub) => {
            let results = MetaReport::of_submission(&sub);
            format!("{}", results.formatter_markdown(&settings.reporting))
        }
        Err(e) => {
            return e;
        }
    };

    Ok(HttpResponse::Ok()
        .insert_header(header::ContentType::plaintext())
        .body(BoxBody::new(md_text)))
}
