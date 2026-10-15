//! sea-orm entities for the five tables the TS server's drizzle schema defines
//! (`apps/server/src/lib/db/schema/*.ts`, current after migration 0014). Column names and
//! nullability mirror that schema exactly; the DDL itself comes only from the frozen drizzle
//! migrations, never from these types. Used to build queries and writes; reads decode through
//! the stores' lenient `FromQueryResult` impls instead of these strict `Model`s.

pub(super) mod conversations;
pub(super) mod oauth_tokens;
pub(super) mod providers;
pub(super) mod scheduled_job_runs;
pub(super) mod scheduled_jobs;
