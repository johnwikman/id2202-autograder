-- Your SQL goes here

-- Direct submission origin
CREATE TABLE "submission_origin_direct" (
    "id" BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,

    "domain" TEXT NOT NULL, -- domain of the service that submitted this
    "entity" TEXT NOT NULL, -- any text string that defines the submitter within the origin domain

	UNIQUE ("domain", "entity")
);

-- Additional information about a specific submission from a direct origin
CREATE TABLE "submission_info_direct" (
    "id" BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    "submission_id" BIGINT REFERENCES submissions(id) UNIQUE NOT NULL,
    "direct_origin_id" BIGINT REFERENCES submission_origin_direct(id) NOT NULL,

    "local_path" TEXT NOT NULL, -- "path to the local file containing the submitted code"

    "sink_url" TEXT,
    "sink_secret_key" TEXT
        CHECK ((sink_secret_key IS NULL AND sink_url IS NULL)
           OR  (sink_secret_key IS NOT NULL AND sink_url IS NOT NULL))
);
