CREATE TABLE `tool_dispatches` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`created_at` integer DEFAULT (unixepoch('subsec') * 1000) NOT NULL,
	`agent_id` text NOT NULL,
	`slug` text NOT NULL,
	`agent_label` text NOT NULL,
	`session_id` text NOT NULL,
	`tool_name` text NOT NULL,
	`page_id` integer,
	`target_id` text,
	`url` text,
	`title` text,
	`args_json` text,
	`result_meta` text,
	`duration_ms` integer
);
--> statement-breakpoint
CREATE INDEX `tool_dispatches_created_at_idx` ON `tool_dispatches` (`created_at`);--> statement-breakpoint
CREATE INDEX `tool_dispatches_agent_created_idx` ON `tool_dispatches` (`agent_id`,`created_at`);--> statement-breakpoint
CREATE INDEX `tool_dispatches_session_idx` ON `tool_dispatches` (`session_id`);