--- Removes the BrowserOS-hosted provider. It is no longer offered, and while
--- the row survives it stays eligible as the stored default, so a request that
--- names no provider would still be answered on the hosted gateway.
---
--- Jobs pointing at it are unassigned first rather than left to the
--- ON DELETE SET NULL on scheduled_jobs.provider_id. The result is the same
--- where foreign keys are enforced, and unlike the cascade it does not depend
--- on the connection having switched them on.
---
--- Both predicates, not just the id: a second BrowserOS-typed provider could
--- have been created by hand while the type was offered in the provider form.
UPDATE `scheduled_jobs`
SET `provider_id` = NULL
WHERE `provider_id` IN (
  SELECT `id` FROM `providers` WHERE `id` = 'browseros' OR `type` = 'browseros'
);--> statement-breakpoint
DELETE FROM `providers` WHERE `id` = 'browseros' OR `type` = 'browseros';
