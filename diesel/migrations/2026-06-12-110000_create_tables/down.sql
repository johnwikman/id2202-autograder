-- This file should undo anything in `up.sql`
DROP VIEW IF EXISTS "v_active_jobs";
DROP VIEW IF EXISTS "v_claimable_jobs";
DROP VIEW IF EXISTS "v_pending_jobs";
DROP TABLE IF EXISTS "submission_info_github";
DROP TABLE IF EXISTS "submission_origin_github";
DROP TABLE IF EXISTS "submission_jobs";
DROP TABLE IF EXISTS "submissions";
DROP TABLE IF EXISTS "submission_origins";
DROP FUNCTION IF EXISTS array_is_distinct(text[]);
