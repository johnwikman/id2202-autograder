-- Your SQL goes here
--
-- https://www.postgresql.org/docs/current/datatype.html

-- AFAIK, there is no built-in function that does something similar.
-- (https://www.postgresql.org/docs/current/functions-array.html)
CREATE FUNCTION array_is_distinct(text[]) RETURNS BOOL AS $body$
    SELECT count(*) = count(DISTINCT a)
    FROM unnest($1) AS a
$body$ LANGUAGE sql IMMUTABLE PARALLEL SAFE;

-- Origins (sources) for a submission
CREATE TABLE "submission_origins" (
    "id" BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,

    -- Submission kind (internal enumerator defined in the autograder)
    "kind" INT4 NOT NULL,

    -- ID in the table corresponding to the kind
    -- (Actually a FK, but here we don't know which table it corresponds to)
    "kind_id" BIGINT NOT NULL,

    -- Auth key for this specific origin, that this specific origin can use to
    -- fetch results from the autograder.
    "auth_key" TEXT NOT NULL,

    UNIQUE ("kind", "kind_id")
);

-- Table for individual submissions. This records the submission, whereas
-- individual graded jobs and their status lives in `submission_jobs`.
CREATE TABLE "submissions" (
    "id" BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    "submitted_at" TIMESTAMPTZ NOT NULL,

    -- The raw tag strings the submitter asked for, before resolution.
    "requested_tags" TEXT[] NOT NULL
        CHECK (num_nulls(VARIADIC requested_tags) = 0)
        CHECK (array_is_distinct(requested_tags)),

    -- Submission origin
    "origin_id" BIGINT REFERENCES submission_origins(id) NOT NULL,

    -- Optional submission report for exceptional circumstances, usually when
    -- no jobs could be created.
    "report" JSON
);

-- A job for a single grading tag. A submission with many tags will have
-- multiple of these jobs associated with them.
CREATE TABLE "submission_jobs" (
    "id" BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    "submission_id" BIGINT REFERENCES submissions(id) NOT NULL,

    -- The actual resolved tag being graded. This must not be an alias for
    -- another grading tag.
    "tag" TEXT NOT NULL,

    -- Which of the submission's requested_tags resolved to this tag.
    "requested_as" TEXT[] NOT NULL
        CHECK (num_nulls(VARIADIC requested_as) = 0)
        CHECK (array_is_distinct(requested_as)),

    -- If specified, the job must not be started before this time.
    "eligible_at" TIMESTAMPTZ,

    -- Set if the job was never attempted to be graded. If set, then the job
    -- must never attempt to be started again.
    "voided_at" TIMESTAMPTZ CHECK (voided_at IS NULL OR assigned_runner_id IS NULL),

    -- ID of the runner that has been assigned to this job.
    "assigned_runner_id" INT4,

    -- Execution status
    "status_code" INT4 NOT NULL,
    "status_text" TEXT,

    -- Time when this job started being graded.
    "started_at" TIMESTAMPTZ CHECK (started_at IS NULL OR assigned_runner_id IS NOT NULL),

    -- Time at which this job had finished grading. Checking whether this is
    -- NULL or not also acts as the "is_finished" check on the job.
    "finished_at" TIMESTAMPTZ CHECK (finished_at IS NULL OR started_at IS NOT NULL),

    -- Generated report for the job.
    --
    -- NOTE: Intentionally specified as opaque JSON blob (as opposed to JSONB)
    -- since this may be very large and should be compressed wherever possible.
    "report" JSON,

    -- One grading run per tag and submission.
    UNIQUE ("submission_id", "tag")
);

-- GitHub submission origin
CREATE TABLE "submission_origin_github" (
    "id" BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,

    "domain" TEXT NOT NULL, -- domain, e.g. github.foo.bar.com
    "org" TEXT NOT NULL,    -- the organization of the repository
    "repo" TEXT NOT NULL,   -- repository name, excluding the organization

    "ssh_url" TEXT NOT NULL, -- URL used to clone repo over SSH

    UNIQUE ("domain", "org", "repo")
);

-- Additional information about a specific submission from a GitHub origin
CREATE TABLE "submission_info_github" (
    "id" BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    "submission_id" BIGINT REFERENCES submissions(id) UNIQUE NOT NULL,
    "github_origin_id" BIGINT REFERENCES submission_origin_github(id) NOT NULL,

    "user" TEXT NOT NULL,
    "commit" TEXT NOT NULL
);


-- Pending jobs waiting to be run. This is any job that has not been voided
-- and has not been assigned to a runner for grading.
CREATE VIEW "v_pending_jobs" AS
    SELECT j.id, j.submission_id, s.origin_id, j.eligible_at
    FROM submission_jobs j
    JOIN submissions s ON j.submission_id = s.id
    WHERE j.voided_at IS NULL
      AND j.assigned_runner_id IS NULL;

-- Jobs that a runner may claim for grading, i.e. all pending jobs which are
-- eligible for grading.
CREATE VIEW "v_claimable_jobs" AS
    SELECT vpj.id, vpj.submission_id, vpj.origin_id
    FROM v_pending_jobs vpj
    WHERE (vpj.eligible_at IS NULL OR vpj.eligible_at <= now());

-- Jobs currently being graded by a runner.
CREATE VIEW "v_active_jobs" AS
    SELECT j.id, j.submission_id, s.origin_id
    FROM submission_jobs j
    JOIN submissions s ON j.submission_id = s.id
    WHERE j.finished_at IS NULL
      AND j.assigned_runner_id IS NOT NULL;
