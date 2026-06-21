use actix_web::{
    body::BoxBody,
    error::InternalError,
    http::StatusCode,
    web::{self},
    HttpMessage, HttpRequest, HttpResponse,
};
use num_traits::FromPrimitive;
use sailfish::TemplateSimple;

use id2202_autograder::{
    config::Settings, db::models::SubmissionInfo, reporting::Report,
    utils::systemtime_to_utc_string,
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
) -> Result<(SubmissionInfo, Option<Report>), Result<HttpResponse, actix_web::Error>> {
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

    let mut conn = match DatabaseConnection::connect(&settings) {
        Ok(conn) => conn,
        Err(e) => {
            log::error!("Could not open database connection: {e}");
            return Err(error_msg::internal_server_error(settings));
        }
    };

    let subinfo = match conn.get_submission_info(parsed_id) {
        Ok(subinfo) => subinfo,
        Err(_) => {
            log::error!("Submission id not found: {parsed_id}");
            return Err(error_msg::not_found(settings));
        }
    };

    let sub = subinfo.get_submission();
    let src = subinfo.get_source();
    if auth_info.api_auth_ok {
        // OK, this counts as a valid authentication for all submissions
    } else if let Some(provided_auth_key) = &auth_info.auth_key {
        if &src.auth_key != provided_auth_key {
            return Err(error_msg::unauthorized(settings));
        }
    } else {
        // This shouldn't normally happen, but including this here for safety's sake.
        return Err(error_msg::unauthorized(settings));
    }

    let report = match conn.get_submission_report(sub.id) {
        Ok(maybe_r) => maybe_r,
        Err(e) => {
            log::warn!("Could not fetch report: {:?}", e);
            None
        }
    };

    Ok((subinfo, report))
}

/// Display information about a single submission
pub async fn get_submission(
    data: web::Data<Settings>,
    req: HttpRequest,
    submission_id: web::Path<String>,
) -> Result<HttpResponse, actix_web::Error> {
    use id2202_autograder::db::models::SubmissionStatusCode as SSC;

    let settings = data.get_ref();

    let (subinfo, opt_report) =
        match fetch_submission_and_report(settings, &req, submission_id.as_str()) {
            Ok(tup) => tup,
            Err(e) => {
                return e;
            }
        };
    let sub = subinfo.get_submission();

    let mut status_lists: Vec<SubmissionStatusList> = vec![];

    // First submission runtime metadata
    let mut statlist_general = SubmissionStatusList {
        title: None,
        title_href: None,
        items: vec![],
    };

    if let Some(ssc) = SSC::from_i32(sub.exec_status_code) {
        let (li_class, rhs_symbol) = match ssc {
            SSC::NotStarted | SSC::Running => (None, "".to_string()),
            SSC::Success => (
                Some("list-group-item-success".to_string()),
                settings.reporting.markdown.symbol_ok.clone(),
            ),
            _ => (
                Some("list-group-item-danger".to_string()),
                settings.reporting.markdown.symbol_failed.clone(),
            ),
        };
        statlist_general.items.push(SubmissionStatusListItem {
            li_class: li_class.into(),
            label: "Status",
            value: format!("{} {}", ssc.to_string(), rhs_symbol),
            ..Default::default()
        });
    }

    statlist_general.items.push(SubmissionStatusListItem {
        label: "Submitted At",
        value: systemtime_to_utc_string(&sub.date_submitted).unwrap_or("NO_TIME".to_string()),
        ..Default::default()
    });

    if let Some(started_at) = &sub.exec_date_started {
        statlist_general.items.push(SubmissionStatusListItem {
            label: "Started At",
            value: systemtime_to_utc_string(started_at).unwrap_or("NO_TIME".to_string()),
            ..Default::default()
        });
    }
    if let Some(finished_at) = &sub.exec_date_finished {
        statlist_general.items.push(SubmissionStatusListItem {
            label: "Finished At",
            value: systemtime_to_utc_string(finished_at).unwrap_or("NO_TIME".to_string()),
            ..Default::default()
        });
    }

    if let Some(runner_id) = &sub.assigned_runner_id {
        statlist_general.items.push(SubmissionStatusListItem {
            label: "Assigned Runner",
            value: runner_id.to_string(),
            ..Default::default()
        });
    }

    status_lists.push(statlist_general);

    // Collect submission source information
    let mut statlist_source = SubmissionStatusList {
        title: Some("Submission Source"),
        title_href: None,
        items: vec![],
    };

    match &subinfo {
        SubmissionInfo::GitHub {
            sub: _,
            src: _,
            gh_src,
            gh_info,
        } => {
            statlist_source.title_href = Some(format!(
                "https://{}/{}/{}/commit/{}",
                gh_src.domain, gh_src.org, gh_src.repo, gh_info.commit
            ));
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Origin",
                value: "GitHub".to_string(),
                svg_icon: Some("source-github"),
                ..Default::default()
            });
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Domain",
                value: gh_src.domain.clone(),
                ..Default::default()
            });
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Organization",
                value: gh_src.org.clone(),
                ..Default::default()
            });
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Repository",
                value: gh_src.repo.clone(),
                ..Default::default()
            });
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Commit",
                value: gh_info.commit.clone(),
                ..Default::default()
            });
        }
        SubmissionInfo::GitLab {
            sub: _,
            src: _,
            gl_src,
            gl_info,
        } => {
            let protocol = if settings
                .submission
                .gitlab
                .known_instances
                .iter()
                .find(|ki| ki.domain == gl_src.domain)
                .map(|ki| ki.use_https)
                .unwrap_or(true)
            {
                "https"
            } else {
                "http"
            };
            statlist_source.title_href = Some(format!(
                "{}://{}/{}/{}/-/commit/{}",
                protocol, gl_src.domain, gl_src.namespace, gl_src.repo, gl_info.commit
            ));

            statlist_source.items.push(SubmissionStatusListItem {
                label: "Origin",
                value: "GitLab".to_string(),
                svg_icon: Some("source-gitlab"),
                ..Default::default()
            });
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Domain",
                value: gl_src.domain.to_string(),
                ..Default::default()
            });
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Namespace",
                value: gl_src.namespace.to_string(),
                ..Default::default()
            });
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Repository",
                value: gl_src.repo.to_string(),
                ..Default::default()
            });
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Commit",
                value: gl_info.commit.to_string(),
                ..Default::default()
            });
        }
    }
    status_lists.push(statlist_source);

    let mut tpl = SubmissionTemplate {
        common: CommonInformation::from_title(settings, &format!("Submission {}", sub.id)),
        submission_id: sub.id,
        status_lists: status_lists,
        report: RenderReport {
            v: opt_report,
            settings: settings,
        },
    };
    tpl.common.include_syntax_highlighting = false;

    let body: String = tpl
        .render_once()
        .map_err(|e| InternalError::new(e, StatusCode::INTERNAL_SERVER_ERROR))?;

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
        Ok((_, Some(report))) => format!("{}", report.formatter_markdown(&settings.reporting)),
        Ok(_) => "No report generated for this submission".to_string(),
        Err(e) => {
            return e;
        }
    };

    Ok(HttpResponse::Ok()
        .insert_header(header::ContentType::plaintext())
        .body(BoxBody::new(md_text)))
}
