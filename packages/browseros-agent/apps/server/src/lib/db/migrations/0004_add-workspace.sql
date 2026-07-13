CREATE TABLE IF NOT EXISTS `workspace_collections` (
	`id` text PRIMARY KEY NOT NULL,
	`name` text NOT NULL,
	`description` text,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL
);
--> statement-breakpoint
CREATE TABLE IF NOT EXISTS `workspace_databases` (
	`id` text PRIMARY KEY NOT NULL,
	`collection_id` text,
	`name` text NOT NULL,
	`description` text,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	FOREIGN KEY (`collection_id`) REFERENCES `workspace_collections`(`id`) ON UPDATE no action ON DELETE set null
);
--> statement-breakpoint
CREATE TABLE IF NOT EXISTS `workspace_fields` (
	`id` text PRIMARY KEY NOT NULL,
	`database_id` text NOT NULL,
	`name` text NOT NULL,
	`key` text NOT NULL,
	`type` text NOT NULL,
	`position` integer DEFAULT 0 NOT NULL,
	`required` integer DEFAULT false NOT NULL,
	`configuration_json` text,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	FOREIGN KEY (`database_id`) REFERENCES `workspace_databases`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE TABLE IF NOT EXISTS `research_sessions` (
	`id` text PRIMARY KEY NOT NULL,
	`conversation_id` text,
	`goal` text NOT NULL,
	`status` text NOT NULL,
	`active_step_id` text,
	`browser_profile_id` text,
	`collection_id` text,
	`database_id` text,
	`recap_json` text,
	`started_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	`completed_at` integer,
	FOREIGN KEY (`collection_id`) REFERENCES `workspace_collections`(`id`) ON UPDATE no action ON DELETE set null,
	FOREIGN KEY (`database_id`) REFERENCES `workspace_databases`(`id`) ON UPDATE no action ON DELETE set null
);
--> statement-breakpoint
CREATE TABLE IF NOT EXISTS `research_plan_steps` (
	`id` text PRIMARY KEY NOT NULL,
	`session_id` text NOT NULL,
	`step_order` integer NOT NULL,
	`title` text NOT NULL,
	`description` text,
	`status` text NOT NULL,
	`tool_category` text NOT NULL,
	`requires_approval` integer DEFAULT false NOT NULL,
	`expected_output` text,
	`error` text,
	`started_at` integer,
	`completed_at` integer,
	FOREIGN KEY (`session_id`) REFERENCES `research_sessions`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE TABLE IF NOT EXISTS `research_events` (
	`id` text PRIMARY KEY NOT NULL,
	`session_id` text NOT NULL,
	`kind` text NOT NULL,
	`title` text NOT NULL,
	`detail` text,
	`payload_json` text,
	`created_at` integer NOT NULL,
	FOREIGN KEY (`session_id`) REFERENCES `research_sessions`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE TABLE IF NOT EXISTS `workspace_sources` (
	`id` text PRIMARY KEY NOT NULL,
	`session_id` text,
	`url` text NOT NULL,
	`title` text,
	`excerpt` text,
	`content_hash` text,
	`snapshot_path` text,
	`accessed_at` integer NOT NULL,
	`created_at` integer NOT NULL,
	FOREIGN KEY (`session_id`) REFERENCES `research_sessions`(`id`) ON UPDATE no action ON DELETE set null
);
--> statement-breakpoint
CREATE TABLE IF NOT EXISTS `workspace_records` (
	`id` text PRIMARY KEY NOT NULL,
	`database_id` text NOT NULL,
	`session_id` text,
	`source_id` text,
	`title` text,
	`data_json` text NOT NULL,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	FOREIGN KEY (`database_id`) REFERENCES `workspace_databases`(`id`) ON UPDATE no action ON DELETE cascade,
	FOREIGN KEY (`session_id`) REFERENCES `research_sessions`(`id`) ON UPDATE no action ON DELETE set null,
	FOREIGN KEY (`source_id`) REFERENCES `workspace_sources`(`id`) ON UPDATE no action ON DELETE set null
);
--> statement-breakpoint
CREATE TABLE IF NOT EXISTS `workspace_assets` (
	`id` text PRIMARY KEY NOT NULL,
	`session_id` text,
	`record_id` text,
	`source_id` text,
	`filename` text NOT NULL,
	`mime_type` text NOT NULL,
	`storage_key` text NOT NULL,
	`byte_size` integer NOT NULL,
	`content_hash` text NOT NULL,
	`width` integer,
	`height` integer,
	`created_at` integer NOT NULL,
	FOREIGN KEY (`session_id`) REFERENCES `research_sessions`(`id`) ON UPDATE no action ON DELETE set null,
	FOREIGN KEY (`record_id`) REFERENCES `workspace_records`(`id`) ON UPDATE no action ON DELETE set null,
	FOREIGN KEY (`source_id`) REFERENCES `workspace_sources`(`id`) ON UPDATE no action ON DELETE set null
);
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS `workspace_collections_updated_at_idx` ON `workspace_collections` (`updated_at`);
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS `workspace_databases_collection_idx` ON `workspace_databases` (`collection_id`);
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS `workspace_databases_updated_at_idx` ON `workspace_databases` (`updated_at`);
--> statement-breakpoint
CREATE UNIQUE INDEX IF NOT EXISTS `workspace_fields_database_key_unique` ON `workspace_fields` (`database_id`,`key`);
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS `workspace_fields_database_position_idx` ON `workspace_fields` (`database_id`,`position`);
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS `research_sessions_conversation_idx` ON `research_sessions` (`conversation_id`);
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS `research_sessions_status_updated_idx` ON `research_sessions` (`status`,`updated_at`);
--> statement-breakpoint
CREATE UNIQUE INDEX IF NOT EXISTS `research_plan_steps_session_order_unique` ON `research_plan_steps` (`session_id`,`step_order`);
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS `research_plan_steps_session_status_idx` ON `research_plan_steps` (`session_id`,`status`);
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS `research_events_session_created_idx` ON `research_events` (`session_id`,`created_at`);
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS `workspace_sources_session_accessed_idx` ON `workspace_sources` (`session_id`,`accessed_at`);
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS `workspace_sources_url_idx` ON `workspace_sources` (`url`);
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS `workspace_records_database_updated_idx` ON `workspace_records` (`database_id`,`updated_at`);
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS `workspace_records_session_idx` ON `workspace_records` (`session_id`);
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS `workspace_records_source_idx` ON `workspace_records` (`source_id`);
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS `workspace_assets_session_created_idx` ON `workspace_assets` (`session_id`,`created_at`);
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS `workspace_assets_record_idx` ON `workspace_assets` (`record_id`);
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS `workspace_assets_source_idx` ON `workspace_assets` (`source_id`);
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS `workspace_assets_hash_idx` ON `workspace_assets` (`content_hash`);
