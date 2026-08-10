use documented::{Documented, DocumentedFields};
use id2202_autograder_macros::TestKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::{discover_by_suffix, PostInit, PostInitCtx};
use crate::error::Error;

/// Execute a binary and hand its stdout, stderr and exit code to a
/// course-provided verifier program, which decides whether the test passed.
#[derive(JsonSchema, Debug, Clone, Documented, DocumentedFields, TestKind)]
#[testkind(ident = "run_verifier")]
pub struct RunVerifier {
    /// Binary to execute.
    pub bin: String,
    /// Command-line arguments.
    pub args: Vec<String>,
    /// Optional text to write on stdin.
    #[testkind(ignorable)]
    pub stdin: Option<String>,
    /// Files to copy into the container.
    pub input_files: Vec<String>,
    /// Suffixes for automatically discovering input files,
    /// e.g. `[".cpp"]`.
    pub auto_input_files: Vec<String>,

    /// Verifier program. The suffix selects the interpreter. Only `.py`
    /// (Python) is supported at the moment.
    ///
    /// Setting it discards any inherited parameters and parameter schema, since
    /// those belong to the verifier that declared them.
    #[testkind(relpath, clears(verifier_params, verifier_param_schema))]
    pub verifier_path: String,

    /// Parameters passed to the verifier, merged key by key down the tree. Each
    /// one has to be declared in `verifier_param_schema`.
    #[testkind(merge = "deep")]
    pub verifier_params: BTreeMap<String, ParamValue>,

    /// Parameters the verifier accepts, declared alongside `verifier_path`.
    pub verifier_param_schema: BTreeMap<String, ParamSpec>,

    /// Seconds a single run of the verifier may take. Exceeding it is an
    /// autograder error, not a failed test case, so this is about catching a
    /// broken verifier rather than a slow solution.
    pub verifier_timeout: u32,
}

/// A verifier parameter value. Only these three types cross the boundary into
/// the verifier, so the wire format stays trivially representable in JSON.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum ParamValue {
    Bool(bool),
    Int(i64),
    Str(String),
}

impl ParamValue {
    fn type_name(&self) -> &'static str {
        match self {
            Self::Bool(_) => "bool",
            Self::Int(_) => "int",
            Self::Str(_) => "str",
        }
    }
}

/// The declaration of a single verifier parameter.
#[derive(Deserialize, JsonSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct ParamSpec {
    /// One of `bool`, `int` or `str`.
    #[serde(rename = "type")]
    pub ty: ParamType,

    /// Value used when a test case does not set the parameter. Without one the
    /// parameter is required.
    pub default: Option<ParamValue>,
}

#[derive(Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    Bool,
    Int,
    Str,
}

impl ParamType {
    fn accepts(&self, value: &ParamValue) -> bool {
        matches!(
            (self, value),
            (Self::Bool, ParamValue::Bool(_))
                | (Self::Int, ParamValue::Int(_))
                | (Self::Str, ParamValue::Str(_))
        )
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::Int => "int",
            Self::Str => "str",
        }
    }
}

impl PostInit for RunVerifier {
    fn post_init(&mut self, ctx: &PostInitCtx) -> Result<(), Error> {
        discover_by_suffix(&mut self.input_files, &self.auto_input_files, ctx)?;

        let path = &self.verifier_path;
        if !std::path::Path::new(path).is_file() {
            return Err(
                Error::test_config_msg(format!("verifier \"{path}\" is not a file")).into(),
            );
        }
        if !path.ends_with(".py") {
            return Err(Error::test_config_msg(format!(
                "verifier \"{path}\" has no supported suffix, expected \".py\""
            ))
            .into());
        }

        self.resolve_params()
    }
}

impl RunVerifier {
    /// Checks every parameter against the schema and fills in the defaults, so
    /// that the verifier always receives a complete set.
    fn resolve_params(&mut self) -> Result<(), Error> {
        for (name, value) in self.verifier_params.iter() {
            let spec = self.verifier_param_schema.get(name).ok_or_else(|| {
                Error::test_config_msg("parameter is not declared in verifier_param_schema")
                    .key(name)
            })?;
            if !spec.ty.accepts(value) {
                return Err(Error::test_config_msg(format!(
                    "expected {}, got {}",
                    spec.ty.name(),
                    value.type_name()
                ))
                .key(name)
                .into());
            }
        }

        for (name, spec) in self.verifier_param_schema.iter() {
            if self.verifier_params.contains_key(name) {
                continue;
            }
            let default = spec.default.clone().ok_or_else(|| {
                Error::test_config_msg("parameter has no default and was not set").key(name)
            })?;
            if !spec.ty.accepts(&default) {
                return Err(Error::test_config_msg(format!(
                    "default is {}, expected {}",
                    default.type_name(),
                    spec.ty.name()
                ))
                .key(name)
                .into());
            }
            self.verifier_params.insert(name.to_owned(), default);
        }

        Ok(())
    }
}
