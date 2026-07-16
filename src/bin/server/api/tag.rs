use std::path::Path;

use actix_web::{
    web::{self},
    HttpRequest, HttpResponse, Responder,
};

use id2202_autograder::config::{Settings, Tests, TestsLoadingOptions};

use crate::api::response::{ErrorResponse, TagListResponse, TagResponse};

/// Fetches a list of all grading tags and some of their metadata.
///
/// Required headers:
///  - Authorization
pub async fn get_taglist(
    data: web::Data<Settings>,
    req: HttpRequest,
) -> Result<impl Responder, actix_web::Error> {
    let settings = data.get_ref();

    let tc = Tests::load(
        &settings.runner.test_config,
        TestsLoadingOptions {
            taginfo_only: true,
            ..Default::default()
        },
    )
    .map_err(|e| {
        log::error!("Could not load test configuration: {e}");
        ErrorResponse::internal_server_error(&req)
    })?;

    Ok(TagListResponse::new(&req, &tc).to_http())
}

/// Fetches a single grading tags (or an alias of many) and some of its metadata.
///
/// Required headers:
///  - Authorization
pub async fn get_tag(
    data: web::Data<Settings>,
    req: HttpRequest,
    tagname: web::Path<String>,
) -> Result<impl Responder, actix_web::Error> {
    let settings = data.get_ref();
    let tagname = tagname.into_inner();

    let tc = Tests::load(
        &settings.runner.test_config,
        TestsLoadingOptions {
            taginfo_only: true,
            ..Default::default()
        },
    )
    .map_err(|e| {
        log::error!("Could not load test configuration: {e}");
        ErrorResponse::internal_server_error(&req)
    })?;

    TagResponse::new(&req, &tc, &tagname)
        .ok_or_else(|| ErrorResponse::not_found(&req, "tag not found").into())
        .map(|r| r.to_http())
}

/// Fetch the description/task for a specific grading tag. Unlike the other API
/// calls, this does not return a JSON response on success. Instead, this will
/// return data of an unknown format.
///
/// Required headers:
///  - Authorization
pub async fn get_tag_task(
    data: web::Data<Settings>,
    req: HttpRequest,
    tagname: web::Path<String>,
) -> Result<impl Responder, actix_web::Error> {
    use actix_web::http::header::{
        ContentDisposition, ContentType, DispositionParam, DispositionType,
    };

    let settings = data.get_ref();
    let tagname = tagname.into_inner();

    let tc = Tests::load(
        &settings.runner.test_config,
        TestsLoadingOptions {
            taginfo_only: true,
            ..Default::default()
        },
    )
    .map_err(|e| {
        log::error!("Could not load test configuration: {e}");
        ErrorResponse::internal_server_error(&req)
    })?;

    let tag_group = tc
        .tag_groups
        .get(&tagname)
        .ok_or_else(|| ErrorResponse::not_found(&req, "tag not found"))?;

    match tag_group.as_slice() {
        [t] => {
            if let Some(path) = &t.task_file {
                let data = std::fs::read(path).map_err(|e| {
                    log::error!("Could not read the task file: {e}");
                    ErrorResponse::internal_server_error(&req)
                })?;
                let basename = Path::new(path)
                    .file_name()
                    .and_then(|p| p.to_str())
                    .ok_or_else(|| {
                        log::error!("Cannot get basename from path: {path}");
                        ErrorResponse::internal_server_error(&req)
                    })?;
                HttpResponse::Ok()
                    .insert_header(ContentType::octet_stream())
                    .insert_header(ContentDisposition {
                        disposition: DispositionType::Attachment,
                        parameters: vec![
                            DispositionParam::Name("file".to_string()),
                            DispositionParam::Filename(basename.to_string()),
                        ],
                    })
                    .message_body(data)
            } else {
                Err(ErrorResponse::not_found(&req, "tag does not have task description").into())
            }
        }
        _ => Err(ErrorResponse::bad_request(
            &req,
            "provided tag name is not associated with a single grading tag",
        )
        .into()),
    }
}
