//! The runtime that runs Almond's pure core, and the one object Swift talks to.
//!
//! Swift calls [`Core::dispatch`] with an [`Event`] and reads [`Core::view`].
//! `update` runs synchronously on the caller's thread, so the next frame
//! already sees the new view. Storage commands go to one worker thread that
//! owns SQLite; platform commands go to Swift through [`Shell`].

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::thread;

use almond_core::{
    Cmd, DbCmd, Env, Event, Model, Msg, PlatformCmd, TimeZone, Timestamp, ViewModel, update, view,
};
use almond_store::Store;

uniffi::setup_scaffolding!();

/// What Swift implements. Both methods may be called from any thread.
#[uniffi::export(with_foreign)]
pub trait Shell: Send + Sync {
    /// The view changed. Read [`Core::view`] on the main thread.
    fn changed(&self);
    fn perform(&self, cmd: PlatformCmd);
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CoreError {
    #[error("could not open the database: {message}")]
    Storage { message: String },
    #[error("unknown time zone {name:?}")]
    UnknownTimeZone { name: String },
}

#[derive(uniffi::Object)]
pub struct Core {
    inner: Arc<Inner>,
}

struct Inner {
    model: Mutex<Model>,
    db: Sender<Job>,
    shell: Arc<dyn Shell>,
    tz: RwLock<TimeZone>,
    ids: AtomicU64,
    seed: RandomState,
}

enum Job {
    Run(DbCmd),
    Flush(Sender<()>),
}

#[uniffi::export]
impl Core {
    /// Opens (or creates) the database at `db_path` and loads the due queue.
    /// `time_zone` is an IANA name such as `Asia/Kolkata`
    /// (`TimeZone.current.identifier` in Swift).
    ///
    /// # Errors
    /// If the database cannot be opened or the time zone is unknown.
    #[uniffi::constructor]
    pub fn new(
        db_path: &str,
        time_zone: &str,
        shell: Arc<dyn Shell>,
    ) -> Result<Arc<Self>, CoreError> {
        let tz = time_zone_named(time_zone)?;
        let store = Store::open(db_path).map_err(|e| CoreError::Storage {
            message: e.to_string(),
        })?;
        let (tx, rx) = mpsc::channel();
        let inner = Arc::new_cyclic(|weak: &Weak<Inner>| {
            let weak = weak.clone();
            thread::Builder::new()
                .name("almond-db".into())
                .spawn(move || worker(store, &rx, &weak))
                .expect("the OS can start a thread");
            Inner {
                model: Mutex::new(Model::default()),
                db: tx,
                shell,
                tz: RwLock::new(tz),
                ids: AtomicU64::new(0),
                seed: RandomState::new(),
            }
        });
        inner.send(Event::Opened.into());
        Ok(Arc::new(Self { inner }))
    }

    /// Runs `update` for one user event. Returns once the view is updated.
    pub fn dispatch(&self, event: Event) {
        self.inner.send(event.into());
    }

    pub fn view(&self) -> ViewModel {
        view(&self.inner.model())
    }

    /// Call when the system time zone changes, then send `Opened` to reload.
    ///
    /// # Errors
    /// If the time zone is unknown; the previous one stays in effect.
    pub fn set_time_zone(&self, time_zone: &str) -> Result<(), CoreError> {
        let tz = time_zone_named(time_zone)?;
        *self
            .inner
            .tz
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = tz;
        Ok(())
    }
}

impl Core {
    /// Blocks until every storage command sent so far has run. For tests.
    pub fn flush(&self) {
        let (tx, rx) = mpsc::channel();
        if self.inner.db.send(Job::Flush(tx)).is_ok() {
            let _ = rx.recv();
        }
    }
}

impl Inner {
    fn model(&self) -> std::sync::MutexGuard<'_, Model> {
        // A panic inside `update` would be a bug; keep serving the last model.
        self.model
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn send(&self, msg: Msg) {
        let env = Env {
            now: Timestamp::now(),
            tz: self
                .tz
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
            entropy: self.entropy(),
        };
        let cmds = update(&mut self.model(), msg, &env);
        for cmd in cmds {
            match cmd {
                Cmd::Render => self.shell.changed(),
                Cmd::Db(cmd) => {
                    let _ = self.db.send(Job::Run(cmd));
                }
                Cmd::Platform(cmd) => self.shell.perform(cmd),
            }
        }
    }

    /// 128 random-enough bits for ids: a per-process random key over a counter.
    fn entropy(&self) -> u128 {
        let n = self.ids.fetch_add(1, Ordering::Relaxed);
        let mut a = self.seed.build_hasher();
        a.write_u64(n);
        let mut b = self.seed.build_hasher();
        b.write_u64(!n);
        (u128::from(a.finish()) << 64) | u128::from(b.finish())
    }
}

fn worker(mut store: Store, jobs: &Receiver<Job>, core: &Weak<Inner>) {
    for job in jobs {
        match job {
            Job::Run(cmd) => {
                let is_load = matches!(cmd, DbCmd::LoadDue { .. });
                let reply = match store.apply(cmd) {
                    Ok(reply) => reply,
                    Err(e) if is_load => Some(Msg::LoadFailed(e.to_string())),
                    // TODO: surface failed writes in the view too.
                    Err(e) => {
                        eprintln!("almond: storage command failed: {e}");
                        None
                    }
                };
                if let Some(msg) = reply {
                    match core.upgrade() {
                        Some(core) => core.send(msg),
                        None => return,
                    }
                }
            }
            Job::Flush(done) => {
                let _ = done.send(());
            }
        }
    }
}

fn time_zone_named(name: &str) -> Result<TimeZone, CoreError> {
    TimeZone::get(name).map_err(|_| CoreError::UnknownTimeZone {
        name: name.to_owned(),
    })
}
