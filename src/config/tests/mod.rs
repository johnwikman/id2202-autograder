use documented::{Documented, DocumentedFields};
use schemars::JsonSchema;
use serde::Deserialize;
use smart_default::SmartDefault;
use std::collections::BTreeMap;

use crate::{error::Error, utils::path_absolute_parent};

pub mod group;
pub mod kind;
pub mod tag;

use group::TestConfig;
use tag::{BuildConfig, Tag, TagDefaults};

/// The test system operates in two phases: **build** and **test**. First, the
/// submitted code is compiled inside a container using a configured build
/// command. Then, individual test cases are executed against the resulting
/// binary and their outputs are verified.
///
/// A **tag** is the unit of grading. It defines what to build (source directory,
/// build command) and which test cases to run. Multiple tags can be grouped
/// under a **tag group** so they can be invoked together:
///
/// ```toml
/// [tag_groups]
/// all = ["task1", "task2", "task3"]
/// ```
///
/// A **test case** is a single verification step. Each test case has a **kind**
/// that determines what it does — for example, running a binary and checking its
/// output, running a multi-stage assembly pipeline, or verifying that a file
/// exists with a particular type. The available kinds are documented below.
///
/// ## Configuration files and inheritance
///
/// Test configuration lives in a directory tree rooted at a TOML file whose path
/// is given by `runner.test_config` in the settings. Three file types are used:
///
/// - **Root test configuration** — the top-level TOML file (any name). Defines
///   global defaults (including per-test-kind defaults), tags, and tag groups.
/// - **`config.toml`** — placed in a test directory. Sets shared configuration
///   for all test cases in that directory and its subdirectories.
/// - **`*.test.toml`** — defines a single test case. The filename (minus the
///   `.test.toml` suffix) becomes the test name.
///
/// Configuration is inherited hierarchically. A `.test.toml` file inherits from
/// the `config.toml` in the same directory, which inherits from the
/// `config.toml` in its parent directory, and so on up to the global defaults in
/// the root test configuration. Only explicitly set values override inherited
/// ones. Each directory containing a `config.toml` forms a **test group**, and a
/// directory nested under it forms a sub-group.
///
/// ```text
/// tests/
///   tests.toml          # [default.kind.run] sets bin = "mybin", stdout_trim = true
///   hello/
///     config.toml       # title = "Hello", [test] sets kind = "run"
///     basic.test.toml   # [test.options] sets stdout = ["Hello"]
///     advanced/
///       config.toml     # title = "Advanced", [test.options] sets args = ["--verbose"]
///       full.test.toml  # [test.options] sets stdout = ["Hello, World!"]
/// ```
///
/// Here `basic.test.toml` inherits `kind = "run"` from `hello/config.toml` and
/// `bin = "mybin"` from the global defaults, so it only needs to specify
/// `stdout`. `full.test.toml` additionally picks up `args = ["--verbose"]` from
/// `advanced/config.toml`.
// `trim = false` preserves the indentation inside the fenced directory-tree
// block above (documented trims each line by default, which would flatten it).
#[derive(Debug, Clone, Documented)]
#[documented(trim = false)]
pub struct Tests {
    pub default: Defaults,

    /// All grading tags that can be graded by a submission job.
    pub tags: BTreeMap<String, Tag>,

    /// Lookup of _requested grading tag_ to actual grading tags. This includes
    /// tag groups (one to many), tag aliases (one to one), but also the
    /// identity lookup of each entry within `tags`.
    ///
    /// ```rust
    /// // If something is contained in tags, it must also be
    /// // contained in the tag_resolution.
    /// assert!(tags.contains_key("hello"))
    /// assert_eq!(tag_resolution.get("hello"), vec!["hello"])
    /// ```
    pub tag_resolution: BTreeMap<String, Vec<String>>,
}

/// Global defaults inherited by all tags and test cases unless overridden.
#[derive(Deserialize, JsonSchema, Debug, Clone, Documented, DocumentedFields)]
pub struct Defaults {
    /// Defaults applying to a tag as a whole.
    pub tag: TagDefaults,

    /// Defaults for the build phase.
    pub build: BuildConfig,

    /// Defaults for individual test cases.
    pub test: TestConfig,
}

/// Options to set when loading tests.
#[derive(Debug, SmartDefault)]
pub struct TestsLoadingOptions {
    /// Only include information about the tags themselves, and skip loading
    /// any of the tests cases. This will cause the `test_groups` field under
    /// `Tag` to be empty vectors for all tags, and the tests will not be
    /// checked as well.
    #[default = false]
    pub taginfo_only: bool,
}
impl AsRef<TestsLoadingOptions> for TestsLoadingOptions {
    fn as_ref(&self) -> &TestsLoadingOptions {
        self
    }
}

impl Tests {
    /// Load test configuration from `path`.
    pub fn load(path: &str, options: impl AsRef<TestsLoadingOptions>) -> Result<Self, Error> {
        // "Hidden" structs that are only used for deserialization
        #[derive(Deserialize, Debug, Clone)]
        struct _UntreatedTests {
            pub default: Defaults,
            pub tags: BTreeMap<String, toml::Value>,
            pub tag_groups: BTreeMap<String, Vec<String>>,
        }

        let options = options.as_ref();

        log::debug!("Loading root test configuration from {path}");

        let contents: String = std::fs::read_to_string(path)
            .inspect_err(|e| log::error!("Could not load configuration from \"{path}\": {e}"))
            .map_err(Error::from)?;
        let ut: _UntreatedTests = toml::from_str(&contents)
            .inspect_err(|e| log::error!("Error parsing configuration from \"{path}\": {e}"))
            .map_err(Error::from)?;

        log::debug!("Instantiating tags");
        let root_dir = path_absolute_parent(path)?;
        let tags = Tag::from_toml(ut.tags, &ut.default, &root_dir, path, options)?;

        log::debug!("Building the tag resolution table");
        let mut tag_resolution: BTreeMap<String, Vec<String>> =
            tags.keys().map(|k| (k.to_owned(), vec![k.to_owned()])).collect();
        for (k, lst) in ut.tag_groups.iter() {
            if tag_resolution.contains_key(k) {
                return Err(Error::test_config_msg("duplicate tag name").tag(k).path(path).into());
            }
            if lst.is_empty() {
                return Err(Error::test_config_msg("empty tag group").tag(k).path(path).into());
            }
            for tname in lst {
                if !tags.contains_key(tname) {
                    return Err(Error::test_config_msg(format!(
                        "unknown tag {tname} in tag group"
                    ))
                    .tag(k)
                    .path(path)
                    .into());
                }
            }
            tag_resolution.insert(k.to_owned(), lst.to_owned());
        }

        Ok(Tests { default: ut.default, tags, tag_resolution })
    }
}

#[cfg(test)]
#[expect(clippy::module_inception)]
mod tests {
    use super::*;
    use asserting::prelude::*;

    /// Path to the example tests.toml file (relative to project root)
    const EXAMPLE_TESTS_TOML: &str =
        concat!(env!("CARGO_MANIFEST_DIR"), "/example/tests/tests.toml");

    #[test]
    fn test_load_example_tests_toml() {
        let tests = Tests::load(EXAMPLE_TESTS_TOML, TestsLoadingOptions::default())
            .expect("Failed to load example tests.toml");

        // Verify default values are loaded correctly
        assert_that!(tests.default.build.timeout).is_equal_to(60);
        assert_that!(tests.default.test.timeout).is_equal_to(60);
        assert_that!(tests.default.tag.timeout_total).is_equal_to(1200);
        assert_that!(tests.default.test.max_output).is_equal_to(4194304);
        assert_that!(tests.default.test.kind.as_str()).is_equal_to("run");
        assert_that!(tests.default.build.prohibit_binary_files).is_true();
    }

    #[test]
    fn test_example_tags_exist() {
        let tests = Tests::load(EXAMPLE_TESTS_TOML, TestsLoadingOptions::default())
            .expect("Failed to load example tests.toml");

        // Verify all expected tags exist
        assert_that!(&tests.tags).contains_key("hello");
        assert_that!(&tests.tags).contains_key("hello-extra");
        assert_that!(&tests.tags).contains_key("hello-asm");
        assert_that!(&tests.tags).contains_key("hello-file");
        assert_that!(&tests.tags).does_not_contain_key("hello-all");

        // The group is resolvable, but is not a tag of its own
        assert_that!(&tests.tag_resolution).contains_key("hello-all");
        assert_eq!(tests.tag_resolution.get("hello"), Some(&vec!["hello".to_string()]));
    }

    #[test]
    fn test_example_tag_group_hello_all() {
        let tests = Tests::load(EXAMPLE_TESTS_TOML, TestsLoadingOptions::default())
            .expect("Failed to load example tests.toml");

        // Verify hello-all tag group contains all expected tags
        let hello_all =
            tests.tag_resolution.get("hello-all").expect("hello-all tag group not found");
        assert_that!(hello_all.len()).is_equal_to(4);

        assert_that!(hello_all).contains(&"hello".to_string());
        assert_that!(hello_all).contains(&"hello-extra".to_string());
        assert_that!(hello_all).contains(&"hello-asm".to_string());
        assert_that!(hello_all).contains(&"hello-file".to_string());
    }

    #[test]
    fn test_example_hello_tag_has_tests() {
        let tests = Tests::load(EXAMPLE_TESTS_TOML, TestsLoadingOptions::default())
            .expect("Failed to load example tests.toml");

        let hello_tag = tests.tags.get("hello").expect("hello tag not found");
        assert_that!(hello_tag.name.as_str()).is_equal_to("hello");
        assert_that!(&hello_tag.test_groups).is_not_empty();

        // The hello tag should have at least one test
        let total_tests: usize = hello_tag.test_groups.iter().map(|g| g.tests.len()).sum();
        assert_that!(total_tests).is_greater_than(0);
    }

    #[test]
    fn test_example_build_config() {
        let tests = Tests::load(EXAMPLE_TESTS_TOML, TestsLoadingOptions::default())
            .expect("Failed to load example tests.toml");

        let hello_tag = tests.tags.get("hello").expect("hello tag not found");

        // Verify build configuration
        assert_that!(hello_tag.build.srcdir.as_str()).is_equal_to("solutions/hello");
        assert_eq!(hello_tag.build.cmd, vec!["make"]);

        // Test the hello-extra tag
        let hello_extra = tests.tags.get("hello-extra").expect("hello-extra tag not found");
        let hetg = &hello_extra.test_groups[0];
        assert_eq!(hetg.title, "Hello (Extra tests)");
        assert_eq!(hetg.tests.len(), 0);
        assert_eq!(hetg.subgroups.len(), 4);
    }

    #[test]
    fn test_example_default_kind_run() {
        let tests = Tests::load(EXAMPLE_TESTS_TOML, TestsLoadingOptions::default())
            .expect("Failed to load example tests.toml");

        // Verify default kind.run configuration
        let run_config = &tests.default.test.kinds.run;
        assert_that!(run_config.bin.as_str()).is_equal_to("cigrid");
        assert_that!(&run_config.code).contains_exactly(&[0i32]);
        assert_that!(&run_config.stdin).is_none();
        assert_that!(run_config.stdout_trim).is_true();
        assert_that!(run_config.stderr_trim).is_true();
        assert_eq!(run_config.auto_input_files, vec![".cpp"]);
    }

    #[test]
    fn test_example_default_kind_gen_asm_and_run() {
        let tests = Tests::load(EXAMPLE_TESTS_TOML, TestsLoadingOptions::default())
            .expect("Failed to load example tests.toml");

        // Verify default kind.gen_asm_and_run configuration
        let asm_config = &tests.default.test.kinds.gen_asm_and_run;
        assert_that!(asm_config.bin.as_str()).is_equal_to("cigrid");
        assert_eq!(asm_config.args, vec!["--asm"]);
        assert_that!(&asm_config.assemble_code).contains_exactly(&[0i32]);
        assert_that!(&asm_config.compile_code).contains_exactly(&[0i32]);
        assert_that!(&asm_config.run_code).contains_exactly(&[0i32]);
    }

    #[test]
    fn test_example_allowed_binary_files() {
        let tests = Tests::load(EXAMPLE_TESTS_TOML, TestsLoadingOptions::default())
            .expect("Failed to load example tests.toml");

        // Verify allowed binary files
        assert_that!(&tests.default.build.allowed_binary_files)
            .contains(&"regalloc.pdf".to_string());
        assert_that!(&tests.default.build.allowed_binary_files)
            .contains(&"liveness.pdf".to_string());

        // Verify allowed binary mimetypes
        assert_that!(&tests.default.build.allowed_binary_mimetypes)
            .contains(&"application/pdf".to_string());
        assert_that!(&tests.default.build.allowed_binary_mimetypes)
            .contains(&"application/javascript".to_string());
        assert_that!(&tests.default.build.allowed_binary_mimetypes)
            .contains(&"application/json".to_string());
    }
}
