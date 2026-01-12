// @generated automatically by Diesel CLI.

diesel::table! {
    submission_info_github (id) {
        id -> Int8,
        submission_id -> Int8,
        github_source_id -> Int8,
        user -> Text,
        commit -> Text,
    }
}

diesel::table! {
    submission_source_github (id) {
        id -> Int8,
        domain -> Text,
        org -> Text,
        repo -> Text,
    }
}

diesel::table! {
    submission_sources (id) {
        id -> Int8,
        kind -> Int4,
        kind_id -> Int8,
        auth_key -> Text,
    }
}

diesel::table! {
    submissions (id) {
        id -> Int8,
        date_submitted -> Timestamp,
        assigned_runner_id -> Nullable<Int4>,
        grading_tags -> Text,
        exec_finished -> Bool,
        exec_status_code -> Int4,
        exec_status_text -> Nullable<Text>,
        exec_date_started -> Nullable<Timestamp>,
        exec_date_finished -> Nullable<Timestamp>,
        exec_report -> Nullable<Json>,
        source_id -> Int8,
    }
}

diesel::joinable!(submission_info_github -> submission_source_github (github_source_id));
diesel::joinable!(submission_info_github -> submissions (submission_id));
diesel::joinable!(submissions -> submission_sources (source_id));

diesel::allow_tables_to_appear_in_same_query!(
    submission_info_github,
    submission_source_github,
    submission_sources,
    submissions,
);
