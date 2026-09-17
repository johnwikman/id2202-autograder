//! The shell, the body builder, the field tables and the widgets every route is
//! assembled from.

pub mod body;
pub mod field;
pub mod shell;
pub mod widget;

pub use body::{slug, Body};
pub use field::{
    doc_table, field_table, type_badge, value_markdown, value_markdown_with_lead, warn_untyped,
    FieldDoc,
};
pub use shell::{html_page, warn_dangling_fragments, VENDOR_DIR};
pub use widget::{code_block, details, html_table, notched_box, LOCK_ICON};
