-- Your SQL goes here
--
-- https://www.postgresql.org/docs/current/datatype.html

CREATE TABLE "submissions" (
	"id" BIGSERIAL NOT NULL PRIMARY KEY,
	"date_submitted" TIMESTAMP NOT NULL,
	"assigned_runner" INT4,

	-- Semi colon separated list of grading tags
	-- "tag1;tag2;..."
	"grading_tags" TEXT NOT NULL,

	-- Execution status
	"exec_finished" BOOLEAN NOT NULL,
	"exec_status_code" INT4 NOT NULL,
	"exec_status_text" TEXT,
	"exec_date_started" TIMESTAMP,
	"exec_date_finished" TIMESTAMP,

	-- GitHub specific data.
	"github_address" TEXT NOT NULL,
	"github_org" TEXT NOT NULL,  -- the organization of the repository
	"github_repo" TEXT NOT NULL, -- repository name, excluding the organization
	"github_user" TEXT NOT NULL,
	"github_commit" TEXT NOT NULL
);

-- keeping track of runner processes
CREATE TABLE "runners" (
	"id" INT4 NOT NULL PRIMARY KEY,
	"pid" INT8,
	"last_pinged" TIMESTAMP
);
