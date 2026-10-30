//! Who makes the current tool call: its MCP session and client. The
//! server sets it around each handler; Slack reads note the workspaces a
//! session has read, and sends check them (SPEC §10.1.2).

use std::cell::RefCell;
use std::fmt;
use std::sync::Arc;

/// What the user answered to a confirmation shown in their agent
/// (MCP elicitation, SPEC §10.1.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Accept,
    Decline,
    /// Dismissed without choosing.
    Cancel,
    /// No answer: timed out, or the request failed. Says why.
    Unanswered(String),
}

/// Asks the user to confirm in the calling agent. Blocks until they answer
/// or it gives up; handlers run on a blocking thread.
pub trait Ask: Send + Sync {
    fn ask(&self, message: &str) -> Answer;
}

#[derive(Clone, Default)]
pub struct Caller {
    /// `Mcp-Session-Id`; `None` outside a session (tests, stateless calls).
    pub session: Option<String>,
    /// `clientInfo.name` from `initialize`.
    pub client: Option<String>,
    /// Set when the client declared form elicitation in `initialize`.
    pub ask: Option<Arc<dyn Ask>>,
}

impl fmt::Debug for Caller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Caller")
            .field("session", &self.session)
            .field("client", &self.client)
            .field("ask", &self.ask.is_some())
            .finish()
    }
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
