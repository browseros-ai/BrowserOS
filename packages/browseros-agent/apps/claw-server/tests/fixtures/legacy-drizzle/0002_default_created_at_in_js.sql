PRAGMA foreign_keys=OFF;--> statement-breakpoint
CREATE TABLE `__new_agent_session_ends` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`created_at` integer NOT NULL,
	`session_id` text NOT NULL,
	`kind` text NOT NULL,
	`reason` text
);
--> statement-breakpoint
INSERT INTO `__new_agent_session_ends`("id", "created_at", "session_id", "kind", "reason") SELECT "id", "created_at", "session_id", "kind", "reason" FROM `agent_session_ends`;--> statement-breakpoint
DROP TABLE `agent_session_ends`;--> statement-breakpoint
ALTER TABLE `__new_agent_session_ends` RENAME TO `agent_session_ends`;--> statement-breakpoint
PRAGMA foreign_keys=ON;--> statement-breakpoint
CREATE INDEX `agent_session_ends_session_idx` ON `agent_session_ends` (`session_id`);--> statement-breakpoint
CREATE INDEX `agent_session_ends_created_at_idx` ON `agent_session_ends` (`created_at`);--> statement-breakpoint
CREATE TABLE `__new_agent_session_starts` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`created_at` integer NOT NULL,
	`session_id` text NOT NULL,
	`agent_id` text NOT NULL,
	`slug` text NOT NULL,
	`agent_label` text NOT NULL,
	`client_name` text NOT NULL,
	`client_version` text NOT NULL
);
--> statement-breakpoint
INSERT INTO `__new_agent_session_starts`("id", "created_at", "session_id", "agent_id", "slug", "agent_label", "client_name", "client_version") SELECT "id", "created_at", "session_id", "agent_id", "slug", "agent_label", "client_name", "client_version" FROM `agent_session_starts`;--> statement-breakpoint
DROP TABLE `agent_session_starts`;--> statement-breakpoint
ALTER TABLE `__new_agent_session_starts` RENAME TO `agent_session_starts`;--> statement-breakpoint
CREATE INDEX `agent_session_starts_session_idx` ON `agent_session_starts` (`session_id`);--> statement-breakpoint
CREATE INDEX `agent_session_starts_created_at_idx` ON `agent_session_starts` (`created_at`);--> statement-breakpoint
CREATE TABLE `__new_tool_dispatches` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`created_at` integer NOT NULL,
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
INSERT INTO `__new_tool_dispatches`("id", "created_at", "agent_id", "slug", "agent_label", "session_id", "tool_name", "page_id", "target_id", "url", "title", "args_json", "result_meta", "duration_ms") SELECT "id", "created_at", "agent_id", "slug", "agent_label", "session_id", "tool_name", "page_id", "target_id", "url", "title", "args_json", "result_meta", "duration_ms" FROM `tool_dispatches`;--> statement-breakpoint
DROP TABLE `tool_dispatches`;--> statement-breakpoint
ALTER TABLE `__new_tool_dispatches` RENAME TO `tool_dispatches`;--> statement-breakpoint
CREATE INDEX `tool_dispatches_created_at_idx` ON `tool_dispatches` (`created_at`);--> statement-breakpoint
CREATE INDEX `tool_dispatches_agent_created_idx` ON `tool_dispatches` (`agent_id`,`created_at`);--> statement-breakpoint
CREATE INDEX `tool_dispatches_session_idx` ON `tool_dispatches` (`session_id`);