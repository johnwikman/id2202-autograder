use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{
    error::Error,
    utils::{path_absolute_join, path_absolute_parent, path_join, single_linefeed_to_space},
};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TestkindRun {
    pub bin: String,
    pub args: Vec<String>,
    pub stdin: String,
    pub stdin_ignore: bool,
    pub code: Vec<i32>,
    pub stdout: Vec<String>,
    pub stdout_trim: bool,
    pub stdout_strip_whitespace: bool,
    pub stderr: Vec<String>,
    pub stderr_trim: bool,
    pub stderr_strip_whitespace: bool,
    pub input_files: Vec<String>,

    /// Suffixes for automatically discovering input files, e.g. ["*.cpp"]
    pub auto_input_files: Vec<String>,
}

impl TestkindRun {
    const IDENT: &'static str = "run";
}

/// Configuration for running a built binary to generate an assembly file,
/// assembing the generated file, compile it, and run the compiled binary. The
/// output from each stage is checked along the way, only proceeding to the next
/// stage if the previous one was successful.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TestkindGenASMAndRun {
    pub bin: String,
    pub args: Vec<String>,
    pub stdin: String,
    pub stdin_ignore: bool,
    pub code: Vec<i32>,
    pub stderr: Vec<String>,
    pub stderr_trim: bool,
    pub stderr_strip_whitespace: bool,
    pub input_files: Vec<String>,
    /// Suffixes for automatically discovering input files, e.g. ["*.cpp"]
    pub auto_input_files: Vec<String>,

    /// Command for assembing an output file
    pub assemble_cmd: Vec<String>,
    pub assemble_code: Vec<i32>,

    /// Command for compiling the assembled file
    pub compile_cmd: Vec<String>,
    pub compile_code: Vec<i32>,

    /// Options for when running the generated binary
    pub run_cmd: Vec<String>,
    pub run_stdin: String,
    pub run_stdin_ignore: bool,
    pub run_code: Vec<i32>,
    pub run_stdout: Vec<String>,
    pub run_stdout_trim: bool,
    pub run_stdout_strip_whitespace: bool,
    pub run_stderr: Vec<String>,
    pub run_stderr_trim: bool,
    pub run_stderr_strip_whitespace: bool,
}

impl TestkindGenASMAndRun {
    const IDENT: &'static str = "gen_asm_and_run";
}

/// Configuration for checking if a specific file exists, and that it is of the
/// correct MIME type.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TestkindCheckFileExists {
    pub path: String,
    pub mimetype_prefix: String,
    pub mimetype_prefix_ignore: bool,
}

impl TestkindCheckFileExists {
    const IDENT: &'static str = "check_file_exists";
}

#[derive(Deserialize, Debug, Clone)]
pub struct TestkindDefault {
    pub run: TestkindRun,
    pub gen_asm_and_run: TestkindGenASMAndRun,
    pub check_file_exists: TestkindCheckFileExists,
}

impl TestkindDefault {
    /// Returns the set of default toml values associated with the kind
    /// identifier.
    fn toml_from_ident(&self, ident: &str) -> Result<toml::Table, Error> {
        match ident {
            TestkindRun::IDENT => toml::Table::try_from(&self.run).map_err(Error::from),
            TestkindGenASMAndRun::IDENT => {
                toml::Table::try_from(&self.gen_asm_and_run).map_err(Error::from)
            }
            TestkindCheckFileExists::IDENT => {
                toml::Table::try_from(&self.check_file_exists).map_err(Error::from)
            }
            _ => Error::err_identifier(
                ident,
                vec![
                    TestkindRun::IDENT.to_string(),
                    TestkindGenASMAndRun::IDENT.to_string(),
                    TestkindCheckFileExists::IDENT.to_string(),
                ],
            ),
        }
    }
}

/// Enum for representing test kinds.
#[derive(Debug, Clone)]
pub enum Testkind {
    Run(TestkindRun),
    GenASMAndRun(TestkindGenASMAndRun),
    CheckFileExists(TestkindCheckFileExists),
}

impl Testkind {
    /// Automatically discover input files for a test kind. Will look for files
    /// located in `dir` and which has the matching prefix `prefix`. The actual
    /// criteria for automatically discovering a file is specified for each
    /// testkind using the `auto_input_files` field if present.
    fn auto_discover_input_files(&mut self, dir: &str, prefix: &str) -> Result<(), Error> {
        /// Finds input files for the test case based on prefix matching.
        fn find_input_files(
            input_files: &mut Vec<String>,
            auto_input_files: &[String],
            dir: &str,
            prefix: &str,
        ) -> Result<(), Error> {
            let contents = std::fs::read_dir(dir).map_err(|e| {
                Error::fs("listing files for auto discovery", dir).with_cause(Box::new(e))
            })?;
            for entry in contents {
                let filename = entry?
                    .file_name()
                    .to_str()
                    .map(String::from)
                    .ok_or_else(|| {
                        Error::convert("Couldn't get string representation of DirEntry")
                    })?;
                for suffix in auto_input_files {
                    if let Some((p, "")) = filename.rsplit_once(suffix) {
                        if p == prefix {
                            input_files.push(path_join(dir, &filename)?);
                        }
                    }
                }
            }
            Ok(())
        }
        match self {
            Self::Run(t) => find_input_files(&mut t.input_files, &t.auto_input_files, dir, prefix),
            Self::GenASMAndRun(t) => {
                find_input_files(&mut t.input_files, &t.auto_input_files, dir, prefix)
            }
            Self::CheckFileExists(_) => Ok(()),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct TestDefault {
    /// Default timeout for building the project.
    pub timeout_build: u32,

    /// Default timeout for a single test case.
    pub timeout_test: u32,

    /// Maximum total timeout for a grading session.
    pub timeout_total: u32,

    /// Maximum output characters on stdout and stderr for a test case.
    pub max_output: usize,

    /// Truncate output that exceeds this length.
    pub truncate_len: usize,

    /// Number of failed tests to show to the student.
    pub shown_failures: usize,

    /// Default command when building projects
    pub build_cmd: Vec<String>,

    /// If this is set to `true`, then the solution directory is not allowed to
    /// contain any binary files. More specifically, all files have to be text.
    pub build_prohibit_binary_files: bool,

    /// A list of binary files that we are going allow regardless. This could
    /// be a PDF file or .docx which should be part of this submission, but
    /// otherwise forbidden.
    pub build_allowed_binary_files: Vec<String>,

    /// Additional MIME types that do not begin with `"text/"` that shall be
    /// allowed regardless.
    pub build_allowed_binary_mimetypes: Vec<String>,

    pub kind: TestkindDefault,
}

/// Configuration related to building the project for a test tag.
#[derive(Debug, Clone)]
pub struct TagBuildConfig {
    /// The source directory that contains the files to build
    pub srcdir: String,

    /// The command used for building the project once located in the project folder.
    pub cmd: Vec<String>,

    /// Timeout (in seconds) for building the project.
    pub timeout: u32,

    /// If this is true, then the runner will give a build error if there are
    /// any binary files present in the build directory.
    pub prohibit_binary_files: bool,

    /// If prohibit_binary_files is true, then this specifies a list of
    /// exceptions. I.e. binary files that should still be allowed.
    pub allowed_binary_files: Vec<String>,

    /// Additional MIME types that do not begin with `"text/"` that shall be
    /// allowed regardless.
    pub allowed_binary_mimetypes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TagConfig {
    pub name: String,
    pub dirs: Vec<String>,
    pub build: TagBuildConfig,
}

/// A test case to run.
#[derive(Debug, Clone)]
pub struct Test {
    pub name: String,
    pub description: Option<String>,
    pub timeout: u32,
    pub kind: Testkind,
}

/// A group of test cases to run. Can also involve several subtests.
#[derive(Debug, Clone)]
pub struct TestGroup {
    pub title: String,
    pub description: Option<String>,
    pub tests: Vec<Test>,
    pub subgroups: Vec<TestGroup>,
}

/// A test tag that can be invoked and graded.
#[derive(Debug, Clone)]
pub struct Tag {
    pub name: String,
    pub test_groups: Vec<TestGroup>,
    pub build: TagBuildConfig,
}

#[derive(Debug, Clone)]
pub struct Tests {
    pub default: TestDefault,
    pub tag_groups: BTreeMap<String, Vec<Tag>>,
}

impl Tests {
    /// Load test configuration from path
    pub fn load(path: &str) -> Result<Self, Error> {
        // "Hidden" structs that are only used for deserialization
        #[derive(Deserialize, Debug, Clone)]
        struct _UntreatedTests {
            pub default: TestDefault,
            pub tags: BTreeMap<String, toml::Value>,
            pub tag_groups: BTreeMap<String, Vec<String>>,
        }

        #[derive(Deserialize, Debug, Clone)]
        struct _UntreatedExtensibleTag {
            extends: String,
            dirs: Vec<String>,
        }

        #[derive(Deserialize, Debug, Clone)]
        pub struct _UntreatedTagBuild {
            pub srcdir: String,
            pub cmd: Option<Vec<String>>,
            pub timeout: Option<u32>,
            pub prohibit_binary_files: Option<bool>,
            pub allowed_binary_files: Option<Vec<String>>,
            pub allowed_binary_mimetypes: Option<Vec<String>>,
        }

        #[derive(Deserialize, Debug, Clone)]
        struct _UntreatedTag {
            dirs: Vec<String>,
            build: _UntreatedTagBuild,
        }

        log::debug!("Loading root test configuration from {path}");

        let contents: String = std::fs::read_to_string(path)
            .inspect_err(|e| log::error!("Could not load configuration from \"{path}\": {e}"))
            .map_err(Error::from)?;
        let mut ut: _UntreatedTests = toml::from_str(&contents)
            .inspect_err(|e| log::error!("Error parsing configuration from \"{path}\": {e}"))
            .map_err(Error::from)?;

        let root_dir = path_absolute_parent(path)?;

        log::debug!("Extracting each tag configuration");
        let mut tag_configs: BTreeMap<String, TagConfig> = BTreeMap::new();
        while !ut.tags.is_empty() {
            let mut found: Vec<String> = vec![];

            for (name, data) in ut.tags.iter() {
                match data {
                    toml::Value::Table(t) => {
                        if t.contains_key("extends") {
                            let uetg: _UntreatedExtensibleTag =
                                data.to_owned().try_into().map_err(Error::from)?;
                            if let Some(t_found) = tag_configs.get(&uetg.extends) {
                                let t = TagConfig {
                                    name: name.to_owned(),
                                    dirs: [t_found.dirs.to_owned(), uetg.dirs].concat(),
                                    build: t_found.build.to_owned(),
                                };
                                log::debug!("Found tag {t:?}");
                                found.push(name.to_string());
                                tag_configs.insert(name.to_string(), t);
                            }
                        } else {
                            // This is a root tag that doesn't extend anything
                            let utg: _UntreatedTag =
                                data.to_owned().try_into().map_err(Error::from)?;
                            let t = TagConfig {
                                name: name.to_owned(),
                                dirs: utg.dirs,
                                build: TagBuildConfig {
                                    srcdir: utg.build.srcdir,
                                    cmd: utg.build.cmd.unwrap_or(ut.default.build_cmd.clone()),
                                    timeout: utg.build.timeout.unwrap_or(ut.default.timeout_build),
                                    prohibit_binary_files: utg
                                        .build
                                        .prohibit_binary_files
                                        .unwrap_or(ut.default.build_prohibit_binary_files),
                                    allowed_binary_files: utg
                                        .build
                                        .allowed_binary_files
                                        .unwrap_or(ut.default.build_allowed_binary_files.clone()),
                                    allowed_binary_mimetypes: utg
                                        .build
                                        .allowed_binary_mimetypes
                                        .unwrap_or(
                                            ut.default.build_allowed_binary_mimetypes.clone(),
                                        ),
                                },
                            };
                            log::debug!("Found tag {t:?}");
                            found.push(name.to_string());
                            tag_configs.insert(name.to_string(), t);
                        }
                    }
                    _ => {
                        return Err(Error::test_config_msg("tag specification must be a table")
                            .tag(name)
                            .path(path)
                            .into());
                    }
                }
            }

            if found.is_empty() {
                return Err(Error::test_config_msg(format!(
                    "Could not instantiate tag configuration. Remaining keys: {:?}",
                    ut.tags
                ))
                .path(path)
                .into());
            } else {
                log::debug!("Removing all found keys");
                for k in found {
                    ut.tags.remove(&k);
                }
            }
        }

        log::debug!("Converting all configuration to tags and instantiating the tag groups");
        let mut tags: BTreeMap<String, Tag> = BTreeMap::new();
        for (k, v) in tag_configs.iter() {
            // While this technically is a .map() operation, we do it as a loop
            // to propagate the error from .to_tag().
            tags.insert(k.to_owned(), v.to_tag(&ut.default, &root_dir)?);
        }

        let mut tag_groups: BTreeMap<String, Vec<Tag>> = tags
            .iter()
            .map(|(k, t)| (k.to_owned(), vec![t.to_owned()]))
            .collect();
        for (k, lst) in ut.tag_groups.iter() {
            if tag_groups.contains_key(k) {
                return Err(Error::test_config_msg("duplicate tag name")
                    .tag(k)
                    .path(path)
                    .into());
            }
            if lst.is_empty() {
                return Err(Error::test_config_msg("empty tag group")
                    .tag(k)
                    .path(path)
                    .into());
            }
            let mut gtags: Vec<Tag> = Vec::new();
            for tname in lst {
                let t = tags.get(tname).ok_or_else(|| {
                    Error::from(
                        Error::test_config_msg(format!("unknown tag {tname} in tag group"))
                            .tag(k)
                            .path(path),
                    )
                })?;
                gtags.push(t.to_owned());
            }
            tag_groups.insert(k.to_owned(), gtags);
        }

        Ok(Tests {
            default: ut.default,
            tag_groups: tag_groups,
        })
    }
}

// Used to track the default setup keys for a test case
#[derive(Deserialize, Debug, Clone)]
struct _UntreatedTest {
    pub kind: Option<String>,
    pub timeout: Option<u32>,
    pub options: Option<toml::Table>,
}

impl _UntreatedTest {
    /// Merges two untreated test configs, with config specified in `other`
    /// overriding the config specified in `self`.
    fn merge(self: &Self, other: &Option<Self>) -> Self {
        if let Some(ut) = other {
            let mut new_ut = self.clone();
            new_ut.kind = ut.kind.clone().or_else(|| self.kind.clone());
            new_ut.timeout = ut.timeout.or(self.timeout);
            if let Some(opts) = &ut.options {
                if let Some(ut_opts) = &self.options {
                    // Override options from ut_opts
                    let mut new_opts = ut_opts.to_owned();
                    for (k, v) in opts.iter() {
                        new_opts.insert(k.to_owned(), v.to_owned());
                    }
                    new_ut.options = Some(new_opts);
                } else {
                    // No default options, use the ones from this test group
                    new_ut.options = Some(opts.to_owned());
                }
            }
            new_ut
        } else {
            self.clone()
        }
    }
}

impl TagConfig {
    /// Instantiates a tag from a configuration.
    ///
    /// The `defaults` variable provide standard default values to provide for
    /// each test kind and build variables that may be absent.
    /// The `root_dir` is the directory from which every path is relative to.
    fn to_tag(self: &Self, defaults: &TestDefault, root_dir: &str) -> Result<Tag, Error> {
        log::debug!("Instantiating tag \"{}\"", self.name);
        if !tag_is_valid(&self.name) {
            return Err(Error::test_config_msg("invalid tag name")
                .tag(&self.name)
                .into());
        }

        let mut t = Tag {
            name: self.name.to_owned(),
            test_groups: vec![],
            build: self.build.to_owned(),
        };

        log::debug!("Converting each directory to a test group");
        for dir in self.dirs.iter() {
            log::debug!("Scanning directory {dir}");
            let absdir = path_absolute_join(root_dir, &dir)?;
            t.test_groups.push(TestGroup::new(
                &absdir,
                defaults,
                &_UntreatedTest {
                    kind: None,
                    timeout: None,
                    options: None,
                },
                vec![],
            )?);
        }

        Ok(t)
    }
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
    /// - `defaults`: Default configuration to use for absent values
    /// - `test_defaults`: Test defaults which subsequent config.toml and
    ///                    .test.toml files extends.
    /// - `numbering`: Sequence of numbers that keeps that track of numbering
    ///                for test group titles.
    fn new(
        dir: &str,
        defaults: &TestDefault,
        test_defaults: &_UntreatedTest,
        numbering: Vec<i32>,
    ) -> Result<TestGroup, Error> {
        log::debug!("Creating test group from directory {dir}");

        #[derive(Deserialize, Debug, Clone)]
        struct _UntreatedTestGroup {
            pub title: Option<String>,
            pub description: Option<String>,
            pub include: Option<Vec<String>>,
            pub test: Option<_UntreatedTest>,
        }

        let config_path = path_join(dir, "config.toml")?;

        let mut tc_err = Error::test_config().path(&config_path);

        let contents: String = std::fs::read_to_string(&config_path).map_err(|e| {
            tc_err
                .to_owned()
                .msg("could not read into string")
                .as_error()
                .with_cause(Box::new(e))
        })?;
        let utg: _UntreatedTestGroup = toml::from_str(&contents).map_err(|e| {
            tc_err
                .to_owned()
                .msg("could not deserialize toml")
                .as_error()
                .with_cause(Box::new(e))
        })?;

        // Setting up the defaults for this test group
        let testgroup_defaults = test_defaults.merge(&utg.test);

        // Build title with numbering prefix (e.g., "1.2.3. Title")
        let title = utg
            .title
            .ok_or_else(|| tc_err.to_owned().msg("missing title for test group"))?;
        let prefix: String = numbering.iter().map(|i| format!("{i}.")).collect();
        let tg_title = if prefix.is_empty() {
            title
        } else {
            format!("{prefix} {title}")
        };

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
                tg.subgroups.push(TestGroup::new(
                    &subdir,
                    defaults,
                    &testgroup_defaults,
                    new_numbering,
                )?);
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

                let testkind_opts = testgroup_defaults.clone().merge(&test_contents.test);

                let kind_ident = testkind_opts
                    .kind
                    .ok_or_else(|| tc_err.to_owned().msg("no test kind provided"))?;

                tc_err.kind = Some(kind_ident.to_owned());

                let opts = testkind_opts.options.unwrap_or_else(toml::Table::new);

                // Override options in existing defaults
                let mut run_opts = defaults.kind.toml_from_ident(&kind_ident).map_err(|e| {
                    tc_err
                        .to_owned()
                        .msg("could not get defaults")
                        .as_error()
                        .with_cause(Box::new(e))
                })?;
                // If any option has an "ignore" flag, we set that to false by
                // default if specified
                for k in opts.keys() {
                    let ignore_key = format!("{k}_ignore");
                    match run_opts.get(&ignore_key) {
                        Some(toml::Value::Boolean(_)) => {
                            run_opts.insert(ignore_key, toml::Value::Boolean(false));
                        }
                        _ => {}
                    }
                }
                for (k, v) in opts {
                    if !run_opts.contains_key(&k) {
                        return Err(tc_err.msg("invalid test.option key").key(k).into());
                    }
                    run_opts.insert(k.clone(), v.clone());
                }
                let mut tk = match kind_ident.as_str() {
                    TestkindRun::IDENT => Testkind::Run(run_opts.try_into()?),
                    TestkindGenASMAndRun::IDENT => Testkind::GenASMAndRun(run_opts.try_into()?),
                    TestkindCheckFileExists::IDENT => {
                        Testkind::CheckFileExists(run_opts.try_into()?)
                    }
                    _ => return Err(tc_err.msg("invalid test kind").into()),
                };

                tk.auto_discover_input_files(dir, prefix)?;

                tg.tests.push(Test {
                    name: prefix.to_string(),
                    description: test_contents
                        .description
                        .map(single_linefeed_to_space)
                        .or(tg.description.clone()),
                    timeout: testkind_opts.timeout.unwrap_or(defaults.timeout_test),
                    kind: tk,
                });
            }
        }

        Ok(tg)
    }
}

/// Matches the string `s` if it contains any leading tag. Returns a tuple with
/// the matched tag and the remaining text after the tag.
pub fn tag_match(s: &str) -> (&str, &str) {
    for (i, c) in s.chars().enumerate() {
        // Could technically use a regex here instead, but could not find a
        // lightweight and well-documented regex library that allows for
        // compile-time evaluation.
        if ('0' <= c && c <= '9')
            || ('A' <= c && c <= 'Z')
            || ('a' <= c && c <= 'z')
            || c == '-'
            || c == '_'
        {
            () // OK
        } else {
            // Invalid character, here the tag ends.
            return s.split_at(i);
        }
    }
    (s, "")
}

/// Returns `true` if the tag is valid, otherwise `false`.
pub fn tag_is_valid(tag: &str) -> bool {
    if tag.is_empty() {
        return false;
    }
    let (_, rest) = tag_match(tag);
    rest.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use asserting::prelude::*;

    /// Path to the example tests.toml file (relative to project root)
    const EXAMPLE_TESTS_TOML: &str =
        concat!(env!("CARGO_MANIFEST_DIR"), "/example/tests/tests.toml");

    #[test]
    fn test_load_example_tests_toml() {
        let tests = Tests::load(EXAMPLE_TESTS_TOML).expect("Failed to load example tests.toml");

        // Verify default values are loaded correctly
        assert_that!(tests.default.timeout_build).is_equal_to(60);
        assert_that!(tests.default.timeout_test).is_equal_to(60);
        assert_that!(tests.default.timeout_total).is_equal_to(1200);
        assert_that!(tests.default.max_output).is_equal_to(4194304);
        assert_that!(tests.default.truncate_len).is_equal_to(2000);
        assert_that!(tests.default.shown_failures).is_equal_to(3);
        assert_that!(tests.default.build_prohibit_binary_files).is_true();
    }

    #[test]
    fn test_example_tags_exist() {
        let tests = Tests::load(EXAMPLE_TESTS_TOML).expect("Failed to load example tests.toml");

        // Verify all expected tags exist
        assert_that!(tests.tag_groups.contains_key("hello")).is_true();
        assert_that!(tests.tag_groups.contains_key("hello-extra")).is_true();
        assert_that!(tests.tag_groups.contains_key("hello-asm")).is_true();
        assert_that!(tests.tag_groups.contains_key("hello-file")).is_true();
        assert_that!(tests.tag_groups.contains_key("hello-all")).is_true();
    }

    #[test]
    fn test_example_tag_group_hello_all() {
        let tests = Tests::load(EXAMPLE_TESTS_TOML).expect("Failed to load example tests.toml");

        // Verify hello-all tag group contains all expected tags
        let hello_all = tests
            .tag_groups
            .get("hello-all")
            .expect("hello-all tag group not found");
        assert_that!(hello_all.len()).is_equal_to(4);

        let tag_names: Vec<&str> = hello_all.iter().map(|t| t.name.as_str()).collect();
        assert_that!(tag_names.contains(&"hello")).is_true();
        assert_that!(tag_names.contains(&"hello-extra")).is_true();
        assert_that!(tag_names.contains(&"hello-asm")).is_true();
        assert_that!(tag_names.contains(&"hello-file")).is_true();
    }

    #[test]
    fn test_example_hello_tag_has_tests() {
        let tests = Tests::load(EXAMPLE_TESTS_TOML).expect("Failed to load example tests.toml");

        let hello_tags = tests.tag_groups.get("hello").expect("hello tag not found");
        assert_that!(hello_tags.len()).is_equal_to(1);

        let hello_tag = &hello_tags[0];
        assert_that!(hello_tag.name.as_str()).is_equal_to("hello");
        assert_that!(hello_tag.test_groups.is_empty()).is_false();

        // The hello tag should have at least one test
        let total_tests: usize = hello_tag.test_groups.iter().map(|g| g.tests.len()).sum();
        assert_that!(total_tests).is_greater_than(0);
    }

    #[test]
    fn test_example_build_config() {
        let tests = Tests::load(EXAMPLE_TESTS_TOML).expect("Failed to load example tests.toml");

        let hello_tags = tests.tag_groups.get("hello").expect("hello tag not found");
        let hello_tag = &hello_tags[0];

        // Verify build configuration
        assert_that!(hello_tag.build.srcdir.as_str()).is_equal_to("solutions/hello");
        assert_eq!(hello_tag.build.cmd, vec!["make"]);

        // Test the hello-extra tag
        let hello_extra = tests
            .tag_groups
            .get("hello-extra")
            .expect("hello-extra tag not found");
        let hetg = &hello_extra[0].test_groups[0];
        assert_eq!(hetg.title, "Hello (Extra tests)");
        assert_eq!(hetg.tests.len(), 0);
        assert_eq!(hetg.subgroups.len(), 3);

        // Should be parsed in lexicographical order, so first should be
        // file-cpp and then file-md.
        let he_2file = &hetg.subgroups[1];
        assert_eq!(he_2file.title, "2. File Input");
        let Testkind::Run(he_2f_t0) = &he_2file.tests[0].kind else {
            panic!("Expected Testkind::Run");
        };
        assert_eq!(he_2f_t0.auto_input_files, vec![".cpp"]);
        assert_eq!(
            he_2f_t0
                .input_files
                .iter()
                .map(|p| std::path::Path::new(p)
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap())
                .collect::<Vec<&str>>(),
            vec!["file-cpp.cpp"]
        );
        let Testkind::Run(he_2f_t1) = &he_2file.tests[1].kind else {
            panic!("Expected Testkind::Run");
        };
        assert_eq!(he_2f_t1.auto_input_files, vec![".md"]);
        assert_eq!(
            he_2f_t1
                .input_files
                .iter()
                .map(|p| std::path::Path::new(p)
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap())
                .collect::<Vec<&str>>(),
            vec!["file-md.md"]
        );
    }

    #[test]
    fn test_example_default_kind_run() {
        let tests = Tests::load(EXAMPLE_TESTS_TOML).expect("Failed to load example tests.toml");

        // Verify default kind.run configuration
        let run_config = &tests.default.kind.run;
        assert_that!(run_config.bin.as_str()).is_equal_to("cigrid");
        assert_that!(&run_config.code).contains_exactly(&[0i32]);
        assert_that!(run_config.stdin_ignore).is_true();
        assert_that!(run_config.stdout_trim).is_true();
        assert_that!(run_config.stderr_trim).is_true();
        assert_eq!(run_config.auto_input_files, vec![".cpp"]);
    }

    #[test]
    fn test_example_default_kind_gen_asm_and_run() {
        let tests = Tests::load(EXAMPLE_TESTS_TOML).expect("Failed to load example tests.toml");

        // Verify default kind.gen_asm_and_run configuration
        let asm_config = &tests.default.kind.gen_asm_and_run;
        assert_that!(asm_config.bin.as_str()).is_equal_to("cigrid");
        assert_eq!(asm_config.args, vec!["--asm"]);
        assert_that!(&asm_config.assemble_code).contains_exactly(&[0i32]);
        assert_that!(&asm_config.compile_code).contains_exactly(&[0i32]);
        assert_that!(&asm_config.run_code).contains_exactly(&[0i32]);
    }

    #[test]
    fn test_example_allowed_binary_files() {
        let tests = Tests::load(EXAMPLE_TESTS_TOML).expect("Failed to load example tests.toml");

        // Verify allowed binary files
        assert_that!(tests
            .default
            .build_allowed_binary_files
            .contains(&"regalloc.pdf".to_string()))
        .is_true();
        assert_that!(tests
            .default
            .build_allowed_binary_files
            .contains(&"liveness.pdf".to_string()))
        .is_true();

        // Verify allowed binary mimetypes
        assert_that!(tests
            .default
            .build_allowed_binary_mimetypes
            .contains(&"application/pdf".to_string()))
        .is_true();
        assert_that!(tests
            .default
            .build_allowed_binary_mimetypes
            .contains(&"application/javascript".to_string()))
        .is_true();
        assert_that!(tests
            .default
            .build_allowed_binary_mimetypes
            .contains(&"application/json".to_string()))
        .is_true();
    }

    /// Test cases for tag_match and tag_is_valid.
    /// Each tuple is (input, expected_tag, expected_remainder).
    /// tag_is_valid is derived: true when remainder is empty.
    const TAG_TEST_CASES: &[(&str, &str, &str)] = &[
        // Basic cases
        ("hello 5", "hello", " 5"),
        (" hello 5", "", " hello 5"),
        ("some-thing. else", "some-thing", ". else"),
        ("A_B_c_Zz-009", "A_B_c_Zz-009", ""),
        ("hello", "hello", ""),
        ("hello5", "hello5", ""),
        // Single characters
        ("a", "a", ""),
        ("Z", "Z", ""),
        ("5", "5", ""),
        ("-", "-", ""),
        ("_", "_", ""),
        (".", "", "."),
        // Numbers at start
        ("123abc", "123abc", ""),
        ("123", "123", ""),
        ("0-tag", "0-tag", ""),
        ("1st-tag", "1st-tag", ""),
        // Only special valid characters
        ("---", "---", ""),
        ("___", "___", ""),
        ("-_-_-", "-_-_-", ""),
        ("-_-", "-_-", ""),
        // Whitespace delimiters
        ("tag\ttab", "tag", "\ttab"),
        ("tag\nnewline", "tag", "\nnewline"),
        ("tag\r\nwindows", "tag", "\r\nwindows"),
        ("tag\n", "tag", "\n"),
        ("\t", "", "\t"),
        ("\n", "", "\n"),
        ("\r\n", "", "\r\n"),
        (" ", "", " "),
        ("  ", "", "  "),
        ("\t\t", "", "\t\t"),
        (" tag", "", " tag"),
        ("\ttag", "", "\ttag"),
        ("\ntag", "", "\ntag"),
        ("tag ", "tag", " "),
        ("tag\t", "tag", "\t"),
        ("tag \t\n", "tag", " \t\n"),
        ("a b c", "a", " b c"),
        ("tag\x0b", "tag", "\x0b"), // vertical tab
        ("tag\x0c", "tag", "\x0c"), // form feed
        // Unicode characters (should stop at them)
        ("tagåäö", "tag", "åäö"),
        ("héllo", "h", "éllo"),
        ("日本語", "", "日本語"),
        ("tägname", "t", "ägname"),
        ("tag™", "tag", "™"),
        // Special characters as delimiters
        ("tag@email", "tag", "@email"),
        ("tag/path", "tag", "/path"),
        ("tag:value", "tag", ":value"),
        ("tag=value", "tag", "=value"),
        ("tag.name", "tag", ".name"),
        ("!! hello", "", "!! hello"),
        ("#hello", "", "#hello"),
    ];

    #[test]
    fn test_tag_match() {
        // Empty string is a special case
        assert_eq!(tag_match(""), ("", ""));

        for &(input, expected_tag, expected_rest) in TAG_TEST_CASES {
            assert_eq!(
                tag_match(input),
                (expected_tag, expected_rest),
                "tag_match({input:?})"
            );
        }
    }

    #[test]
    fn test_tag_is_valid() {
        // Empty string is a special case: not a valid tag
        assert!(!tag_is_valid(""));

        for &(input, _, expected_rest) in TAG_TEST_CASES {
            let expected_valid = expected_rest.is_empty();
            assert_eq!(
                tag_is_valid(input),
                expected_valid,
                "tag_is_valid({input:?})"
            );
        }
    }
}
