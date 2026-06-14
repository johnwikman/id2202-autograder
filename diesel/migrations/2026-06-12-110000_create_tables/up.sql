-- Your SQL goes here
--
-- https://www.postgresql.org/docs/current/datatype.html

-- Sources for a submission
CREATE TABLE "submission_sources" (
    "id" BIGSERIAL NOT NULL PRIMARY KEY,

    -- Submission kind (internal enumerator defined in the autograder)
	"kind" INT4 NOT NULL,

	-- ID in the table corresponding to the kind
	"kind_id" BIGINT NOT NULL,

	-- Auth key for this specific source, that this specific source can use to
	-- fetch results from the autograder.
	"auth_key" TEXT NOT NULL,

	UNIQUE ("kind", "kind_id")
);

-- Table for individual submissions
CREATE TABLE "submissions" (
	"id" BIGSERIAL NOT NULL PRIMARY KEY,
	"date_submitted" TIMESTAMP NOT NULL,
	"assigned_runner_id" INT4,

	-- Semi colon separated list of grading tags
	-- "tag1;tag2;..."
	"grading_tags" TEXT NOT NULL,

	-- Execution status
	"exec_finished" BOOLEAN NOT NULL,
	"exec_status_code" INT4 NOT NULL,
	"exec_status_text" TEXT,
	"exec_date_started" TIMESTAMP,
	"exec_date_finished" TIMESTAMP,
	"exec_report" JSON,

	-- Submission source
	"source_id" BIGINT REFERENCES submission_sources(id) NOT NULL
);

-- GitHub submission source
CREATE TABLE "submission_source_github" (
    "id" BIGSERIAL NOT NULL PRIMARY KEY,

    "domain" TEXT NOT NULL, -- domain, e.g. github.foo.bar.com
	"org" TEXT NOT NULL,    -- the organization of the repository
	"repo" TEXT NOT NULL,   -- repository name, excluding the organization

	"ssh_url" TEXT NOT NULL, -- URL used to clone repo over SSH

	UNIQUE ("domain", "org", "repo")
);

-- Additional information about a specific submission from a GitHub source
CREATE TABLE "submission_info_github" (
    "id" BIGSERIAL NOT NULL PRIMARY KEY,
    "submission_id" BIGINT REFERENCES submissions(id) UNIQUE NOT NULL,
    "github_source_id" BIGINT REFERENCES submission_source_github(id) NOT NULL,

    "user" TEXT NOT NULL,
	"commit" TEXT NOT NULL
);
