--- Drops profile_id from every table that reserved one. The column was held
--- for per-profile isolation and was never written: it is null on every row.
---
--- Nothing has to change about the single-default unique index, which would
--- have been the catch had this gone the other way. It is keyed on is_default
--- alone, deliberately, because SQLite treats NULLs as distinct and a
--- (profile_id, is_default) pair would have admitted one default per row while
--- the column stayed unset.
DROP INDEX `providers_profile_id_idx`;--> statement-breakpoint
ALTER TABLE `providers` DROP COLUMN `profile_id`;--> statement-breakpoint
DROP INDEX `scheduled_jobs_profile_id_idx`;--> statement-breakpoint
ALTER TABLE `scheduled_jobs` DROP COLUMN `profile_id`;--> statement-breakpoint
ALTER TABLE `scheduled_job_runs` DROP COLUMN `profile_id`;