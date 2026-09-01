// @generated automatically by Diesel CLI.

diesel::table! {
    submission_info_github (id) {
        id -> Int8,
        submission_id -> Int8,
        github_origin_id -> Int8,
        user -> Text,
        commit -> Text,
    }
}

diesel::table! {
    submission_info_gitlab (id) {
        id -> Int8,
        submission_id -> Int8,
        gitlab_origin_id -> Int8,
        user -> Text,
        commit -> Text,
    }
}

diesel::table! {
    submission_jobs (id) {
        id -> Int8,
        submission_id -> Int8,
        tag -> Text,
        requested_as -> Array<Text>,
        eligible_at -> Nullable<Timestamptz>,
        voided_at -> Nullable<Timestamptz>,
        assigned_runner_id -> Nullable<Int4>,
        status_code -> Int4,
        status_text -> Nullable<Text>,
        started_at -> Nullable<Timestamptz>,
        finished_at -> Nullable<Timestamptz>,
        report -> Nullable<Json>,
    }
}

diesel::table! {
    submission_origin_github (id) {
        id -> Int8,
        domain -> Text,
        org -> Text,
        repo -> Text,
        ssh_url -> Text,
    }
}

diesel::table! {
    submission_origin_gitlab (id) {
        id -> Int8,
        domain -> Text,
        namespace -> Text,
        repo -> Text,
        ssh_url -> Text,
    }
}

diesel::table! {
    submission_origins (id) {
        id -> Int8,
        kind -> Int4,
        kind_id -> Int8,
        auth_key -> Text,
    }
}

diesel::table! {
    submissions (id) {
        id -> Int8,
        submitted_at -> Timestamptz,
        requested_tags -> Array<Text>,
        origin_id -> Int8,
        report -> Nullable<Json>,
    }
}

diesel::joinable!(submission_info_github -> submission_origin_github (github_origin_id));
diesel::joinable!(submission_info_github -> submissions (submission_id));
diesel::joinable!(submission_info_gitlab -> submission_origin_gitlab (gitlab_origin_id));
diesel::joinable!(submission_info_gitlab -> submissions (submission_id));
diesel::joinable!(submission_jobs -> submissions (submission_id));
diesel::joinable!(submissions -> submission_origins (origin_id));

diesel::allow_tables_to_appear_in_same_query!(
    submission_info_github,
    submission_info_gitlab,
    submission_jobs,
    submission_origin_github,
    submission_origin_gitlab,
    submission_origins,
    submissions,
);
