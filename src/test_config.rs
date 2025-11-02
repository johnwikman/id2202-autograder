use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use toml;

use crate::{
    error::Error,
    utils::{path_absolute_join, path_absolute_parent, path_join, single_linefeed_to_space},
};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TestkindBaseRun {
    pub bin: String,
    pub args: Vec<String>,
    pub code: i32,
    pub ignore_stdin: bool,
    pub stdin: String,
    pub ignore_stdout: bool,
    pub trim_stdout: bool,
    pub strip_whitespace_stdout: bool,
    pub stdout: String,
    pub ignore_stderr: bool,
    pub trim_stderr: bool,
    pub strip_whitespace_stderr: bool,
    pub stderr: String,
}

/// Configuration for running the build binary and checking its output.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TestkindConfigRun {
    #[serde(flatten)]
    pub base: TestkindBaseRun,

    /// Suffixes for automatically discovering input files
    pub auto_input_files: Vec<String>,
}

/// Same as `TestkindConfigRun`, but with the input file instantiated to an
/// actual file (if applicable).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TestkindRun {
    #[serde(flatten)]
    pub base: TestkindBaseRun,

    /// An optional input file to provide to the program
    pub input_file: Option<String>,
}

impl TestkindRun {
    const IDENT: &'static str = "run";
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TestkindBaseGenASMAndRun {
    pub bin: String,
    pub args: Vec<String>,
    pub code: i32,
    pub ignore_stdin: bool,
    pub stdin: String,
    pub ignore_stderr: bool,
    pub trim_stderr: bool,
    pub strip_whitespace_stderr: bool,
    pub stderr: String,

    /// Command for assembing an output file
    pub assemble_cmd: Vec<String>,
    pub assemble_code: i32,

    /// Command for compiling the assembled file
    pub compile_cmd: Vec<String>,
    pub compile_code: i32,

    /// Options for when running the generated binary
    pub run_cmd: Vec<String>,
    pub run_code: i32,
    pub run_ignore_stdin: bool,
    pub run_stdin: String,
    pub run_ignore_stdout: bool,
    pub run_trim_stdout: bool,
    pub run_stdout: String,
    pub run_strip_whitespace_stdout: bool,
    pub run_ignore_stderr: bool,
    pub run_trim_stderr: bool,
    pub run_strip_whitespace_stderr: bool,
    pub run_stderr: String,
}

/// Configuration for running a built binary to generate an assembly file,
/// assembing the generated file, compile it, and run the compiled binary. The
/// output from each stage is checked along the way, only proceeding to the next
/// stage if the previous one was successful.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TestkindConfigGenASMAndRun {
    #[serde(flatten)]
    pub base: TestkindBaseGenASMAndRun,

    pub auto_input_files: Vec<String>,
}

/// Same as `TestkindConfigGenASMAndRun`, but with the input and output file
/// instantiated to actual files.
#[derive(Deserialize, Debug, Clone)]
pub struct TestkindGenASMAndRun {
    #[serde(flatten)]
    pub base: TestkindBaseGenASMAndRun,

    pub input_file: String,
}

impl TestkindGenASMAndRun {
    const IDENT: &'static str = "gen_asm_and_run";
}

/// Same as `TestkindConfigRun`, but with the input file instantiated to an
/// actual file (if applicable).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TestkindCheckFileExists {
    pub path: String,
    pub ignore_mimetype: bool,
    pub mimetype_prefix: String,
}

impl TestkindCheckFileExists {
    const IDENT: &'static str = "check_file_exists";
}

#[derive(Deserialize, Debug, Clone)]
pub struct TestkindDefault {
    pub run: TestkindConfigRun,
    pub gen_asm_and_run: TestkindConfigGenASMAndRun,
    pub check_file_exists: TestkindCheckFileExists,
}

/// Enum for representing test kinds.
#[derive(Debug, Clone)]
pub enum Testkind {
    Run(TestkindRun),
    GenASMAndRun(TestkindGenASMAndRun),
    CheckFileExists(TestkindCheckFileExists),
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
        while ut.tags.len() > 0 {
            let mut found: Vec<String> = vec![];

            for (name, data) in ut.tags.iter() {
                match data {
                    toml::Value::Table(t) => {
                        if t.contains_key("extends") {
                            let uetg: _UntreatedExtensibleTag =
                                data.to_owned().try_into().map_err(Error::from)?;
                            match tag_configs.get(&uetg.extends) {
                                Some(t_found) => {
                                    let t = TagConfig {
                                        name: name.to_owned(),
                                        dirs: [t_found.dirs.to_owned(), uetg.dirs].concat(),
                                        build: t_found.build.to_owned(),
                                    };
                                    log::debug!("Found tag {t:?}");
                                    found.push(name.to_string());
                                    tag_configs.insert(name.to_string(), t);
                                }
                                None => {}
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
                                        .unwrap_or(ut.default.build_prohibit_binary_files.clone()),
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
                        let errmsg = "tag specification must be a table";
                        log::error!("{errmsg}");
                        return Err(Error::from(errmsg));
                    }
                }
            }

            if found.len() == 0 {
                let errmsg = format!(
                    "Could not instantiate tag configuration. Remaining keys: {:?}",
                    ut.tags
                );
                log::error!("{errmsg}");
                return Err(Error::from(errmsg));
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
                return Error::err_string(format!("Duplicate tag name {k}"));
            }
            if lst.len() == 0 {
                return Error::err_string(format!("Tag group {k} is empty"));
            }
            let mut gtags: Vec<Tag> = Vec::new();
            for tname in lst {
                let t = tags
                    .get(tname)
                    .ok_or(Error::from(format!("Unknown tag {tname} in tag group {k}")))?;
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
            new_ut.kind = ut
                .kind
                .as_ref()
                .map(|k| k.to_owned())
                .or(self.kind.to_owned());
            new_ut.timeout = ut
                .timeout
                .as_ref()
                .map(|k| k.to_owned())
                .or(self.timeout.to_owned());
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

        let contents: String = std::fs::read_to_string(&config_path)
            .inspect_err(|e| log::error!("Could not load {config_path}: {e}"))?;
        let utg: _UntreatedTestGroup = toml::from_str(&contents)
            .inspect_err(|e| log::error!("Error deserialzing {config_path}: {e}"))?;

        // Setting up the defaults for this test group
        let testgroup_defaults = test_defaults.merge(&utg.test);

        // Set up title prefix
        let mut tg_title: String = if numbering.len() == 0 {
            "".to_string()
        } else {
            let mut s = numbering
                .iter()
                .map(|i| format!("{i}."))
                .collect::<String>();
            s.push(' ');
            s
        };

        // Add the title name
        tg_title.push_str(
            utg.title
                .ok_or(format!("Missing title for test group under {config_path}"))?
                .as_str(),
        );

        let mut tg = TestGroup {
            title: tg_title,
            description: utg.description.map(single_linefeed_to_space),
            tests: vec![],
            subgroups: vec![],
        };

        // Find all the test cases in the same directory
        // filename: [(fname: String, is_dir: Bool), ...]
        let mut filenames: Vec<(String, bool)> = vec![];
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let filename: String = entry
                .file_name()
                .to_str()
                .map(String::from)
                .ok_or("Couldn't get string representation of DirEntry")?;
            filenames.push((filename, entry.metadata()?.is_dir()));
        }

        // Sort the listed directories
        filenames.sort_by(|(lf, _), (rf, _)| lf.cmp(rf));

        // Add any included directories
        // (there are checked last, in the specified order)
        if let Some(dirs) = utg.include {
            for d in dirs.iter() {
                let d_path = path_absolute_join(dir, d)?;
                if std::path::Path::new(&d_path).is_dir() {
                    filenames.push((d_path, true))
                } else {
                    return Error::err_string(format!("The included test {d} is not a directory"));
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

                let contents: String = std::fs::read_to_string(&testfile_path)?;
                let test_contents: _UntreatedTestGroup = toml::from_str(&contents)?;

                let testkind_opts = testgroup_defaults.clone().merge(&test_contents.test);

                if let Some(kind_ident) = testkind_opts.kind {
                    let opts: toml::Table = testkind_opts.options.unwrap_or(toml::Table::new());

                    /// Finds input files for the test case based on prefix
                    /// matching. This is common functionality for all test
                    /// kinds, so lifted it out to its own function.
                    fn find_input_files(
                        auto_input_files: &Vec<String>,
                        dir: &str,
                        prefix: &str,
                    ) -> Result<Vec<String>, Error> {
                        let mut input_files: Vec<String> = vec![];
                        for infile_entry in std::fs::read_dir(dir)? {
                            let filename: String = infile_entry?
                                .file_name()
                                .to_str()
                                .map(String::from)
                                .ok_or("Couldn't get string representation of DirEntry")?;
                            for valid_suffix in auto_input_files.iter() {
                                if let Some((infile_prefix, "")) =
                                    filename.rsplit_once(valid_suffix)
                                {
                                    if infile_prefix == prefix {
                                        input_files.push(path_join(dir, &filename)?);
                                    }
                                }
                            }
                        }
                        Ok(input_files)
                    }

                    /// Override options from an existing table with new options.
                    fn override_opts(
                        kind_ident: &str,
                        src_file: &str,
                        existing_opts: &mut toml::Table,
                        override_opts: &toml::Table,
                    ) -> Result<(), Error> {
                        for (k, v) in override_opts.iter() {
                            if !existing_opts.contains_key(k) {
                                return Err(Error::from(format!(
                                    "invalid test.option key \"{k}\" for test kind \"{}\" in file: {}",
                                    kind_ident,
                                    src_file,
                                )));
                            }
                            existing_opts.insert(k.to_owned(), v.to_owned());
                        }
                        Ok(())
                    }

                    let tk: Testkind = if kind_ident == TestkindRun::IDENT {
                        let mut run_opts: toml::Table =
                            toml::Table::try_from(&defaults.kind.run).map_err(Error::from)?;

                        override_opts(&kind_ident, &testfile_path, &mut run_opts, &opts)?;

                        let tkrun: TestkindConfigRun = run_opts.try_into()?;

                        let input_files: Vec<String> =
                            find_input_files(&tkrun.auto_input_files, dir, prefix)?;
                        if input_files.len() > 1 {
                            return Error::err_string(format!("Found multiple input files for test case {testfile_path}. Only a single input file is allowed"));
                        }
                        Testkind::Run(TestkindRun {
                            base: tkrun.base,
                            input_file: input_files.get(0).map(String::to_owned),
                        })
                    } else if kind_ident == TestkindGenASMAndRun::IDENT {
                        let mut genasmrun_opts: toml::Table =
                            toml::Table::try_from(&defaults.kind.gen_asm_and_run)
                                .map_err(Error::from)?;

                        override_opts(&kind_ident, &testfile_path, &mut genasmrun_opts, &opts)?;

                        let tkasm: TestkindConfigGenASMAndRun = genasmrun_opts.try_into()?;

                        let input_files: Vec<String> =
                            find_input_files(&tkasm.auto_input_files, dir, prefix)?;
                        if input_files.len() != 1 {
                            return Error::err_string(format!("Must provide exactly one input file for test case {}. Found {} input files.", testfile_path, input_files.len()));
                        }
                        Testkind::GenASMAndRun(TestkindGenASMAndRun {
                            base: tkasm.base,
                            input_file: input_files
                                .get(0)
                                .map(String::to_owned)
                                .ok_or("internal error at input_file for Testkind::GenASMAndRun")?,
                        })
                    } else if kind_ident == TestkindCheckFileExists::IDENT {
                        let mut checkfile_opts: toml::Table =
                            toml::Table::try_from(&defaults.kind.check_file_exists)
                                .map_err(Error::from)?;

                        override_opts(&kind_ident, &testfile_path, &mut checkfile_opts, &opts)?;

                        let tkfile: TestkindCheckFileExists = checkfile_opts.try_into()?;

                        Testkind::CheckFileExists(tkfile)
                    } else {
                        return Error::err_string(format!(
                            "Invalid test kind identifier \"{kind_ident}\""
                        ));
                    };
                    let t = Test {
                        name: prefix.to_string(),
                        description: test_contents
                            .description
                            .map(single_linefeed_to_space)
                            .or(tg.description.to_owned()),
                        timeout: testkind_opts.timeout.unwrap_or(defaults.timeout_test),
                        kind: tk,
                    };
                    tg.tests.push(t);
                } else {
                    return Error::err_string(format!("No test kind provided for {testfile_path}"));
                }
            }
        }

        Ok(tg)
    }
}
