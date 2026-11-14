//! Browser sessions share one node and immutable image analysis, never wallets,
//! enrollments, nonces, approvals or balances. Expiry is absolute, not sliding.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use tokio::{
    sync::{Semaphore, watch},
    task::JoinHandle,
};

use super::{
    chain::Sandbox, config::Config, fingerprint::FingerprintTemplate, model::hex, runtime,
    service::Session,
};

pub const SESSION_LIFETIME: Duration = Duration::from_secs(3600);
const MAX_SESSIONS: usize = 16;

pub struct Resources {
    pub sandbox: Sandbox,
    pub fingerprints: [Arc<FingerprintTemplate>; 2],
    /// Owned permits stay with blocking workers even if an HTTP task is canceled.
    pub work: Arc<Semaphore>,
}

#[derive(Clone)]
pub enum Startup {
    Preparing(String),
    Ready(Arc<Resources>),
    Failed(String),
    Stopped,
}

#[derive(Debug)]
pub enum SessionError {
    Expired,
    Capacity,
    Internal(anyhow::Error),
}

#[derive(Clone)]
pub struct Demo {
    pub config: Config,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    startup: watch::Sender<Startup>,
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
    retiring: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl Demo {
    pub fn new(config: Config) -> Result<Self> {
        let (startup, _) = watch::channel(Startup::Preparing(
            "Preparing the local NEAR environment".into(),
        ));
        Ok(Self {
            config,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            startup,
            tasks: Arc::new(Mutex::new(Vec::new())),
            retiring: Arc::new(Mutex::new(Vec::new())),
        })
    }

    pub fn start(&self) {
        let demo = self.clone();
        let initialize = tokio::spawn(async move {
            if let Err(error) = demo.prepare().await {
                eprintln!("Demo startup failed: {error:#}");
                demo.startup
                    .send_replace(Startup::Failed(format!("{error:#}")));
            }
        });
        let demo = self.clone();
        let expire = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                let expired = {
                    let mut sessions = demo
                        .sessions
                        .lock()
                        .expect("session registry lock poisoned");
                    let ids: Vec<_> = sessions
                        .iter()
                        .filter(|(_, session)| session.expired())
                        .map(|(id, _)| id.clone())
                        .collect();
                    ids.into_iter()
                        .filter_map(|id| sessions.remove(&id))
                        .collect::<Vec<_>>()
                };
                let mut retiring = demo.retiring.lock().expect("retiring task lock poisoned");
                retiring.retain(|task| !task.is_finished());
                for session in expired {
                    retiring.push(tokio::spawn(async move {
                        session.stop().await;
                    }));
                }
            }
        });
        self.tasks
            .lock()
            .expect("demo task lock poisoned")
            .extend([initialize, expire]);
    }

    pub fn session(&self, id: Option<&str>) -> std::result::Result<(Session, bool), SessionError> {
        let mut sessions = self
            .sessions
            .lock()
            .expect("session registry lock poisoned");
        if let Some(session) = id.and_then(|id| sessions.get(id)).filter(|s| !s.expired()) {
            return Ok((session.clone(), false));
        }
        if sessions.len() >= MAX_SESSIONS {
            return Err(SessionError::Capacity);
        }
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes)
            .context("create browser session identifier")
            .map_err(SessionError::Internal)?;
        let id = hex(&bytes);
        let session = Session::new(
            id.clone(),
            Instant::now() + SESSION_LIFETIME,
            self.startup.subscribe(),
        )
        .map_err(SessionError::Internal)?;
        session.start();
        sessions.insert(id, session.clone());
        Ok((session, true))
    }

    pub fn existing_session(&self, id: &str) -> std::result::Result<Session, SessionError> {
        self.sessions
            .lock()
            .expect("session registry lock poisoned")
            .get(id)
            .filter(|session| !session.expired())
            .cloned()
            .ok_or(SessionError::Expired)
    }

    pub async fn stop(&self) {
        self.startup.send_replace(Startup::Stopped);
        let tasks = std::mem::take(&mut *self.tasks.lock().expect("demo task lock poisoned"));
        for task in tasks {
            task.abort();
            let _ = task.await;
        }
        // A startup completing concurrently with stop must not restore Ready.
        self.startup.send_replace(Startup::Stopped);
        let sessions = std::mem::take(
            &mut *self
                .sessions
                .lock()
                .expect("session registry lock poisoned"),
        );
        let mut retiring =
            std::mem::take(&mut *self.retiring.lock().expect("retiring task lock poisoned"));
        retiring.extend(
            sessions
                .into_values()
                .map(|session| tokio::spawn(async move { session.stop().await })),
        );
        // Stop all sessions together. An unavailable node must not turn a
        // process shutdown into sixteen sequential RPC timeout windows.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        for task in &mut retiring {
            if tokio::time::timeout_at(deadline, task).await.is_err() {
                eprintln!("Session cleanup exceeded shutdown grace; disposing the sandbox");
                break;
            }
        }
        for task in retiring {
            if !task.is_finished() {
                task.abort();
                let _ = task.await;
            }
        }
        // Releasing the last shared worker stops the one sandbox process.
        self.startup.send_replace(Startup::Stopped);
    }

    #[cfg(test)]
    pub fn expire_for_test(&self, id: &str) {
        if let Some(session) = self.sessions.lock().unwrap().get(id) {
            session.expire_for_test();
        }
    }

    #[cfg(test)]
    pub fn session_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }

    fn progress(&self, message: &str) {
        eprintln!("{message}");
        self.startup
            .send_replace(Startup::Preparing(message.into()));
    }

    async fn prepare(&self) -> Result<()> {
        let wasm = runtime::prepare_contracts(|message| self.progress(message)).await?;
        self.progress("Starting the shared local NEAR sandbox");
        let sandbox = Sandbox::start(&wasm).await?;
        let work = Arc::new(Semaphore::new(1));
        let mut fingerprints = Vec::with_capacity(2);
        for index in 0..2 {
            self.progress(&format!(
                "Analyzing test fingerprint {} once for all sessions",
                index + 1
            ));
            let points = self.config.points_per_combination;
            let permit = work.clone().acquire_owned().await?;
            let template = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                FingerprintTemplate::prepare(index, points)
            })
            .await??;
            fingerprints.push(Arc::new(template));
        }
        let fingerprints = fingerprints
            .try_into()
            .map_err(|_| anyhow::anyhow!("expected two fingerprint templates"))?;
        self.startup
            .send_replace(Startup::Ready(Arc::new(Resources {
                sandbox,
                fingerprints,
                work,
            })));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn browser_identity_resumes_only_its_own_state_and_expires_absolutely() -> Result<()> {
        let demo = Demo::new(Config::default())?;
        let (a, new) = demo.session(None).ok().unwrap();
        assert!(new);
        let (b, _) = demo.session(None).ok().unwrap();
        assert_ne!(a.id(), b.id());
        assert_ne!(a.snapshot().token, b.snapshot().token);
        assert_eq!(demo.session(Some(a.id())).ok().unwrap().0.id(), a.id());
        assert!(!demo.session(Some(a.id())).ok().unwrap().1);
        demo.expire_for_test(a.id());
        assert!(matches!(
            demo.existing_session(a.id()),
            Err(SessionError::Expired)
        ));
        assert!(demo.existing_session(b.id()).is_ok());
        let (replacement, new) = demo.session(Some(a.id())).ok().unwrap();
        assert!(new);
        assert_ne!(a.id(), replacement.id());
        assert_eq!(SESSION_LIFETIME, Duration::from_secs(3600));
        demo.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn session_capacity_never_evicts_another_visitor() -> Result<()> {
        let demo = Demo::new(Config::default())?;
        let first = demo.session(None).ok().unwrap().0;
        for _ in 1..MAX_SESSIONS {
            demo.session(None).ok().unwrap();
        }
        assert!(matches!(demo.session(None), Err(SessionError::Capacity)));
        assert!(demo.existing_session(first.id()).is_ok());
        demo.stop().await;
        Ok(())
    }
}
