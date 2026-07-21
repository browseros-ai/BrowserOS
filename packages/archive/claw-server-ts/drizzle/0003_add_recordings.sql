CREATE TABLE `tab_claims` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`target_id` text NOT NULL,
	`session_id` text NOT NULL,
	`agent_id` text NOT NULL,
	`claimed_at` integer NOT NULL,
	`released_at` integer
);
--> statement-breakpoint
CREATE INDEX `tab_claims_target_idx` ON `tab_claims` (`target_id`,`claimed_at`);--> statement-breakpoint
CREATE INDEX `tab_claims_session_idx` ON `tab_claims` (`session_id`);--> statement-breakpoint
CREATE TABLE `tab_recordings` (
	`target_id` text PRIMARY KEY NOT NULL,
	`tab_id` integer NOT NULL,
	`first_event_at` integer NOT NULL,
	`last_event_at` integer NOT NULL,
	`size_bytes` integer NOT NULL,
	`event_count` integer NOT NULL
);
--> statement-breakpoint
CREATE INDEX `tab_recordings_last_event_idx` ON `tab_recordings` (`last_event_at`);