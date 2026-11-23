CREATE TABLE `providers` (
	`id` text PRIMARY KEY NOT NULL,
	`profile_id` text,
	`kind` text NOT NULL,
	`type` text NOT NULL,
	`name` text NOT NULL,
	`model_id` text,
	`reasoning_effort` text,
	`is_default` integer DEFAULT false NOT NULL,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	`base_url` text,
	`supports_images` integer DEFAULT true NOT NULL,
	`context_window` integer,
	`temperature` real DEFAULT 0.2 NOT NULL,
	`api_key` text,
	`access_key_id` text,
	`secret_access_key` text,
	`session_token` text,
	`resource_name` text,
	`region` text,
	`reasoning_summary` text,
	`working_directory` text,
	`custom_config` text,
	CONSTRAINT "providers_llm_requires_model_and_context" CHECK("providers"."kind" <> 'llm' OR ("providers"."model_id" IS NOT NULL AND "providers"."context_window" IS NOT NULL))
);
--> statement-breakpoint
CREATE INDEX `providers_profile_id_idx` ON `providers` (`profile_id`);--> statement-breakpoint
CREATE INDEX `providers_kind_updated_at_idx` ON `providers` (`kind`,`updated_at`);--> statement-breakpoint
CREATE UNIQUE INDEX `providers_one_default` ON `providers` (`is_default`) WHERE "providers"."is_default" = 1;--> statement-breakpoint
--- Copy both source tables in. Written by hand because drizzle generates
--- schema changes, not data moves. A colliding id across the two sources
--- fails the insert rather than silently dropping a row; ids are client
--- generated uuids plus the fixed 'browseros' literal, so a collision would
--- mean something is already wrong.
INSERT INTO `providers` (
  `id`, `profile_id`, `kind`, `type`, `name`, `model_id`, `reasoning_effort`,
  `is_default`, `created_at`, `updated_at`,
  `base_url`, `supports_images`, `context_window`, `temperature`,
  `api_key`, `access_key_id`, `secret_access_key`, `session_token`,
  `resource_name`, `region`, `reasoning_summary`
)
SELECT
  `id`, `profile_id`, 'llm', `type`, `name`, `model_id`, `reasoning_effort`,
  0, `created_at`, `updated_at`,
  `base_url`, `supports_images`, `context_window`, `temperature`,
  `api_key`, `access_key_id`, `secret_access_key`, `session_token`,
  `resource_name`, `region`, `reasoning_summary`
FROM `llm_providers`;
--> statement-breakpoint
--- ACP agents carry no profile, model context or credentials, so those stay
--- null or take the column default. The check constraint only requires a model
--- and a context window for kind = 'llm'.
INSERT INTO `providers` (
  `id`, `kind`, `type`, `name`, `model_id`, `reasoning_effort`,
  `is_default`, `created_at`, `updated_at`,
  `working_directory`, `custom_config`
)
SELECT
  `id`, 'acp', `type`, `name`, `model_id`, `reasoning_effort`,
  0, `created_at`, `updated_at`,
  `working_directory`, `custom_config`
FROM `acp_agents`;
