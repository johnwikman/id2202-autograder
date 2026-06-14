use actix_web::{
    error::InternalError,
    http::StatusCode,
    web::{self},
    HttpRequest, HttpResponse,
};
use num_traits::FromPrimitive;
use sailfish::TemplateSimple;

use id2202_autograder::{config::Settings, reporting::Report, utils::systemtime_to_utc_string};
use serde::Deserialize;

use crate::route::{
    common::{CommonInformation, RenderOptionString, RenderReport, ReportRenderOptions},
    error_msg::ErrorMessageTemplate,
};

/// Template for showing job information
#[derive(TemplateSimple)]
#[template(path = "route/submission.stpl")]
struct SubmissionTemplate {
    common: CommonInformation,
    // For this template only
    submission_id: i64,
    status_lists: Vec<SubmissionStatusList>,
    report: RenderReport,
}

struct SubmissionStatusList {
    title: Option<String>,
    title_href: Option<String>,
    items: Vec<SubmissionStatusListItem>,
}

#[derive(Default)]
struct SubmissionStatusListItem {
    li_class: RenderOptionString,
    label: String,
    value: String,
    value_span_class: RenderOptionString,
}

#[derive(Debug, Deserialize)]
struct SubmissionQuery {
    auth_key: String,
}

/// Display information about a single submission
pub async fn get_submission(
    data: web::Data<Settings>,
    req: HttpRequest,
    submission_id: web::Path<String>,
) -> Result<HttpResponse, actix_web::Error> {
    use id2202_autograder::db::{
        conn::DatabaseConnection, models::SubmissionInfo, models::SubmissionStatusCode as SSC,
    };

    let settings = data.get_ref();

    let q = match web::Query::<SubmissionQuery>::from_query(req.query_string()) {
        Ok(q) => q,
        Err(_) => {
            return ErrorMessageTemplate::unauthorized(settings);
        }
    };

    let parsed_id: i64 = match submission_id.parse() {
        Ok(v) => v,
        Err(_) => {
            log::error!("Bad submission id: {submission_id}");
            return ErrorMessageTemplate::not_found(settings);
        }
    };

    let mut conn = match DatabaseConnection::connect(&settings) {
        Ok(conn) => conn,
        Err(e) => {
            log::error!("Could not open database connection: {e}");
            return ErrorMessageTemplate::internal_server_error(settings);
        }
    };

    let subinfo = match conn.get_submission_info(parsed_id) {
        Ok(subinfo) => subinfo,
        Err(_) => {
            log::error!("Submission id not found: {parsed_id}");
            return ErrorMessageTemplate::not_found(settings);
        }
    };

    let sub = subinfo.get_submission();
    let src = subinfo.get_source();
    if src.auth_key != q.auth_key {
        return ErrorMessageTemplate::unauthorized(settings);
    }

    let report = match &sub.exec_report {
        Some(v) => match Report::deserialize(v) {
            Ok(r) => Some(r),
            Err(e) => {
                log::warn!("Invalid report: {:?}\n\n{}", e, v);
                None
            }
        },
        None => None,
    };

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
            label: "Status".to_string(),
            value: format!("{} {}", ssc.to_string(), rhs_symbol),
            ..Default::default()
        });
    }

    statlist_general.items.push(SubmissionStatusListItem {
        label: "Submitted At".to_string(),
        value: systemtime_to_utc_string(&sub.date_submitted).unwrap_or("NO_TIME".to_string()),
        ..Default::default()
    });

    if let Some(started_at) = &sub.exec_date_started {
        statlist_general.items.push(SubmissionStatusListItem {
            label: "Started At".to_string(),
            value: systemtime_to_utc_string(started_at).unwrap_or("NO_TIME".to_string()),
            ..Default::default()
        });
    }
    if let Some(finished_at) = &sub.exec_date_finished {
        statlist_general.items.push(SubmissionStatusListItem {
            label: "Finished At".to_string(),
            value: systemtime_to_utc_string(finished_at).unwrap_or("NO_TIME".to_string()),
            ..Default::default()
        });
    }

    if let Some(runner_id) = &sub.assigned_runner_id {
        statlist_general.items.push(SubmissionStatusListItem {
            label: "Assigned Runner".to_string(),
            value: runner_id.to_string(),
            ..Default::default()
        });
    }

    status_lists.push(statlist_general);

    // Collect submission source information
    let mut statlist_source = SubmissionStatusList {
        title: Some("Submission Source".to_string()),
        title_href: None,
        items: vec![],
    };

    match &subinfo {
        SubmissionInfo::GitHub {
            sub: _,
            src: _,
            gh_src,
            gh_info: _,
        } => {
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Origin".to_string(),
                value: "GitHub".to_string(),
                ..Default::default()
            });
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Domain".to_string(),
                value: gh_src.domain.clone(),
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
                label: "Origin".to_string(),
                value: "GitLab".to_string(),
                ..Default::default()
            });
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Domain".to_string(),
                value: gl_src.domain.clone(),
                ..Default::default()
            });
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Namespace".to_string(),
                value: gl_src.namespace.clone(),
                ..Default::default()
            });
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Repository".to_string(),
                value: gl_src.repo.clone(),
                ..Default::default()
            });
            statlist_source.items.push(SubmissionStatusListItem {
                label: "Commit".to_string(),
                value: gl_info.commit.clone(),
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
            v: report,
            options: ReportRenderOptions {
                symbol_ok: settings.reporting.markdown.symbol_ok.clone(),
                symbol_failed: settings.reporting.markdown.symbol_failed.clone(),
            },
        },
    };
    tpl.common.include_syntax_highlighting = true;

    let body: String = tpl
        .render_once()
        .map_err(|e| InternalError::new(e, StatusCode::INTERNAL_SERVER_ERROR))?;

    Ok(HttpResponse::Ok().body(body))
}
