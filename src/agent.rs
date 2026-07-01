pub mod client;

pub(crate) mod clients;
pub(crate) mod db_agent;
pub(crate) mod explain_agent;
pub(crate) mod fs_agent;
pub(crate) mod skills;
pub mod types;

// Re-export so sibling submodules can do `use super::AgentRouter`
pub(crate) use client::AgentRouter;
