CREATE TABLE `scheduled_job_runs` (
	`id` text PRIMARY KEY NOT NULL,
	`profile_id` text,
	`job_id` text NOT NULL,
	`status` text NOT NULL,
	`started_at` integer NOT NULL,
	`completed_at` integer,
	`result` text,
	`final_result` text,
	`execution_log` text,
	`tool_calls` text,
	`error` text,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	FOREIGN KEY (`job_id`) REFERENCES `scheduled_jobs`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE INDEX `scheduled_job_runs_job_id_idx` ON `scheduled_job_runs` (`job_id`);--> statement-breakpoint
CREATE INDEX `scheduled_job_runs_started_at_idx` ON `scheduled_job_runs` (`started_at`);