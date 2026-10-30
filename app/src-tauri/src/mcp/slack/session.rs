//! Who makes the current tool call: its MCP session and client. The
//! server sets it around each handler; Slack reads note the workspaces a
//! session has read, and sends check them (SPEC §10.1.2).

use std::cell::RefCell;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Caller {
    /// `Mcp-Session-Id`; `None` outside a session (tests, stateless calls).
    pub session: Option<String>,
    /// `clientInfo.name` from `initialize`.
    pub client: Option<String>,
}

thread_local! {
    static CURRENT: RefCell<Caller> = RefCell::default();
}

/// Run `f` as `caller`, on this thread.
pub fn with<T>(caller: Caller, f: impl FnOnce() -> T) -> T {
    let previous = CURRENT.with(|current| current.replace(caller));
    let out = f();
    CURRENT.with(|current| *current.borrow_mut() = previous);
    out
}

pub fn current() -> Caller {
    CURRENT.with(|current| current.borrow().clone())
}
