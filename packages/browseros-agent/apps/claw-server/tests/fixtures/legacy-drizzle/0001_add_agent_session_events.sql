CREATE TABLE `agent_session_ends` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`created_at` integer DEFAULT (unixepoch('subsec') * 1000) NOT NULL,
	`session_id` text NOT NULL,
	`kind` text NOT NULL,
	`reason` text
);
--> statement-breakpoint
CREATE INDEX `agent_session_ends_session_idx` ON `agent_session_ends` (`session_id`);--> statement-breakpoint
CREATE INDEX `agent_session_ends_created_at_idx` ON `agent_session_ends` (`created_at`);--> statement-breakpoint
CREATE TABLE `agent_session_starts` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`created_at` integer DEFAULT (unixepoch('subsec') * 1000) NOT NULL,
	`session_id` text NOT NULL,
	`agent_id` text NOT NULL,
	`slug` text NOT NULL,
	`agent_label` text NOT NULL,
	`client_name` text NOT NULL,
	`client_version` text NOT NULL
);
--> statement-breakpoint
CREATE INDEX `agent_session_starts_session_idx` ON `agent_session_starts` (`session_id`);--> statement-breakpoint
CREATE INDEX `agent_session_starts_created_at_idx` ON `agent_session_starts` (`created_at`);