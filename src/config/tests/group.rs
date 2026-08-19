//! Test groups and the test cases inside them. A group is one directory with a
//! `config.toml`; a test case is one `*.test.toml` file.

use documented::{Documented, DocumentedFields};
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::BTreeMap;

use super::kind::{ApplyCtx, Kind, KindDefaults, PostInit, PostInitCtx};
use crate::{
    error::Error,
    utils::{path_absolute_join, path_join, single_linefeed_to_space},
};

/// Defaults for individual test cases.
#[derive(Deserialize, JsonSchema, Debug, Clone, Documented, DocumentedFields)]
pub struct TestConfig {
    /// Default per-test-case timeout in seconds.
    pub timeout: u32,

    /// Maximum captured output on stdout and stderr for a test case,
    /// in bytes.
    pub max_output: usize,

    /// Test kind used by a test case that does not name one itself.
    pub kind: String,

    /// Per-test-kind default option values, documented separately per kind, so
    /// they are kept out of this schema (and so out of the table below).
    #[schemars(skip)]
    pub kinds: KindDefaults,
}

/// A test case to run.
#[derive(Debug, Clone)]
pub struct Test {
    pub name: String,
    pub description: Option<String>,
    pub timeout: u32,
    pub max_output: usize,
    pub kind: Kind,
}

/// Deserializable `[test]` block of a single `config.toml` or `*.test.toml`
/// file.
#[derive(Deserialize, Debug, Clone, Default)]
struct _UntreatedTest {
    kind: Option<String>,
    timeout: Option<u32>,
    max_output: Option<usize>,

    /// Options written as `[test.options]`, an alias for `[test.default.<kind>]`
    /// with the kind in scope at this file.
    options: Option<toml::Table>,

    /// Options written as `[test.default.<kind>]`, kept apart per kind so that a
    /// test case switching kind does not inherit options meant for another.
    default: Option<BTreeMap<String, toml::Table>>,
}

impl _UntreatedTest {
    /// This file's options keyed by kind, with `[test.options]` resolved.
    /// `inherited_kind` is the kind in scope before this file was read, which
    /// the alias falls back to when the file does not name a kind itself.
    fn options_by_kind(
        &self,
        inherited_kind: &str,
    ) -> Result<BTreeMap<String, toml::Table>, Error> {
        if let Some(opts) = &self.options {
            if self.default.is_some() {
                return Err(Error::test_config_msg(
                    "[test.options] and [test.default.<kind>] cannot both be used in one file",
                )
                .into());
            }
            let kind = self.kind.as_deref().unwrap_or(inherited_kind);
            return Ok(BTreeMap::from([(kind.to_owned(), opts.to_owned())]));
        }

        let by_kind = self.default.clone().unwrap_or_default();
        for ident in by_kind.keys() {
            if !Kind::idents().iter().any(|i| i == ident) {
                return Err(Error::test_config_msg("unknown test kind in [test.default]")
                    .as_error()
                    .with_cause(Box::new(Error::identifier(ident.as_str(), Kind::idents()))));
            }
        }
        Ok(by_kind)
    }
}

impl TestConfig {
    /// Returns the configuration a file inherits from `self`, with everything
    /// the file sets itself applied on top. `dir` is the directory holding that
    /// file, which its relative paths resolve against.
    fn extend(&self, overrides: &Option<_UntreatedTest>, dir: &str) -> Result<Self, Error> {
        let Some(overrides) = overrides else {
            return Ok(self.clone());
        };

        let mut new = self.clone();
        new.timeout = overrides.timeout.unwrap_or(self.timeout);
        new.max_output = overrides.max_output.unwrap_or(self.max_output);
        new.kind = overrides.kind.clone().unwrap_or_else(|| self.kind.clone());

        let mut kinds = self.kinds.clone();
        for (ident, kind_overrides) in overrides.options_by_kind(&self.kind)? {
            kinds.apply(&ident, &kind_overrides, &ApplyCtx { dir })?;
        }

        Ok(Self {
            timeout: overrides.timeout.unwrap_or(self.timeout),
            max_output: overrides.max_output.unwrap_or(self.max_output),
            kind: overrides.kind.clone().unwrap_or_else(|| self.kind.clone()),
            kinds,
        })
    }
}

/// A group of test cases to run. Can also involve several subtests.
#[derive(Debug, Clone)]
pub struct TestGroup {
    pub title: String,
    pub description: Option<String>,
    pub tests: Vec<Test>,
    pub subgroups: Vec<TestGroup>,
}

impl TestGroup {
    /// Constructs a new test group located in the directory dir.
    ///
    /// First scans the config.toml file under dir that updates the
    /// test_defaults with new default test configuration. Then dir is scanned
    /// for tests.
    ///
    /// If a file is encountered that ends with .test.toml, a new test case is
    /// created.
    ///
    /// If a directory is encountered, then that is treated as a test group
    /// which will be a sub group to this test group.
    ///
    /// # Parameters
    ///
    /// - `dir`: The directory to scan
    /// - `inherited`: Configuration accumulated from the root defaults and
    ///                every ancestor `config.toml`.
    /// - `numbering`: Sequence of numbers that keeps that track of numbering
    ///                for test group titles.
    pub fn new(dir: &str, inherited: &TestConfig, numbering: Vec<i32>) -> Result<TestGroup, Error> {
        log::debug!("Creating test group from directory {dir}");

        /// A deserializable test group, i.e. the contents of a `config.toml` file.
        #[derive(Deserialize, Debug, Clone)]
        struct _UntreatedTestGroup {
            pub title: Option<String>,
            pub description: Option<String>,
            pub include: Option<Vec<String>>,
            /// This is actually default configuration for test cases within
            /// this test group.
            pub test: Option<_UntreatedTest>,
        }

        let config_path = path_join(dir, "config.toml")?;

        let mut tc_err = Error::test_config().path(&config_path);

        let contents: String = std::fs::read_to_string(&config_path).map_err(|e| {
            tc_err.to_owned().msg("could not read into string").as_error().with_cause(Box::new(e))
        })?;
        let utg: _UntreatedTestGroup = toml::from_str(&contents).map_err(|e| {
            tc_err.to_owned().msg("could not deserialize toml").as_error().with_cause(Box::new(e))
        })?;

        // Setting up the defaults for this test group
        let group_config = inherited.extend(&utg.test, dir).map_err(|e| {
            tc_err.to_owned().msg("invalid test configuration").as_error().with_cause(Box::new(e))
        })?;

        // Build title with numbering prefix (e.g., "1.2.3. Title")
        let title =
            utg.title.ok_or_else(|| tc_err.to_owned().msg("missing title for test group"))?;
        let prefix: String = numbering.iter().map(|i| format!("{i}.")).collect();
        let tg_title = if prefix.is_empty() { title } else { format!("{prefix} {title}") };

        // Associate the title with any potential errors.
        tc_err.title = Some(tg_title.to_owned());

        let mut tg = TestGroup {
            title: tg_title,
            description: utg.description.map(single_linefeed_to_space),
            tests: vec![],
            subgroups: vec![],
        };

        // Find all the test cases in the same directory
        // filenames: [(fname: String, is_dir: bool), ...]
        let mut filenames: Vec<(String, bool)> = std::fs::read_dir(dir)?
            .map(|e| {
                let e = e?;
                let name = e.file_name().to_str().map(String::from).ok_or_else(|| {
                    tc_err
                        .to_owned()
                        .msg("Couldn't get string representation of DirEntry")
                        .as_error()
                })?;
                Ok((name, e.metadata()?.is_dir()))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        filenames.sort_by(|(a, _), (b, _)| a.cmp(b));

        // Add any included directories
        // (there are checked last, in the specified order)
        if let Some(dirs) = utg.include {
            for d in dirs.into_iter() {
                let d_path = path_absolute_join(dir, d)?;
                if std::path::Path::new(&d_path).is_dir() {
                    filenames.push((d_path, true))
                } else {
                    return Err(tc_err.msg(format!("{d_path} is not a directory")).into());
                }
            }
        }

        let mut group_number: i32 = 0;
        for (filename, is_dir) in filenames {
            if is_dir {
                group_number += 1;
                let mut new_numbering = numbering.clone();
                new_numbering.push(group_number);
                let subdir = path_absolute_join(dir, &filename)?;
                //log::debug!("Scanning test subgroup from {subdir}");
                tg.subgroups.push(TestGroup::new(&subdir, &group_config, new_numbering)?);
            } else if let Some((prefix, "")) = filename.rsplit_once(".test.toml") {
                let testfile_path = path_absolute_join(dir, &filename)?;
                //log::debug!("Found test file {testfile_path}");

                tc_err.path = Some(testfile_path.to_owned());

                let contents: String = std::fs::read_to_string(&testfile_path).map_err(|e| {
                    tc_err
                        .to_owned()
                        .msg("could not read into string")
                        .as_error()
                        .with_cause(Box::new(e))
                })?;
                let test_contents: _UntreatedTestGroup =
                    toml::from_str(&contents).map_err(|e| {
                        tc_err
                            .to_owned()
                            .msg("could not deserialize toml")
                            .as_error()
                            .with_cause(Box::new(e))
                    })?;

                let test_config = group_config.extend(&test_contents.test, dir).map_err(|e| {
                    tc_err
                        .to_owned()
                        .msg("invalid test configuration")
                        .as_error()
                        .with_cause(Box::new(e))
                })?;

                tc_err.kind = Some(test_config.kind.to_owned());

                let mut tk = Kind::from_defaults(&test_config.kind, &test_config.kinds)?;

                tk.post_init(&PostInitCtx { dir, name: prefix })?;

                tg.tests.push(Test {
                    name: prefix.to_string(),
                    description: test_contents
                        .description
                        .map(single_linefeed_to_space)
                        .or(tg.description.clone()),
                    timeout: test_config.timeout,
                    max_output: test_config.max_output,
                    kind: tk,
                });
            }
        }

        Ok(tg)
    }
}
