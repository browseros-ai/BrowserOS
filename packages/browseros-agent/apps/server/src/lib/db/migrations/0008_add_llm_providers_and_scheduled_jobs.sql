CREATE TABLE `llm_providers` (
	`id` text PRIMARY KEY NOT NULL,
	`profile_id` text,
	`type` text NOT NULL,
	`name` text NOT NULL,
	`base_url` text,
	`model_id` text NOT NULL,
	`supports_images` integer DEFAULT true NOT NULL,
	`context_window` integer NOT NULL,
	`temperature` real DEFAULT 0.2 NOT NULL,
	`api_key` text,
	`access_key_id` text,
	`secret_access_key` text,
	`session_token` text,
	`resource_name` text,
	`region` text,
	`reasoning_effort` text,
	`reasoning_summary` text,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL
);
--> statement-breakpoint
CREATE INDEX `llm_providers_profile_id_idx` ON `llm_providers` (`profile_id`);--> statement-breakpoint
CREATE TABLE `scheduled_jobs` (
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
	FOREIGN KEY (`provider_id`) REFERENCES `llm_providers`(`id`) ON UPDATE no action ON DELETE set null
);
--> statement-breakpoint
CREATE INDEX `scheduled_jobs_profile_id_idx` ON `scheduled_jobs` (`profile_id`);--> statement-breakpoint
CREATE INDEX `scheduled_jobs_enabled_idx` ON `scheduled_jobs` (`enabled`);