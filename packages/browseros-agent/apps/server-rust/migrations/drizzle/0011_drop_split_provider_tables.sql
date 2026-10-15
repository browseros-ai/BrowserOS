--- Order matters here, and differs from what drizzle generated.
---
--- Dropping llm_providers while foreign keys are enforced fires the
--- ON DELETE SET NULL on scheduled_jobs.provider_id, so every job silently
--- loses the provider it was pointed at and falls back to the default on its
--- next run. The drops therefore happen inside the foreign_keys=OFF block and
--- after scheduled_jobs has been rebuilt against the new table.
PRAGMA foreign_keys=OFF;--> statement-breakpoint
CREATE TABLE `__new_scheduled_jobs` (
	`id` text PRIMARY KEY NOT NULL,
	`profile_id` text,
	`name` text NOT NULL,
	`query` text NOT NULL,
	`schedule_type` text NOT NULL,
	`schedule_time` text,
	`schedule_interval` integer,
	`enabled` integer DEFAULT true NOT NULL,
	`provider_id` text,
	`last_run_at` integer,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	FOREIGN KEY (`provider_id`) REFERENCES `providers`(`id`) ON UPDATE no action ON DELETE set null
);
--> statement-breakpoint
INSERT INTO `__new_scheduled_jobs`("id", "profile_id", "name", "query", "schedule_type", "schedule_time", "schedule_interval", "enabled", "provider_id", "last_run_at", "created_at", "updated_at") SELECT "id", "profile_id", "name", "query", "schedule_type", "schedule_time", "schedule_interval", "enabled", "provider_id", "last_run_at", "created_at", "updated_at" FROM `scheduled_jobs`;--> statement-breakpoint
DROP TABLE `scheduled_jobs`;--> statement-breakpoint
ALTER TABLE `__new_scheduled_jobs` RENAME TO `scheduled_jobs`;--> statement-breakpoint
DROP TABLE `acp_agents`;--> statement-breakpoint
DROP TABLE `llm_providers`;--> statement-breakpoint
PRAGMA foreign_keys=ON;--> statement-breakpoint
CREATE INDEX `scheduled_jobs_profile_id_idx` ON `scheduled_jobs` (`profile_id`);--> statement-breakpoint
CREATE INDEX `scheduled_jobs_enabled_idx` ON `scheduled_jobs` (`enabled`);
