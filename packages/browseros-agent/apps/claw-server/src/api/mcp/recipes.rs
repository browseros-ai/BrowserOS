//! MCP's recipe view comes from the skill-owned registry below the API boundary.
//! The installed reference and these instruction lines therefore share one instance.

#[cfg(test)]
pub(crate) use crate::services::harness_skills::recipes::all;
pub(crate) use crate::services::harness_skills::recipes::instruction_lines;
