CREATE TABLE `recording_batches` (
	`document_id` text NOT NULL,
	`batch_id` text NOT NULL,
	`accepted_at` integer NOT NULL,
	PRIMARY KEY(`document_id`, `batch_id`),
	FOREIGN KEY (`document_id`) REFERENCES `recording_streams`(`document_id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE TABLE `recording_streams` (
	`document_id` text PRIMARY KEY NOT NULL,
	`tab_id` integer NOT NULL,
	`target_id` text,
	`first_event_at` integer NOT NULL,
	`last_event_at` integer NOT NULL,
	`size_bytes` integer NOT NULL,
	`event_count` integer NOT NULL,
	`has_gap` integer DEFAULT false NOT NULL
);
--> statement-breakpoint
CREATE INDEX `recording_streams_tab_time_idx` ON `recording_streams` (`tab_id`,`first_event_at`,`last_event_at`);--> statement-breakpoint
CREATE INDEX `recording_streams_retention_idx` ON `recording_streams` (`last_event_at`);--> statement-breakpoint
CREATE TABLE `session_tabs` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`session_id` text NOT NULL,
	`agent_id` text NOT NULL,
	`tab_id` integer NOT NULL,
	`opened_target_id` text,
	`claimed_at` integer NOT NULL,
	`released_at` integer
);
--> statement-breakpoint
CREATE INDEX `session_tabs_session_idx` ON `session_tabs` (`session_id`,`claimed_at`);--> statement-breakpoint
CREATE INDEX `session_tabs_tab_window_idx` ON `session_tabs` (`tab_id`,`claimed_at`,`released_at`);--> statement-breakpoint
CREATE UNIQUE INDEX `session_tabs_one_live_owner_idx` ON `session_tabs` (`tab_id`) WHERE "session_tabs"."released_at" is null;--> statement-breakpoint
ALTER TABLE `tool_dispatches` ADD `tab_id` integer;