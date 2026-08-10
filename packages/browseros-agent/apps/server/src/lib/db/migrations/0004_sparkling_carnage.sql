CREATE TABLE `acp_agents` (
	`id` text PRIMARY KEY NOT NULL,
	`name` text NOT NULL,
	`type` text NOT NULL,
	`model_id` text,
	`reasoning_effort` text,
	`working_directory` text,
	`pinned` integer DEFAULT false NOT NULL,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL
);
--> statement-breakpoint
CREATE INDEX `acp_agents_updated_at_idx` ON `acp_agents` (`updated_at`);--> statement-breakpoint
CREATE INDEX `acp_agents_type_updated_at_idx` ON `acp_agents` (`type`,`updated_at`);--> statement-breakpoint
DELETE FROM `agent_definitions`;--> statement-breakpoint
DROP TABLE `produced_files`;
