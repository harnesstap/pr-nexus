//! Plain service functions over the app state, shared by the Tauri
//! commands and future non-Tauri front ends (an MCP server).

pub mod bulk;
pub mod projects;
pub mod prs;
pub mod review;

#[cfg(test)]
mod tests;
