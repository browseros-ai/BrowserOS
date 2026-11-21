CREATE TABLE `agent_definitions` (
	`id` text PRIMARY KEY NOT NULL,
	`name` text NOT NULL,
	`adapter` text NOT NULL,
	`model_id` text NOT NULL,
	`reasoning_effort` text NOT NULL,
	`permission_mode` text DEFAULT 'approve-all' NOT NULL,
	`session_key` text NOT NULL,
	`pinned` integer DEFAULT false NOT NULL,
	`adapter_config_json` text,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL
);
--> statement-breakpoint
CREATE UNIQUE INDEX `agent_definitions_session_key_unique` ON `agent_definitions` (`session_key`);--> statement-breakpoint
CREATE INDEX `agent_definitions_updated_at_idx` ON `agent_definitions` (`updated_at`);--> statement-breakpoint
CREATE INDEX `agent_definitions_adapter_updated_at_idx` ON `agent_definitions` (`adapter`,`updated_at`);