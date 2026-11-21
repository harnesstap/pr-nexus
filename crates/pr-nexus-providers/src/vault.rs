//! One keychain entry, `dev.pr-nexus` / `vault`, holding a JSON map.
//!
//! Every keychain call runs on one worker thread, and concurrent callers share
//! the call already in flight. After a successful read the map stays in memory
//! for the life of the process; later syncs and a wake from sleep do not read
//! it again. A write or delete updates that map, and [`Vault::retry`] is what
//! asks the keychain again.
//!
//! The first read is interactive and starts at launch ([`Vault::prompt_at_launch`])
//! or when the window is opened ([`Vault::note_focus`]). Background callers wait
//! on that pass: a silent first read is never started, because a locked macOS
//! keychain with the prompt disabled can stall the worker for tens of seconds
//! (`SecKeychainSetUserInteractionAllowed`). A write the caller waits on may
//! also show one prompt, so saving a credential is not stuck behind a silent
//! call. Background calls otherwise disable the macOS prompt. A locked
//! keychain, or Cancel / Deny, is remembered: queued work is dropped and
//! nothing calls the keychain until [`Vault::retry`] or the window is opened.
//! A prompt the user refused is not shown again on focus. The vault never
//! writes before a successful read, so a failed read cannot overwrite the
//! stored entry.

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

    /// `interactive` is set only for a user action (Retry, or opening the
    /// window). Background passes use `false`, which on macOS must not show
    /// a prompt. The default ignores the flag; [`SystemKeyring`] does not.
    fn read_interactive(&self, interactive: bool) -> Result<Option<String>, KeyringError> {
        let _ = interactive;
        self.read()
    }

    fn write_interactive(&self, json: &str, interactive: bool) -> Result<(), KeyringError> {
        let _ = interactive;
        self.write(json)
    }
}

/// Why a keychain call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyringError {
    /// No credential store on this system (no Secret Service, no keychain).
    NoBackend(String),
    /// The user refused access (Deny on the macOS prompt, a dismissed unlock).
    Denied(String),
    /// The keychain is locked, or UI was not allowed
    /// (`errSecInteractionNotAllowed`, `errSecNoAccessForItem`).
    Locked(String),
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
    /// The next pass may show a prompt. Consumed when that pass starts.
    allow_prompt: bool,
    /// The user refused an interactive prompt. Focus will not ask again.
    refused: bool,
    /// An interactive call is in progress, so another one must not start.
    prompt_open: bool,
    /// The window is focused. Focus prompts only on the rising edge.
    focused: bool,
    /// `get` calls blocked on the in-flight read. Tests wait on this.
    #[cfg(test)]
    waiters: usize,
}

impl Inner {
    fn fresh(phase: Phase, loaded: bool) -> Self {
        Self {
            phase,
            loaded,
            secrets: BTreeMap::new(),
            early: BTreeMap::new(),
            revision: 0,
            saved: 0,
            allow_prompt: false,
            refused: false,
            prompt_open: false,
            focused: false,
            #[cfg(test)]
            waiters: 0,
        }
    }
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
    /// change the keychain does not hold yet. A first read, or a write the
    /// caller is waiting on, may show one prompt. Other background passes
    /// never show one, and a silent first read is skipped so it cannot stall
    /// the worker. After a failure, a pass that was already queued does
    /// nothing until [`Vault::retry`] or [`Vault::note_focus`].
    fn sync(&self) {
        let Some(backend) = &self.backend else {
            return;
        };
        let interactive = {
            let Ok(mut inner) = self.lock() else {
                return;
            };
            if matches!(inner.phase, Phase::Disabled { .. }) || inner.prompt_open {
                // A dialog is already up. Dropping the flag here means the
                // queued request cannot open a second one when this call returns.
                inner.allow_prompt = false;
                return;
            }
            let interactive = inner.allow_prompt;
            inner.allow_prompt = false;
            if matches!(inner.phase, Phase::Failed { .. }) && !interactive {
                return;
            }
            if interactive {
                inner.refused = false;
                inner.prompt_open = true;
                if matches!(inner.phase, Phase::Failed { .. }) {
                    inner.phase = Phase::Waiting {
                        since: Instant::now(),
                    };
                }
            }
            interactive
        };
        let Ok(loaded) = self.lock().map(|inner| inner.loaded) else {
            self.clear_prompt();
            return;
        };
        if !loaded {
            if !interactive {
                // A locked keychain with interaction disabled can stall the
                // worker for tens of seconds before returning
                // errSecInteractionNotAllowed. Leave the phase as Waiting so
                // callers share a later interactive pass.
                return;
            }
            tracing::info!(interactive, "keychain access starting");
            let result = backend
                .read_interactive(interactive)
                .and_then(|json| decode(json.as_deref()));
            let Ok(mut inner) = self.lock() else {
                self.clear_prompt();
                return;
            };
            match result {
                Ok(mut stored) => {
                    for (key, value) in std::mem::take(&mut inner.early) {
                        match value {
                            Some(value) => {
                                stored.insert(key, value);
                            }
                            None => {
                                stored.remove(&key);
                            }
                        }
                    }
                    inner.secrets = stored;
                    inner.loaded = true;
                    inner.refused = false;
                    inner.phase = Phase::Ready;
                }
                Err(error) => {
                    inner.prompt_open = false;
                    inner.refused = interactive && matches!(error, KeyringError::Denied(_));
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
                let Ok(mut inner) = self.lock() else {
                    self.clear_prompt();
                    return;
                };
                if !inner.unsaved() {
                    // A retry with nothing left to save still ends the wait,
                    // so later changes are written again.
                    if matches!(inner.phase, Phase::Waiting { .. }) || inner.prompt_open {
                        inner.phase = Phase::Ready;
                        inner.prompt_open = false;
                        inner.refused = false;
                        drop(inner);
                        self.changed.notify_all();
                    }
                    return;
                }
                match serde_json::to_string(&inner.secrets) {
                    Ok(json) => (json, inner.revision),
                    Err(err) => {
                        drop(inner);
                        self.fail(
                            KeyringError::Other(format!("encode: {err}")),
                            Op::Write,
                            interactive,
                        );
                        return;
                    }
                }
            };
            tracing::info!(interactive, "keychain write starting");
            match backend.write_interactive(&json, interactive) {
                Ok(()) => {
                    if let Ok(mut inner) = self.lock() {
                        inner.saved = inner.saved.max(revision);
                        inner.phase = Phase::Ready;
                        inner.prompt_open = false;
                        inner.refused = false;
                    }
                    self.changed.notify_all();
                }
                Err(error) => {
                    self.fail(error, Op::Write, interactive);
                    return;
                }
            }
        }
    }

    fn clear_prompt(&self) {
        if let Ok(mut inner) = self.lock() {
            inner.prompt_open = false;
        }
        self.changed.notify_all();
    }

    fn fail(&self, error: KeyringError, op: Op, interactive: bool) {
        if let Ok(mut inner) = self.lock() {
            inner.prompt_open = false;
            inner.refused = interactive && matches!(error, KeyringError::Denied(_));
            inner.phase = Phase::Failed { error, op };
        }
        self.changed.notify_all();
    }

    /// Blocks while the first read is pending, up to the timeout.
    fn wait_for_read(&self) -> Result<MutexGuard<'_, Inner>, CoreError> {
        let mut inner = self.lock()?;
        #[cfg(test)]
        {
            inner.waiters += 1;
            self.changed.notify_all();
        }
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
        #[cfg(test)]
        {
            inner.waiters = inner.waiters.saturating_sub(1);
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

/// A request to the keyring worker: run a sync pass. A test can attach a
/// sender, answered once every request queued before it has been handled.
type Job = Option<mpsc::Sender<()>>;

pub struct Vault {
    shared: Arc<Shared>,
    jobs: Option<mpsc::Sender<Job>>,
    /// Prefix added to every key: empty for the default profile, so keys
    /// saved before profiles existed keep their names; `profile/<id>/`
    /// otherwise. All profiles share the one keychain entry, so adding a
    /// profile never brings up another keychain prompt.
    scope: std::sync::RwLock<String>,
}

/// The key prefix for a profile's secrets. The default profile has none.
pub fn profile_scope(profile: &str) -> String {
    if profile == pr_nexus_core::profiles::DEFAULT_ID {
        String::new()
    } else {
        format!("profile/{profile}/")
    }
}

impl Vault {
    /// The system keychain, unless `PR_NEXUS_DISABLE_KEYRING` is set.
    /// Returns at once: the first read starts from [`Vault::prompt_at_launch`]
    /// or [`Vault::note_focus`], not here.
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
    /// counts as *waiting* before it is reported as timed out. The first read
    /// is not started here; call [`Vault::prompt_at_launch`] or
    /// [`Vault::note_focus`].
    pub fn with_backend(backend: Box<dyn KeyringBackend>, timeout: Duration) -> Self {
        let shared = Arc::new(Shared {
            inner: Mutex::new(Inner::fresh(
                Phase::Waiting {
                    since: Instant::now(),
                },
                false,
            )),
            changed: Condvar::new(),
            backend: Some(backend),
            timeout,
        });
        let (tx, rx) = mpsc::channel::<Job>();
        let worker = Arc::clone(&shared);
        let spawned = thread::Builder::new()
            .name("pr-nexus-keyring".into())
            .spawn(move || {
                while let Ok(first) = rx.recv() {
                    // Coalesce queued requests: one pass covers them all.
                    let mut sync = false;
                    let mut flushed = Vec::new();
                    for job in std::iter::once(first).chain(rx.try_iter()) {
                        match job {
                            None => sync = true,
                            Some(done) => flushed.push(done),
                        }
                    }
                    if sync {
                        worker.sync();
                    }
                    for done in flushed {
                        let _ = done.send(());
                    }
                }
            });
        let vault = match spawned {
            Ok(_) => Self {
                shared,
                jobs: Some(tx),
                scope: Default::default(),
            },
            Err(err) => {
                return Self::memory(&format!(
                    "Keyring unavailable (could not start its worker: {err}). Tokens stay in memory for this session and are lost when the app quits."
                ));
            }
        };
        vault
    }

    /// An in-memory vault that never touches a keychain.
    pub fn memory(warning: &str) -> Self {
        Self {
            shared: Arc::new(Shared {
                inner: Mutex::new(Inner::fresh(
                    Phase::Disabled {
                        message: warning.to_owned(),
                    },
                    true,
                )),
                changed: Condvar::new(),
                backend: None,
                timeout: Duration::ZERO,
            }),
            jobs: None,
            scope: Default::default(),
        }
    }

    /// The same keychain entry seen through `profile`'s key prefix.
    pub fn view(&self, profile: &str) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            jobs: self.jobs.clone(),
            scope: std::sync::RwLock::new(profile_scope(profile)),
        }
    }

    /// The same entry without a prefix, for app-wide secrets such as the
    /// MCP token.
    pub fn global(&self) -> Self {
        self.view(pr_nexus_core::profiles::DEFAULT_ID)
    }

    /// Points this vault at `profile`'s secrets (profile switch).
    pub fn set_profile(&self, profile: &str) {
        if let Ok(mut scope) = self.scope.write() {
            *scope = profile_scope(profile);
        }
    }

    fn key(&self, key: &str) -> String {
        let scope = self.scope.read().map(|s| s.clone()).unwrap_or_default();
        format!("{scope}{key}")
    }

    /// Copies `keys` from `from`'s secrets into `to`'s. Missing keys are
    /// skipped. The copies are independent of the originals.
    pub fn copy_between(&self, from: &str, to: &str, keys: &[String]) -> Result<(), CoreError> {
        let (from, to) = (profile_scope(from), profile_scope(to));
        let values: Vec<(String, String)> = {
            let inner = self.shared.wait_for_read()?;
            keys.iter()
                .filter_map(|key| {
                    inner
                        .secrets
                        .get(&format!("{from}{key}"))
                        .map(|value| (format!("{to}{key}"), value.clone()))
                })
                .collect()
        };
        for (key, value) in values {
            self.change_raw(&key, Some(&value))?;
        }
        Ok(())
    }

    /// Removes every secret of a non-default `profile`.
    pub fn remove_profile(&self, profile: &str) -> Result<usize, CoreError> {
        let scope = profile_scope(profile);
        if scope.is_empty() {
            return Err(CoreError::Store(
                "the default profile's secrets are not removed".into(),
            ));
        }
        let keys: Vec<String> = {
            let inner = self.shared.wait_for_read()?;
            inner
                .secrets
                .keys()
                .filter(|key| key.starts_with(&scope))
                .cloned()
                .collect()
        };
        for key in &keys {
            self.change_raw(key, None)?;
        }
        Ok(keys.len())
    }

    fn kick(&self) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(None);
        }
    }

    /// Waits until the worker has handled every request queued so far, so a
    /// test can check what it did, or did not do, without sleeping.
    #[cfg(test)]
    fn flush(&self) {
        let (done, finished) = mpsc::channel();
        if let Some(jobs) = &self.jobs {
            jobs.send(Some(done)).expect("keyring worker is gone");
            finished
                .recv_timeout(Duration::from_secs(5))
                .expect("keyring worker is stuck in a keychain call");
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
    /// what is only in memory. Does not wait for the answer. This is a user
    /// action, so the pass may show one prompt.
    pub fn retry(&self) -> KeyringStatus {
        if self.jobs.is_some()
            && let Ok(mut inner) = self.shared.lock()
            && !matches!(inner.phase, Phase::Ready | Phase::Disabled { .. })
            && !inner.prompt_open
        {
            inner.refused = false;
            inner.allow_prompt = true;
            inner.phase = Phase::Waiting {
                since: Instant::now(),
            };
        }
        self.kick();
        self.status()
    }

    /// The window gained or lost focus. Opening it (the rising edge) may
    /// show one prompt when the keychain was locked and the user has not
    /// just refused one. A second focus event while the window stays focused
    /// does not ask again.
    pub fn note_focus(&self, focused: bool) {
        if self.jobs.is_none() {
            return;
        }
        let kick = {
            let Ok(mut inner) = self.shared.lock() else {
                return;
            };
            if !focused {
                inner.focused = false;
                return;
            }
            let rising = !inner.focused;
            inner.focused = true;
            if !rising
                || inner.refused
                || inner.prompt_open
                || matches!(inner.phase, Phase::Ready | Phase::Disabled { .. })
            {
                return;
            }
            inner.allow_prompt = true;
            true
        };
        if kick {
            self.kick();
        }
    }

    /// Start the first keychain read as a user-visible prompt. Call this once
    /// at app start, before background sync or token work. Background callers
    /// wait on this pass instead of starting a silent read.
    pub fn prompt_at_launch(&self) {
        if self.jobs.is_none() {
            return;
        }
        let kick = {
            let Ok(mut inner) = self.shared.lock() else {
                return;
            };
            if inner.loaded
                || inner.prompt_open
                || matches!(inner.phase, Phase::Ready | Phase::Disabled { .. })
            {
                return;
            }
            inner.allow_prompt = true;
            true
        };
        if kick {
            self.kick();
        }
    }

    /// Block until `n` [`Vault::get`] calls are waiting on the in-flight read.
    #[cfg(test)]
    fn wait_until_getters(&self, n: usize) {
        let mut inner = self.shared.lock().expect("vault lock");
        while inner.waiters < n {
            inner = self.shared.changed.wait(inner).expect("vault lock");
        }
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
        let key = self.key(key);
        Ok(self.shared.wait_for_read()?.secrets.get(&key).cloned())
    }

    /// The secret under `key` as known now, without waiting for the keychain.
    /// Use it for display: before the first read it only sees this session's
    /// changes.
    pub fn peek(&self, key: &str) -> Result<Option<String>, CoreError> {
        let key = self.key(key);
        Ok(self.shared.lock()?.secrets.get(&key).cloned())
    }

    /// Saves `value` in memory at once, then in the keychain. When the
    /// keychain is ready this waits for the write and may show one prompt: a
    /// silent write can stall the worker for tens of seconds when the
    /// keychain is locked. While a keychain call is pending the value is
    /// written once it answers; after a failure (Deny, no keychain, an
    /// error) it stays in memory until [`Vault::retry`] succeeds, and no
    /// keychain call is made, so no prompt is shown. A failed write shows
    /// in [`Vault::status`], not as an error.
    pub fn set(&self, key: &str, value: &str) -> Result<(), CoreError> {
        self.change(key, Some(value))
    }

    pub fn remove(&self, key: &str) -> Result<(), CoreError> {
        self.change(key, None)
    }

    fn change(&self, key: &str, value: Option<&str>) -> Result<(), CoreError> {
        let key = self.key(key);
        self.change_raw(&key, value)
    }

    fn change_raw(&self, key: &str, value: Option<&str>) -> Result<(), CoreError> {
        let (revision, ready) = {
            let mut inner = self.shared.lock()?;
            let revision = inner.change(key, value);
            (revision, matches!(inner.phase, Phase::Ready))
        };
        // Only a ready keychain is written on a change. While a call is
        // pending, the pass in flight saves the change once it answers: it
        // sets `Ready` or checks for unsaved changes under this lock, so it
        // either sees this change or this change sees `Ready` and kicks.
        // After a failure nothing is sent: calling the keychain again would
        // bring back the prompt the user just refused.
        if self.jobs.is_none() || !ready {
            return Ok(());
        }
        // The caller waits for this write. A silent keychain write can stall
        // the worker for tens of seconds when the keychain is locked; allow
        // one prompt instead.
        if let Ok(mut inner) = self.shared.lock() {
            if !inner.prompt_open {
                inner.allow_prompt = true;
            }
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
        (KeyringError::Locked(_), _) => (
            KeyringState::Denied,
            format!("Keychain access needed. {LOST} Retry to ask again."),
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

    fn read_interactive(&self, interactive: bool) -> Result<Option<String>, KeyringError> {
        let _guard = PromptSuppression::unless(interactive);
        self.read()
    }

    fn write_interactive(&self, json: &str, interactive: bool) -> Result<(), KeyringError> {
        let _guard = PromptSuppression::unless(interactive);
        self.write(json)
    }
}

/// Turns the macOS keychain dialog off for one background call. The flag is
/// process-wide; the worker is the only caller, so calls do not overlap.
struct PromptSuppression {
    #[cfg(target_os = "macos")]
    restore: bool,
}

impl PromptSuppression {
    fn unless(interactive: bool) -> Self {
        #[cfg(target_os = "macos")]
        {
            if interactive {
                return Self { restore: false };
            }
            unsafe {
                SecKeychainSetUserInteractionAllowed(0);
            }
            return Self { restore: true };
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = interactive;
            Self {}
        }
    }
}

impl Drop for PromptSuppression {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        if self.restore {
            unsafe {
                SecKeychainSetUserInteractionAllowed(1);
            }
        }
    }
}

#[cfg(target_os = "macos")]
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecKeychainSetUserInteractionAllowed(allowed: u8) -> i32;
}

fn classify(err: keyring::Error) -> KeyringError {
    let reason = err.to_string();
    let detail = format!("{err:?}");
    match &err {
        keyring::Error::NoDefaultStore | keyring::Error::NotSupportedByStore(_) => {
            KeyringError::NoBackend(reason)
        }
        keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_)
            if is_locked(&detail) =>
        {
            KeyringError::Locked(reason)
        }
        // Secret Service reports a dismissed unlock prompt as no access.
        keyring::Error::NoStorageAccess(_) if is_denial(&detail) => KeyringError::Denied(reason),
        keyring::Error::NoStorageAccess(_) => KeyringError::NoBackend(reason),
        keyring::Error::PlatformFailure(_) if is_denial(&detail) => KeyringError::Denied(reason),
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

/// A locked keychain, or a call that was not allowed to show UI.
/// `errSecInteractionNotAllowed` is -25308, `errSecNoAccessForItem` is -25243.
fn is_locked(detail: &str) -> bool {
    let detail = detail.to_ascii_lowercase();
    [
        "-25308",
        "-25243",
        "interactionnotallowed",
        "interaction not allowed",
        "noaccessforitem",
    ]
    .iter()
    .any(|needle| detail.contains(needle))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

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
        vault.prompt_at_launch();
        script.reads.send(Ok(())).unwrap();
        assert_eq!(vault.get("github:U_1").unwrap().as_deref(), Some("saved"));
        assert!(vault.available());
        assert_eq!(vault.warning(), None);
    }

    #[test]
    fn a_slow_prompt_times_out_then_a_late_allow_saves_the_session_tokens() {
        let (backend, script) = fake(Some(r#"{"github:U_1":"old","slack:T0:U0":"kept"}"#));
        let vault = Vault::with_backend(Box::new(backend), Duration::from_millis(50));
        vault.prompt_at_launch();
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
        let original = r#"{"slack:T0:U0":"kept"}"#;
        let (backend, script) = fake(Some(original));
        let vault = Vault::with_backend(Box::new(backend), PROMPT_TIMEOUT);
        vault.prompt_at_launch();
        script
            .reads
            .send(Err(KeyringError::Denied(
                "User canceled the operation.".into(),
            )))
            .unwrap();
        vault.flush();
        let status = vault.status();
        assert_eq!(status.state, KeyringState::Denied);
        assert!(status.can_retry);
        assert!(status.message.unwrap().contains("denied"));

        // Saving after a Deny keeps tokens in memory and never calls the
        // keychain again, so no second prompt appears.
        vault.set("github:U_1", "token").unwrap();
        vault.set("slack-client:C1", "secret").unwrap();
        vault.remove("slack-client:C1").unwrap();
        vault.flush();
        assert_eq!(*script.calls.lock().unwrap(), vec!["read"]);
        assert_eq!(script.stored.lock().unwrap().as_deref(), Some(original));
        assert_eq!(vault.get("github:U_1").unwrap().as_deref(), Some("token"));
        let status = vault.status();
        assert_eq!(status.state, KeyringState::Denied);
        assert!(status.unsaved);

        // Only Retry asks again. The read is held open like a prompt on
        // screen until the test answers it.
        assert_eq!(vault.retry().state, KeyringState::Waiting);
        script.reads.send(Ok(())).unwrap();
        script.writes.send(Ok(())).unwrap();
        vault.flush();
        assert_eq!(*script.calls.lock().unwrap(), vec!["read", "read", "write"]);
        let status = vault.status();
        assert_eq!(status.state, KeyringState::Ready);
        assert!(!status.unsaved);
        let saved = stored(&script);
        assert_eq!(saved["github:U_1"], "token");
        assert_eq!(saved["slack:T0:U0"], "kept");
        assert!(!saved.contains_key("slack-client:C1"));
    }

    #[test]
    fn a_denied_write_is_not_asked_again_until_retry() {
        let (backend, script) = fake(None);
        let vault = Vault::with_backend(Box::new(backend), PROMPT_TIMEOUT);
        vault.prompt_at_launch();
        script.reads.send(Ok(())).unwrap();
        vault.flush();
        assert_eq!(vault.status().state, KeyringState::Ready);

        script
            .writes
            .send(Err(KeyringError::Denied("denied".into())))
            .unwrap();
        vault.set("github:U_1", "first").unwrap();
        vault.flush();
        assert_eq!(vault.status().state, KeyringState::Denied);

        // The next save stays in memory: no write, no prompt.
        vault.set("slack:T0:U0", "second").unwrap();
        vault.flush();
        assert_eq!(*script.calls.lock().unwrap(), vec!["read", "write"]);
        assert!(script.stored.lock().unwrap().is_none());
        assert!(vault.status().unsaved);

        script.writes.send(Ok(())).unwrap();
        vault.retry();
        vault.flush();
        assert_eq!(
            *script.calls.lock().unwrap(),
            vec!["read", "write", "write"]
        );
        assert_eq!(vault.status().state, KeyringState::Ready);
        let saved = stored(&script);
        assert_eq!(saved["github:U_1"], "first");
        assert_eq!(saved["slack:T0:U0"], "second");
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
                KeyringError::Locked("errSecInteractionNotAllowed -25308".into()),
                KeyringState::Denied,
                "Keychain access needed",
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
            vault.prompt_at_launch();
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
        vault.prompt_at_launch();
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
        vault.prompt_at_launch();
        let reader = {
            let vault = Arc::clone(&vault);
            thread::spawn(move || vault.get("github:U_1").unwrap())
        };
        vault.wait_until_getters(1);
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
        assert!(is_locked("Error { code: -25308 }"));
        assert!(is_locked(
            "errSecInteractionNotAllowed: interaction not allowed"
        ));
        assert!(is_locked("Error { code: -25243 }"));
        assert!(is_locked("errSecNoAccessForItem"));
        assert!(!is_locked("Error { code: -128 }"));
        assert!(!is_locked("Error { code: -25293 }"));
        assert!(!is_denial("Error { code: -25308 }"));
        assert!(!is_denial("Error { code: -25243 }"));
    }

    #[test]
    fn profiles_see_only_their_own_secrets() {
        let vault = Vault::memory("test");
        vault.set("github:1", "default-token").unwrap();
        let work = vault.view("work-1");
        assert_eq!(work.get("github:1").unwrap(), None);
        work.set("github:1", "work-token").unwrap();
        assert_eq!(
            vault.get("github:1").unwrap().as_deref(),
            Some("default-token")
        );
        assert_eq!(work.get("github:1").unwrap().as_deref(), Some("work-token"));
        vault.set_profile("work-1");
        assert_eq!(
            vault.get("github:1").unwrap().as_deref(),
            Some("work-token")
        );
        assert_eq!(
            vault.global().get("github:1").unwrap().as_deref(),
            Some("default-token")
        );
    }

    #[test]
    fn copies_are_independent_and_removal_is_scoped() {
        let vault = Vault::memory("test");
        vault.set("slack:T1:U1", "xoxp-a").unwrap();
        vault.set("mcp.token", "global").unwrap();
        vault
            .copy_between(
                "default",
                "work-1",
                &["slack:T1:U1".into(), "missing".into()],
            )
            .unwrap();
        let work = vault.view("work-1");
        assert_eq!(work.get("slack:T1:U1").unwrap().as_deref(), Some("xoxp-a"));
        assert_eq!(work.get("missing").unwrap(), None);
        work.set("slack:T1:U1", "xoxp-b").unwrap();
        assert_eq!(vault.get("slack:T1:U1").unwrap().as_deref(), Some("xoxp-a"));
        assert_eq!(vault.remove_profile("work-1").unwrap(), 1);
        assert_eq!(work.get("slack:T1:U1").unwrap(), None);
        assert_eq!(vault.get("slack:T1:U1").unwrap().as_deref(), Some("xoxp-a"));
        assert_eq!(vault.get("mcp.token").unwrap().as_deref(), Some("global"));
        assert!(vault.remove_profile("default").is_err());
    }

    struct Count {
        reads: Arc<AtomicUsize>,
        writes: Arc<AtomicUsize>,
        /// How the nth read answers. Missing entries succeed with `json`.
        answers: Mutex<Vec<Result<(), KeyringError>>>,
        json: String,
        entered: Arc<(Mutex<bool>, Condvar)>,
        release: Arc<(Mutex<bool>, Condvar)>,
        hold_first: bool,
        interactive_reads: Arc<Mutex<Vec<bool>>>,
        interactive_writes: Arc<Mutex<Vec<bool>>>,
    }

    impl Count {
        fn open(json: &str, hold_first: bool) -> (Self, Arc<AtomicUsize>, Arc<AtomicUsize>) {
            let reads = Arc::new(AtomicUsize::new(0));
            let writes = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    reads: Arc::clone(&reads),
                    writes: Arc::clone(&writes),
                    answers: Mutex::new(Vec::new()),
                    json: json.to_owned(),
                    entered: Arc::new((Mutex::new(false), Condvar::new())),
                    release: Arc::new((Mutex::new(!hold_first), Condvar::new())),
                    hold_first,
                    interactive_reads: Arc::new(Mutex::new(Vec::new())),
                    interactive_writes: Arc::new(Mutex::new(Vec::new())),
                },
                reads,
                writes,
            )
        }
    }

    impl KeyringBackend for Count {
        fn read(&self) -> Result<Option<String>, KeyringError> {
            let n = self.reads.fetch_add(1, Ordering::SeqCst);
            if n == 0 && self.hold_first {
                {
                    let (lock, cv) = &*self.entered;
                    *lock.lock().expect("entered") = true;
                    cv.notify_all();
                }
                let (lock, cv) = &*self.release;
                let mut guard = lock.lock().expect("hold");
                while !*guard {
                    guard = cv.wait(guard).expect("hold");
                }
            }
            let answer = self.answers.lock().expect("answers").pop();
            match answer {
                Some(Err(err)) => Err(err),
                _ => Ok(Some(self.json.clone())),
            }
        }

        fn write(&self, _json: &str) -> Result<(), KeyringError> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn read_interactive(&self, interactive: bool) -> Result<Option<String>, KeyringError> {
            self.interactive_reads
                .lock()
                .expect("interactive reads")
                .push(interactive);
            self.read()
        }

        fn write_interactive(&self, json: &str, interactive: bool) -> Result<(), KeyringError> {
            self.interactive_writes
                .lock()
                .expect("interactive writes")
                .push(interactive);
            self.write(json)
        }
    }

    #[test]
    fn fifty_concurrent_readers_share_one_keychain_call_then_use_the_cache() {
        let (backend, reads, _) = Count::open(r#"{"acme":"token"}"#, true);
        let entered = Arc::clone(&backend.entered);
        let release = Arc::clone(&backend.release);
        let vault = Arc::new(Vault::with_backend(Box::new(backend), PROMPT_TIMEOUT));
        vault.prompt_at_launch();
        {
            let (lock, cv) = &*entered;
            let mut guard = lock.lock().expect("entered");
            while !*guard {
                guard = cv.wait(guard).expect("entered");
            }
        }
        let readers: Vec<_> = (0..50)
            .map(|_| {
                let vault = Arc::clone(&vault);
                thread::spawn(move || vault.get("acme").unwrap())
            })
            .collect();
        vault.wait_until_getters(50);
        {
            let (lock, cv) = &*release;
            *lock.lock().expect("hold") = true;
            cv.notify_all();
        }
        for reader in readers {
            assert_eq!(reader.join().unwrap().as_deref(), Some("token"));
        }
        assert_eq!(reads.load(Ordering::SeqCst), 1);

        let again: Vec<_> = (0..50)
            .map(|_| {
                let vault = Arc::clone(&vault);
                thread::spawn(move || vault.get("acme").unwrap())
            })
            .collect();
        for reader in again {
            assert_eq!(reader.join().unwrap().as_deref(), Some("token"));
        }
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_cancel_stops_background_reads_until_retry() {
        let (backend, reads, writes) = Count::open(r#"{"acme":"token"}"#, false);
        backend
            .answers
            .lock()
            .expect("answers")
            .push(Err(KeyringError::Denied("errSecUserCanceled -128".into())));
        // `pop` takes the last item, so the denial has to be the only answer
        // and the launch read has to run before we push a success.
        let vault = Arc::new(Vault::with_backend(Box::new(backend), PROMPT_TIMEOUT));
        vault.flush();
        assert_eq!(reads.load(Ordering::SeqCst), 0, "construct starts no read");
        vault.prompt_at_launch();
        vault.flush();
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert!(
            vault
                .status()
                .message
                .unwrap()
                .contains("Keychain access was denied")
        );

        let readers: Vec<_> = (0..50)
            .map(|_| {
                let vault = Arc::clone(&vault);
                thread::spawn(move || {
                    let _ = vault.get("acme").unwrap();
                    vault.set("slack:T0ACME:U0ACME", "xoxp-acme").unwrap();
                })
            })
            .collect();
        for reader in readers {
            reader.join().unwrap();
        }
        vault.flush();
        assert_eq!(reads.load(Ordering::SeqCst), 1, "background reads");
        assert_eq!(writes.load(Ordering::SeqCst), 0, "background writes");

        // The startup answer was popped. The retry read succeeds.
        vault.retry();
        vault.flush();
        assert_eq!(reads.load(Ordering::SeqCst), 2);
        assert_eq!(
            vault.get("acme").unwrap().as_deref(),
            Some("token"),
            "a later read is the cached value"
        );
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_locked_keychain_prompts_once_on_focus_and_not_again_after_cancel() {
        let (backend, reads, writes) = Count::open(r#"{"acme":"token"}"#, false);
        {
            let mut answers = backend.answers.lock().expect("answers");
            // pop() reads from the back: cancel on the first focus prompt.
            answers.push(Err(KeyringError::Denied("errSecUserCanceled -128".into())));
        }
        let vault = Arc::new(Vault::with_backend(Box::new(backend), PROMPT_TIMEOUT));
        vault.flush();
        assert_eq!(reads.load(Ordering::SeqCst), 0, "construct starts no read");
        assert_eq!(vault.status().state, KeyringState::Waiting);

        vault.set("acme", "local").unwrap();
        vault.flush();
        assert_eq!(writes.load(Ordering::SeqCst), 0);
        assert_eq!(
            reads.load(Ordering::SeqCst),
            0,
            "a save before the first read does not call the keychain"
        );

        vault.note_focus(true);
        vault.flush();
        assert_eq!(
            reads.load(Ordering::SeqCst),
            1,
            "opening the window asks once"
        );
        vault.note_focus(true);
        vault.note_focus(false);
        vault.note_focus(true);
        vault.flush();
        assert_eq!(
            reads.load(Ordering::SeqCst),
            1,
            "a refused prompt is not shown again on focus"
        );
        assert_eq!(writes.load(Ordering::SeqCst), 0);

        vault.retry();
        vault.flush();
        assert_eq!(reads.load(Ordering::SeqCst), 2);
        assert_eq!(vault.get("acme").unwrap().as_deref(), Some("local"));
    }

    #[test]
    fn queued_retries_while_a_prompt_is_open_do_not_stack() {
        let (backend, reads, _) = Count::open(r#"{"acme":"token"}"#, true);
        let entered = Arc::clone(&backend.entered);
        let release = Arc::clone(&backend.release);
        let vault = Arc::new(Vault::with_backend(Box::new(backend), PROMPT_TIMEOUT));
        vault.prompt_at_launch();
        wait_entered_arc(&entered);
        for _ in 0..50 {
            vault.retry();
        }
        {
            let (lock, cv) = &*release;
            *lock.lock().expect("hold") = true;
            cv.notify_all();
        }
        vault.flush();
        // The in-flight read, plus at most one coalesced follow-up.
        let calls = reads.load(Ordering::SeqCst);
        assert!(calls <= 2, "keychain calls: {calls}");
        assert!(calls >= 1);
    }

    #[test]
    fn background_callers_wait_for_an_interactive_first_read() {
        let (backend, reads, writes) = Count::open(r#"{"acme":"token"}"#, true);
        let entered = Arc::clone(&backend.entered);
        let release = Arc::clone(&backend.release);
        let interactive_reads = Arc::clone(&backend.interactive_reads);
        let vault = Arc::new(Vault::with_backend(Box::new(backend), PROMPT_TIMEOUT));
        vault.flush();
        assert_eq!(reads.load(Ordering::SeqCst), 0);

        let readers: Vec<_> = (0..20)
            .map(|_| {
                let vault = Arc::clone(&vault);
                thread::spawn(move || vault.get("acme").unwrap())
            })
            .collect();
        vault.wait_until_getters(20);
        vault.flush();
        assert_eq!(
            reads.load(Ordering::SeqCst),
            0,
            "background getters must not start a silent first read"
        );

        vault.prompt_at_launch();
        wait_entered_arc(&entered);
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert_eq!(
            *interactive_reads.lock().expect("interactive reads"),
            vec![true],
            "the first read must be allowed to prompt"
        );
        {
            let (lock, cv) = &*release;
            *lock.lock().expect("hold") = true;
            cv.notify_all();
        }
        for reader in readers {
            assert_eq!(reader.join().unwrap().as_deref(), Some("token"));
        }
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert_eq!(writes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_waiting_write_is_allowed_to_prompt() {
        let (backend, reads, writes) = Count::open(r#"{"acme":"old"}"#, false);
        let interactive_writes = Arc::clone(&backend.interactive_writes);
        let vault = Vault::with_backend(Box::new(backend), PROMPT_TIMEOUT);
        vault.prompt_at_launch();
        vault.flush();
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert_eq!(vault.status().state, KeyringState::Ready);

        vault.set("acme", "fresh").unwrap();
        vault.flush();
        assert_eq!(writes.load(Ordering::SeqCst), 1);
        assert_eq!(
            *interactive_writes.lock().expect("interactive writes"),
            vec![true],
            "a write the caller waits on must be allowed to prompt"
        );
        assert_eq!(vault.get("acme").unwrap().as_deref(), Some("fresh"));
    }

    fn wait_entered_arc(entered: &(Mutex<bool>, Condvar)) {
        let (lock, cv) = entered;
        let mut guard = lock.lock().expect("entered");
        while !*guard {
            guard = cv.wait(guard).expect("entered");
        }
    }
}
