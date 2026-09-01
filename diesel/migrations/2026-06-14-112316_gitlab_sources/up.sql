-- Your SQL goes here

-- GitLab submission origin
CREATE TABLE "submission_origin_gitlab" (
    "id" BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,

    "domain" TEXT NOT NULL,    -- domain, e.g. GitLab.foo.bar.com
	"namespace" TEXT NOT NULL, -- the namespace the repository is within
	"repo" TEXT NOT NULL,      -- repository name, excluding the namespace

	"ssh_url" TEXT NOT NULL, -- URL used to clone repo over SSH

	UNIQUE ("domain", "namespace", "repo")
);

-- Additional information about a specific submission from a GitLab origin
CREATE TABLE "submission_info_gitlab" (
    "id" BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    "submission_id" BIGINT REFERENCES submissions(id) UNIQUE NOT NULL,
    "gitlab_origin_id" BIGINT REFERENCES submission_origin_gitlab(id) NOT NULL,

    "user" TEXT NOT NULL,
	"commit" TEXT NOT NULL
);
