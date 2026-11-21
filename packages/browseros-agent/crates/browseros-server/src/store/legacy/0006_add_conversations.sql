CREATE TABLE `conversations` (
	`id` text PRIMARY KEY NOT NULL,
	`messages` text NOT NULL,
	`last_user_message` text,
	`origin` text,
	`target_type` text NOT NULL,
	`agent_id` text,
	`last_messaged_at` integer NOT NULL,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL
);
--> statement-breakpoint
CREATE INDEX `conversations_last_messaged_at_idx` ON `conversations` (`last_messaged_at`);