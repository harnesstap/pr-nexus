//! One keychain entry, `dev.pr-nexus` / `vault`, holding a JSON map.
//!
//! Every keychain call runs on one worker thread, one at a time, so a caller
//! never blocks the UI on it and the system never shows two prompts at once.
//! The first read can sit behind a macOS authorization prompt for as long as
//! the user takes to answer it. That is reported as *waiting*, not as
//! *unavailable*: secrets saved meanwhile stay in memory and are written to
//! the keychain as soon as it answers, even after the wait timed out.
//! Only a real failure (no keychain, access denied, a platform error) puts
//! the vault in its in-memory fallback, and [`Vault::retry`] asks again.
//! The vault never writes before a successful read, so a failed read cannot
//! overwrite the stored entry.

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use pr_nexus_core::error::CoreError;

const SERVICE: &str = "dev.pr-nexus";
const ACCOUNT: &str = "vault";

/// How long a keychain call may wait on the user before the vault reports
/// that it timed out. The call itself is never abandoned: a late answer is
/// still used.
pub const PROMPT_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Where the vault entry lives. [`SystemKeyring`] in the app; a fake in tests.
pub trait KeyringBackend: Send + Sync + 'static {
    /// The stored JSON, or `None` when there is no entry yet.
    fn read(&self) -> Result<Option<String>, KeyringError>;
    fn write(&self, json: &str) -> Result<(), KeyringError>;
}

/// Why a keychain call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyringError {
    /// No credential store on this system (no Secret Service, no keychain).
    NoBackend(String),
    /// The user refused access (Deny on the macOS prompt, a dismissed unlock).
    Denied(String),
    /// Anything else the platform reported.
    Other(String),
}

/// What the UI shows about the keychain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyringState {
    /// Tokens are read from and saved to the keychain.
    Ready,
    /// A keychain call is in flight, usually behind a system prompt.
    Waiting,
    /// Still waiting after [`PROMPT_TIMEOUT`]. A late answer is still used.
    TimedOut,
    Denied,
    NoBackend,
    Error,
    /// `PR_NEXUS_DISABLE_KEYRING`, or an in-memory test vault.
    Disabled,
}

impl KeyringState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Waiting => "waiting",
            Self::TimedOut => "timed_out",
            Self::Denied => "denied",
            Self::NoBackend => "no_backend",
            Self::Error => "error",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyringStatus {
    pub state: KeyringState,
    /// The banner text; `None` when the keychain is ready.
    pub message: Option<String>,
    /// [`Vault::retry`] can help.
    pub can_retry: bool,
    /// Some secrets are only in memory and not in the keychain yet.
    pub unsaved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Read,
    Write,
}

#[derive(Debug, Clone)]
enum Phase {
    Waiting { since: Instant },
    Ready,
    Failed { error: KeyringError, op: Op },
    Disabled { message: String },
}

struct Inner {
    phase: Phase,
    /// The keychain entry has been read, so `secrets` holds it.
    loaded: bool,
    /// What callers see: the stored map plus this session's changes.
    secrets: BTreeMap<String, String>,
    /// Changes made before the first read, replayed on top of it.
    early: BTreeMap<String, Option<String>>,
    /// Bumped on every change.
    revision: u64,
    /// The revision the keychain holds.
    saved: u64,
}

impl Inner {
    fn unsaved(&self) -> bool {
        self.revision != self.saved
    }

    fn change(&mut self, key: &str, value: Option<&str>) -> u64 {
        match value {
            Some(value) => {
                self.secrets.insert(key.to_owned(), value.to_owned());
            }
            None => {
                self.secrets.remove(key);
            }
        }
        if !self.loaded {
            self.early
                .insert(key.to_owned(), value.map(ToOwned::to_owned));
        }
        self.revision += 1;
        self.revision
    }
}

struct Shared {
    inner: Mutex<Inner>,
    changed: Condvar,
    backend: Option<Box<dyn KeyringBackend>>,
    timeout: Duration,
}

impl Shared {
    fn lock(&self) -> Result<MutexGuard<'_, Inner>, CoreError> {
        self.inner
            .lock()
            .map_err(|_| CoreError::Store("vault lock poisoned".into()))
    }

    /// One worker pass: read the entry if it was never read, then write any
    /// change the keychain does not hold yet.
    fn sync(&self) {
        let Some(backend) = &self.backend else {
            return;
        };
        let Ok(loaded) = self.lock().map(|inner| inner.loaded) else {
            return;
        };
        if !loaded {
            let result = backend.read().and_then(|json| decode(json.as_deref()));
            let Ok(mut inner) = self.lock() else {
                return;
            };
            match result {
                Ok(mut stored) => {
                    for (key, value) in std::mem::take(&mut inner.early) {
                        match value {
                            Some(value) => stored.insert(key, value),
                            None => stored.remove(&key),
                        };
                    }
                    inner.secrets = stored;
                    inner.loaded = true;
                    inner.phase = Phase::Ready;
                }
                Err(error) => {
                    inner.phase = Phase::Failed {
                        error,
                        op: Op::Read,
                    };
                }
            }
            let loaded = inner.loaded;
            drop(inner);
            self.changed.notify_all();
            if !loaded {
                return;
            }
        }
        loop {
            let (json, revision) = {
                let Ok(inner) = self.lock() else {
                    return;
                };
                if !inner.unsaved() {
                    return;
                }
                match serde_json::to_string(&inner.secrets) {
                    Ok(json) => (json, inner.revision),
                    Err(err) => {
                        drop(inner);
                        self.fail(KeyringError::Other(format!("encode: {err}")), Op::Write);
                        return;
                    }
                }
            };
            match backend.write(&json) {
                Ok(()) => {
                    if let Ok(mut inner) = self.lock() {
                        inner.saved = inner.saved.max(revision);
                        inner.phase = Phase::Ready;
                    }
                    self.changed.notify_all();
                }
                Err(error) => {
                    self.fail(error, Op::Write);
                    return;
                }
            }
        }
    }

    fn fail(&self, error: KeyringError, op: Op) {
        if let Ok(mut inner) = self.lock() {
            inner.phase = Phase::Failed { error, op };
        }
        self.changed.notify_all();
    }

    /// Blocks while the first read is pending, up to the timeout.
    fn wait_for_read(&self) -> Result<MutexGuard<'_, Inner>, CoreError> {
        let mut inner = self.lock()?;
        let deadline = Instant::now() + self.timeout;
        while !inner.loaded && matches!(inner.phase, Phase::Waiting { .. }) {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            inner = self
                .changed
                .wait_timeout(inner, deadline - now)
                .map_err(|_| CoreError::Store("vault lock poisoned".into()))?
                .0;
        }
        Ok(inner)
    }

    /// Blocks until `revision` is in the keychain, the write fails, or the
    /// timeout passes. Returns at once when the keychain is not ready.
    fn wait_for_write(&self, revision: u64) -> Result<(), CoreError> {
        let mut inner = self.lock()?;
        let deadline = Instant::now() + self.timeout;
        while inner.saved < revision && matches!(inner.phase, Phase::Ready) {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            inner = self
                .changed
                .wait_timeout(inner, deadline - now)
                .map_err(|_| CoreError::Store("vault lock poisoned".into()))?
                .0;
        }
        Ok(())
    }
}

pub struct Vault {
    shared: Arc<Shared>,
    jobs: Option<mpsc::Sender<()>>,
}

impl Vault {
    /// The system keychain, unless `PR_NEXUS_DISABLE_KEYRING` is set.
    /// Returns at once: the first read runs on the keyring worker.
    pub fn open() -> Self {
        if std::env::var("PR_NEXUS_DISABLE_KEYRING")
            .ok()
            .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        {
            return Self::memory(
                "Keyring disabled via PR_NEXUS_DISABLE_KEYRING. Tokens stay in memory for this session.",
            );
        }
        Self::with_backend(Box::new(SystemKeyring), PROMPT_TIMEOUT)
    }

    /// A vault over `backend`. `timeout` is how long a pending keychain call
    /// counts as *waiting* before it is reported as timed out.
    pub fn with_backend(backend: Box<dyn KeyringBackend>, timeout: Duration) -> Self {
        let shared = Arc::new(Shared {
            inner: Mutex::new(Inner {
                phase: Phase::Waiting {
                    since: Instant::now(),
                },
                loaded: false,
                secrets: BTreeMap::new(),
                early: BTreeMap::new(),
                revision: 0,
                saved: 0,
            }),
            changed: Condvar::new(),
            backend: Some(backend),
            timeout,
        });
        let (tx, rx) = mpsc::channel::<()>();
        let worker = Arc::clone(&shared);
        let spawned = thread::Builder::new()
            .name("pr-nexus-keyring".into())
            .spawn(move || {
                while rx.recv().is_ok() {
                    // Coalesce queued requests: one pass covers them all.
                    while rx.try_recv().is_ok() {}
                    worker.sync();
                }
            });
        let vault = match spawned {
            Ok(_) => Self {
                shared,
                jobs: Some(tx),
            },
            Err(err) => {
                return Self::memory(&format!(
                    "Keyring unavailable (could not start its worker: {err}). Tokens stay in memory for this session and are lost when the app quits."
                ));
            }
        };
        vault.kick();
        vault
    }

    /// An in-memory vault that never touches a keychain.
    pub fn memory(warning: &str) -> Self {
        Self {
            shared: Arc::new(Shared {
                inner: Mutex::new(Inner {
                    phase: Phase::Disabled {
                        message: warning.to_owned(),
                    },
                    loaded: true,
                    secrets: BTreeMap::new(),
                    early: BTreeMap::new(),
                    revision: 0,
                    saved: 0,
                }),
                changed: Condvar::new(),
                backend: None,
                timeout: Duration::ZERO,
            }),
            jobs: None,
        }
    }

    fn kick(&self) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(());
        }
    }

    /// The keychain state. Never blocks on the keychain.
    pub fn status(&self) -> KeyringStatus {
        let Ok(inner) = self.shared.lock() else {
            return KeyringStatus {
                state: KeyringState::Error,
                message: Some("Keyring state is unreadable (lock poisoned).".into()),
                can_retry: false,
                unsaved: true,
            };
        };
        let unsaved = inner.unsaved();
        let (state, message) = match &inner.phase {
            Phase::Ready => (KeyringState::Ready, None),
            Phase::Disabled { message } => (KeyringState::Disabled, Some(message.clone())),
            Phase::Waiting { since } if since.elapsed() >= self.shared.timeout => (
                KeyringState::TimedOut,
                Some(format!(
                    "Still waiting for keychain access after {}. Tokens stay in memory for now and are saved to the keychain as soon as it answers. Approve the system prompt, or retry.",
                    describe(self.shared.timeout)
                )),
            ),
            Phase::Waiting { .. } => (
                KeyringState::Waiting,
                Some(
                    "Waiting for keychain access. If a system prompt asks for PR Nexus's keychain item, approve it (on macOS, click Always Allow)."
                        .into(),
                ),
            ),
            Phase::Failed { error, op } => failure(error, *op),
        };
        KeyringStatus {
            can_retry: matches!(
                state,
                KeyringState::TimedOut
                    | KeyringState::Denied
                    | KeyringState::NoBackend
                    | KeyringState::Error
            ),
            state,
            message,
            unsaved,
        }
    }

    /// Ask the keychain again: read it if it was never read, then save
    /// what is only in memory. Does not wait for the answer.
    pub fn retry(&self) -> KeyringStatus {
        if self.jobs.is_some()
            && let Ok(mut inner) = self.shared.lock()
            && !matches!(inner.phase, Phase::Ready | Phase::Disabled { .. })
        {
            inner.phase = Phase::Waiting {
                since: Instant::now(),
            };
        }
        self.kick();
        self.status()
    }

    pub fn warning(&self) -> Option<String> {
        self.status().message
    }

    pub fn available(&self) -> bool {
        self.status().state == KeyringState::Ready
    }

    /// The secret under `key`. While the first keychain read is pending
    /// (a prompt on screen), waits for it, up to the prompt timeout. Call it
    /// off the UI thread.
    pub fn get(&self, key: &str) -> Result<Option<String>, CoreError> {
        Ok(self.shared.wait_for_read()?.secrets.get(key).cloned())
    }

    /// The secret under `key` as known now, without waiting for the keychain.
    /// Use it for display: before the first read it only sees this session's
    /// changes.
    pub fn peek(&self, key: &str) -> Result<Option<String>, CoreError> {
        Ok(self.shared.lock()?.secrets.get(key).cloned())
    }

    /// Saves `value` in memory at once, then in the keychain. When the
    /// keychain is ready this waits for the write; otherwise the value is
    /// written once the keychain answers. A failed write shows in
    /// [`Vault::status`], not as an error.
    pub fn set(&self, key: &str, value: &str) -> Result<(), CoreError> {
        self.change(key, Some(value))
    }

    pub fn remove(&self, key: &str) -> Result<(), CoreError> {
        self.change(key, None)
    }

    fn change(&self, key: &str, value: Option<&str>) -> Result<(), CoreError> {
        let revision = self.shared.lock()?.change(key, value);
        if self.jobs.is_none() {
            return Ok(());
        }
        self.kick();
        self.shared.wait_for_write(revision)
    }
}

fn failure(error: &KeyringError, op: Op) -> (KeyringState, Option<String>) {
    const LOST: &str = "Tokens stay in memory for this session and are lost when the app quits.";
    let (state, text) = match (error, op) {
        (KeyringError::Denied(_), _) => (
            KeyringState::Denied,
            format!("Keychain access was denied. {LOST} Retry to ask again."),
        ),
        (KeyringError::NoBackend(reason), _) => (
            KeyringState::NoBackend,
            format!("No keychain is available on this system ({reason}). {LOST}"),
        ),
        (KeyringError::Other(reason), Op::Read) => (
            KeyringState::Error,
            format!("Could not read the keychain ({reason}). {LOST}"),
        ),
        (KeyringError::Other(reason), Op::Write) => (
            KeyringState::Error,
            format!("Could not save to the keychain ({reason}). {LOST}"),
        ),
    };
    (state, Some(text))
}

fn describe(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds >= 60 && seconds.is_multiple_of(60) {
        let minutes = seconds / 60;
        format!("{minutes} minute{}", if minutes == 1 { "" } else { "s" })
    } else if seconds >= 1 {
        format!("{seconds}s")
    } else {
        format!("{}ms", duration.as_millis())
    }
}

fn decode(json: Option<&str>) -> Result<BTreeMap<String, String>, KeyringError> {
    match json {
        None => Ok(BTreeMap::new()),
        Some(json) => serde_json::from_str(json)
            .map_err(|err| KeyringError::Other(format!("the stored entry is not valid: {err}"))),
    }
}

/// The platform keychain through the `keyring` crate.
pub struct SystemKeyring;

impl KeyringBackend for SystemKeyring {
    fn read(&self) -> Result<Option<String>, KeyringError> {
        let entry = keyring::Entry::new(SERVICE, ACCOUNT).map_err(classify)?;
        match entry.get_password() {
            Ok(json) => Ok(Some(json)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(classify(err)),
        }
    }

    fn write(&self, json: &str) -> Result<(), KeyringError> {
        let entry = keyring::Entry::new(SERVICE, ACCOUNT).map_err(classify)?;
        entry.set_password(json).map_err(classify)
    }
}

fn classify(err: keyring::Error) -> KeyringError {
    let reason = err.to_string();
    match &err {
        keyring::Error::NoDefaultStore | keyring::Error::NotSupportedByStore(_) => {
            KeyringError::NoBackend(reason)
        }
        // Secret Service reports a dismissed unlock prompt as no access.
        keyring::Error::NoStorageAccess(_) if is_denial(&format!("{err:?}")) => {
            KeyringError::Denied(reason)
        }
        keyring::Error::NoStorageAccess(_) => KeyringError::NoBackend(reason),
        keyring::Error::PlatformFailure(_) if is_denial(&format!("{err:?}")) => {
            KeyringError::Denied(reason)
        }
        _ => KeyringError::Other(reason),
    }
}

/// macOS answers Deny with errSecAuthFailed (-25293) and Cancel with
/// errSecUserCanceled (-128); Secret Service says the prompt was dismissed.
fn is_denial(detail: &str) -> bool {
    let detail = detail.to_ascii_lowercase();
    ["-128", "-25293", "cancel", "denied", "dismiss", "prompt"]
        .iter()
        .any(|needle| detail.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scripted keychain. Each call takes the next answer from its queue
    /// and blocks until the test releases it, like a prompt on screen.
    struct FakeKeyring {
        stored: Arc<Mutex<Option<String>>>,
        reads: Mutex<mpsc::Receiver<Result<(), KeyringError>>>,
        writes: Mutex<mpsc::Receiver<Result<(), KeyringError>>>,
        calls: Arc<Mutex<Vec<&'static str>>>,
        active: Arc<Mutex<usize>>,
        max_active: Arc<Mutex<usize>>,
    }

    struct Script {
        stored: Arc<Mutex<Option<String>>>,
        reads: mpsc::Sender<Result<(), KeyringError>>,
        writes: mpsc::Sender<Result<(), KeyringError>>,
        calls: Arc<Mutex<Vec<&'static str>>>,
        max_active: Arc<Mutex<usize>>,
    }

    fn fake(stored: Option<&str>) -> (FakeKeyring, Script) {
        let stored = Arc::new(Mutex::new(stored.map(ToOwned::to_owned)));
        let (read_tx, read_rx) = mpsc::channel();
        let (write_tx, write_rx) = mpsc::channel();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let max_active = Arc::new(Mutex::new(0));
        (
            FakeKeyring {
                stored: Arc::clone(&stored),
                reads: Mutex::new(read_rx),
                writes: Mutex::new(write_rx),
                calls: Arc::clone(&calls),
                active: Arc::new(Mutex::new(0)),
                max_active: Arc::clone(&max_active),
            },
            Script {
                stored,
                reads: read_tx,
                writes: write_tx,
                calls,
                max_active,
            },
        )
    }

    impl FakeKeyring {
        fn enter(&self, name: &'static str) {
            self.calls.lock().unwrap().push(name);
            let mut active = self.active.lock().unwrap();
            *active += 1;
            let mut max = self.max_active.lock().unwrap();
            *max = (*max).max(*active);
        }

        fn leave(&self) {
            *self.active.lock().unwrap() -= 1;
        }
    }

    impl KeyringBackend for FakeKeyring {
        fn read(&self) -> Result<Option<String>, KeyringError> {
            self.enter("read");
            let answer = self.reads.lock().unwrap().recv().expect("scripted read");
            self.leave();
            answer.map(|()| self.stored.lock().unwrap().clone())
        }

        fn write(&self, json: &str) -> Result<(), KeyringError> {
            self.enter("write");
            let answer = self.writes.lock().unwrap().recv().expect("scripted write");
            self.leave();
            answer.map(|()| *self.stored.lock().unwrap() = Some(json.to_owned()))
        }
    }

    fn wait_until(vault: &Vault, state: KeyringState) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while vault.status().state != state {
            assert!(
                Instant::now() < deadline,
                "still {:?}, wanted {state:?}",
                vault.status()
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_saved(vault: &Vault) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while vault.status().unsaved {
            assert!(Instant::now() < deadline, "never saved");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn stored(script: &Script) -> BTreeMap<String, String> {
        serde_json::from_str(script.stored.lock().unwrap().as_deref().unwrap()).unwrap()
    }

    #[test]
    fn memory_vault_round_trips_without_a_keyring() {
        let vault = Vault::memory("test");
        assert!(!vault.available());
        assert_eq!(vault.status().state, KeyringState::Disabled);
        assert!(!vault.status().can_retry);
        assert!(vault.warning().unwrap().contains("test"));
        vault.set("github:U_1", "secret").unwrap();
        assert_eq!(vault.get("github:U_1").unwrap().as_deref(), Some("secret"));
        vault.remove("github:U_1").unwrap();
        assert_eq!(vault.get("github:U_1").unwrap(), None);
    }

    #[test]
    fn opening_never_blocks_on_a_pending_prompt() {
        let (backend, script) = fake(Some(r#"{"github:U_1":"saved"}"#));
        let started = Instant::now();
        let vault = Vault::with_backend(Box::new(backend), PROMPT_TIMEOUT);
        assert!(started.elapsed() < Duration::from_secs(1));
        let status = vault.status();
        assert_eq!(status.state, KeyringState::Waiting);
        assert!(
            status
                .message
                .unwrap()
                .contains("Waiting for keychain access")
        );
        assert!(!status.can_retry);
        assert_eq!(vault.peek("github:U_1").unwrap(), None);
        script.reads.send(Ok(())).unwrap();
        assert_eq!(vault.get("github:U_1").unwrap().as_deref(), Some("saved"));
        assert!(vault.available());
        assert_eq!(vault.warning(), None);
    }

    #[test]
    fn a_slow_prompt_times_out_then_a_late_allow_saves_the_session_tokens() {
        let (backend, script) = fake(Some(r#"{"github:U_1":"old","slack:T0:U0":"kept"}"#));
        let vault = Vault::with_backend(Box::new(backend), Duration::from_millis(50));
        wait_until(&vault, KeyringState::TimedOut);
        let status = vault.status();
        assert!(status.can_retry);
        assert!(
            status
                .message
                .unwrap()
                .contains("saved to the keychain as soon as it answers")
        );

        // Changes while the prompt is still open stay in memory, nothing is written.
        vault.set("github:U_1", "new").unwrap();
        vault.set("slack-client:C1", "secret").unwrap();
        vault.remove("slack:T0:U0").unwrap();
        vault.set("slack:T0:U0", "fresh").unwrap();
        assert_eq!(vault.get("github:U_1").unwrap().as_deref(), Some("new"));
        assert!(vault.status().unsaved);
        assert_eq!(*script.calls.lock().unwrap(), vec!["read"]);

        // The user finally clicks Allow: read, merge, write.
        script.reads.send(Ok(())).unwrap();
        script.writes.send(Ok(())).unwrap();
        wait_until(&vault, KeyringState::Ready);
        wait_saved(&vault);
        let saved = stored(&script);
        assert_eq!(saved["github:U_1"], "new");
        assert_eq!(saved["slack-client:C1"], "secret");
        assert_eq!(saved["slack:T0:U0"], "fresh");
        assert_eq!(*script.calls.lock().unwrap(), vec!["read", "write"]);
    }

    #[test]
    fn deny_falls_back_to_memory_and_retry_saves_after_allow() {
        let (backend, script) = fake(None);
        let vault = Vault::with_backend(Box::new(backend), PROMPT_TIMEOUT);
        script
            .reads
            .send(Err(KeyringError::Denied(
                "User canceled the operation.".into(),
            )))
            .unwrap();
        wait_until(&vault, KeyringState::Denied);
        let status = vault.status();
        assert!(status.can_retry);
        assert!(status.message.unwrap().contains("denied"));

        // Nothing is written over an entry that could not be read.
        vault.set("github:U_1", "token").unwrap();
        assert_eq!(vault.get("github:U_1").unwrap().as_deref(), Some("token"));
        assert_eq!(*script.calls.lock().unwrap(), vec!["read"]);
        assert!(script.stored.lock().unwrap().is_none());

        let status = vault.retry();
        assert_eq!(status.state, KeyringState::Waiting);
        script.reads.send(Ok(())).unwrap();
        script.writes.send(Ok(())).unwrap();
        wait_until(&vault, KeyringState::Ready);
        wait_saved(&vault);
        assert_eq!(stored(&script)["github:U_1"], "token");
    }

    #[test]
    fn each_failure_has_its_own_message() {
        let cases = [
            (
                KeyringError::NoBackend("no secret service".into()),
                KeyringState::NoBackend,
                "No keychain is available on this system (no secret service)",
            ),
            (
                KeyringError::Denied("denied".into()),
                KeyringState::Denied,
                "Keychain access was denied",
            ),
            (
                KeyringError::Other("boom".into()),
                KeyringState::Error,
                "Could not read the keychain (boom)",
            ),
        ];
        for (error, state, text) in cases {
            let (backend, script) = fake(None);
            let vault = Vault::with_backend(Box::new(backend), PROMPT_TIMEOUT);
            script.reads.send(Err(error)).unwrap();
            wait_until(&vault, state);
            assert!(
                vault.warning().unwrap().contains(text),
                "{:?}",
                vault.warning()
            );
            assert!(!vault.available());
        }
    }

    #[test]
    fn a_failed_write_is_reported_and_retried() {
        let (backend, script) = fake(None);
        let vault = Arc::new(Vault::with_backend(Box::new(backend), PROMPT_TIMEOUT));
        script.reads.send(Ok(())).unwrap();
        wait_until(&vault, KeyringState::Ready);
        script
            .writes
            .send(Err(KeyringError::Other("disk full".into())))
            .unwrap();
        vault.set("github:U_1", "token").unwrap();
        let status = vault.status();
        assert_eq!(status.state, KeyringState::Error);
        assert!(status.unsaved);
        assert!(
            status
                .message
                .unwrap()
                .contains("Could not save to the keychain (disk full)")
        );

        script.writes.send(Ok(())).unwrap();
        vault.retry();
        wait_until(&vault, KeyringState::Ready);
        wait_saved(&vault);
        assert_eq!(stored(&script)["github:U_1"], "token");
    }

    #[test]
    fn keychain_calls_never_overlap() {
        let (backend, script) = fake(None);
        let vault = Arc::new(Vault::with_backend(Box::new(backend), PROMPT_TIMEOUT));
        // Several callers and retries while the first prompt is open.
        let writers: Vec<_> = (0..4)
            .map(|n| {
                let vault = Arc::clone(&vault);
                thread::spawn(move || vault.set(&format!("k{n}"), "v").unwrap())
            })
            .collect();
        for _ in 0..3 {
            vault.retry();
        }
        for writer in writers {
            writer.join().unwrap();
        }
        script.reads.send(Ok(())).unwrap();
        for _ in 0..8 {
            script.writes.send(Ok(())).unwrap();
        }
        wait_until(&vault, KeyringState::Ready);
        wait_saved(&vault);
        assert_eq!(*script.max_active.lock().unwrap(), 1);
        assert_eq!(stored(&script).len(), 4);
        let calls = script.calls.lock().unwrap();
        assert_eq!(calls.iter().filter(|call| **call == "read").count(), 1);
    }

    #[test]
    fn get_waits_for_the_prompt_but_peek_does_not() {
        let (backend, script) = fake(Some(r#"{"github:U_1":"saved"}"#));
        let vault = Arc::new(Vault::with_backend(Box::new(backend), PROMPT_TIMEOUT));
        let reader = {
            let vault = Arc::clone(&vault);
            thread::spawn(move || vault.get("github:U_1").unwrap())
        };
        thread::sleep(Duration::from_millis(50));
        assert!(!reader.is_finished());
        assert_eq!(vault.peek("github:U_1").unwrap(), None);
        script.reads.send(Ok(())).unwrap();
        assert_eq!(reader.join().unwrap().as_deref(), Some("saved"));
    }

    #[test]
    fn denial_errors_are_recognised() {
        assert!(is_denial(
            "Error { code: -128, message: \"User canceled the operation.\" }"
        ));
        assert!(is_denial("Error { code: -25293 }"));
        assert!(is_denial("Prompt dismissed"));
        assert!(!is_denial(
            "Error { code: -25291, message: \"No keychain is available.\" }"
        ));
    }
}
