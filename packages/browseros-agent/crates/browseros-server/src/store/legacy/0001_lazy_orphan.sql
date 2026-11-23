CREATE TABLE `oauth_tokens` (
	`browseros_id` text NOT NULL,
	`provider` text NOT NULL,
	`access_token` text NOT NULL,
	`refresh_token` text NOT NULL,
	`expires_at` integer NOT NULL,
	`email` text,
	`account_id` text,
	`updated_at` integer NOT NULL,
	PRIMARY KEY(`browseros_id`, `provider`)
);
--> statement-breakpoint
CREATE INDEX `oauth_tokens_browseros_id_idx` ON `oauth_tokens` (`browseros_id`);