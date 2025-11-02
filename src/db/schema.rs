// @generated automatically by Diesel CLI.

diesel::table! {
    runners (id) {
        id -> Int4,
        pid -> Nullable<Int8>,
        last_pinged -> Nullable<Timestamp>,
    }
}

diesel::table! {
    submissions (id) {
        id -> Int8,
        date_submitted -> Timestamp,
        assigned_runner -> Nullable<Int4>,
        grading_tags -> Text,
        exec_finished -> Bool,
        exec_status_code -> Int4,
        exec_status_text -> Nullable<Text>,
        exec_date_started -> Nullable<Timestamp>,
        exec_date_finished -> Nullable<Timestamp>,
        github_address -> Text,
        github_org -> Text,
        github_repo -> Text,
        github_user -> Text,
        github_commit -> Text,
    }
}

diesel::allow_tables_to_appear_in_same_query!(
    runners,
    submissions,
);
