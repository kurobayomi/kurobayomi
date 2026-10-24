//! TorrServer process supervisor: starts a TorrServer-compatible binary as a child, waits for `/echo`, and
//! stops it. It replaces the Android library and foreground service (see `plans/13-*/p1-topic-torrserver-lifecycle.md`).
//!
//! `ensure_running` is single-flight: the whole start runs under one async mutex, so a concurrent caller waits
//! for it and then sees `Running`. The configuration is read from a provider at each start, so edited
//! trackers and proxy settings apply on the next start (deviation D-12).

use super::TorrentError;
use super::client::TorrServerClient;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

/// Readiness polls after a start (`wait(10)` is 11 polls).
pub const READY_POLLS: u32 = 11;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerStatus {
    Stopped,
    Starting,
    Running { port: u16 },
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    /// The server binary (the app resolves the user setting, the search path or the bundled copy).
    pub program: PathBuf,
    /// Arguments placed before the ones the supervisor adds.
    pub base_args: Vec<String>,
    /// Passed as `--path`.
    pub data_dir: PathBuf,
    /// The configured port. `0` means any free port: the supervisor picks one and uses it, never `0`.
    pub port: u16,
    /// `PROXY_MODE`; `None` sends no proxy mode (deviation D-13).
    pub proxy_mode: Option<String>,
    pub proxy_url: String,
    /// The tracker list preference, one per line.
    pub trackers: String,
    pub poll_interval: Duration,
    pub stop_grace: Duration,
}

impl SupervisorConfig {
    pub fn new(program: impl Into<PathBuf>, data_dir: impl Into<PathBuf>, port: u16) -> Self {
        Self {
            program: program.into(),
            base_args: Vec::new(),
            data_dir: data_dir.into(),
            port,
            proxy_mode: None,
            proxy_url: String::new(),
            trackers: String::new(),
            poll_interval: Duration::from_secs(1),
            stop_grace: Duration::from_secs(5),
        }
    }
}

type ConfigFn = dyn Fn() -> SupervisorConfig + Send + Sync;

struct Inner {
    child: Option<Child>,
    port: u16,
    stop_grace: Duration,
}

pub struct TorrServerSupervisor {
    config: Arc<ConfigFn>,
    http: reqwest::Client,
    inner: tokio::sync::Mutex<Inner>,
    status: Mutex<ServerStatus>,
}

fn free_port() -> Result<u16, TorrentError> {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| TorrentError::Failed(format!("no free port: {e}")))
}

fn pipe_to_log(stream: impl tokio::io::AsyncRead + Unpin + Send + 'static) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(target: "torrserver", "{line}");
        }
    });
}

impl TorrServerSupervisor {
    pub fn new(config: impl Fn() -> SupervisorConfig + Send + Sync + 'static, http: reqwest::Client) -> Self {
        Self {
            config: Arc::new(config),
            http,
            inner: tokio::sync::Mutex::new(Inner { child: None, port: 0, stop_grace: Duration::from_secs(5) }),
            status: Mutex::new(ServerStatus::Stopped),
        }
    }

    pub fn status(&self) -> ServerStatus {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set(&self, s: ServerStatus) {
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = s;
    }

    fn fail(&self, reason: String, err: TorrentError) -> TorrentError {
        tracing::warn!(target: "torrserver", "{reason}");
        self.set(ServerStatus::Failed(reason));
        err
    }

    /// A client for the running server, with the current tracker list. `None` unless running.
    pub fn client(&self, trackers: &str) -> Option<TorrServerClient> {
        match self.status() {
            ServerStatus::Running { port } => Some(TorrServerClient::new(port, self.http.clone()).with_trackers(trackers)),
            _ => None,
        }
    }

    /// Starts the server unless it already answers, and returns its port.
    pub async fn ensure_running(&self) -> Result<u16, TorrentError> {
        let mut inner = self.inner.lock().await;
        let cfg = (self.config)();
        inner.stop_grace = cfg.stop_grace;

        if let Some(child) = inner.child.as_mut()
            && let Ok(Some(code)) = child.try_wait()
        {
            inner.child = None;
            self.set(ServerStatus::Failed(format!("exited with {code}")));
        }
        if matches!(self.status(), ServerStatus::Running { .. }) && inner.port != 0 {
            return Ok(inner.port);
        }

        let port = if cfg.port == 0 { free_port()? } else { cfg.port };
        if TorrServerClient::new(port, self.http.clone()).echo().await {
            // already answering: use it without spawning
            inner.port = port;
            self.set(ServerStatus::Running { port });
            return Ok(port);
        }

        self.set(ServerStatus::Starting);
        let mut cmd = Command::new(&cfg.program);
        kuro_platform::process::no_console(cmd.as_std_mut());
        cmd.args(&cfg.base_args)
            .arg("--port")
            .arg(port.to_string())
            .arg("--path")
            .arg(&cfg.data_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(mode) = &cfg.proxy_mode {
            cmd.env("PROXY_MODE", mode).env("PROXY_URL", &cfg.proxy_url);
        }
        let mut child = cmd.spawn().map_err(|e| {
            self.fail(format!("cannot start {}: {e}", cfg.program.display()), TorrentError::Failed(format!("cannot start the server: {e}")))
        })?;
        if let Some(out) = child.stdout.take() {
            pipe_to_log(out);
        }
        if let Some(err) = child.stderr.take() {
            pipe_to_log(err);
        }

        let probe = TorrServerClient::new(port, self.http.clone());
        for _ in 0..READY_POLLS {
            tokio::time::sleep(cfg.poll_interval).await;
            if let Ok(Some(code)) = child.try_wait() {
                return Err(self.fail(format!("exited with {code}"), TorrentError::Failed(format!("the server exited with {code}"))));
            }
            if probe.echo().await {
                inner.child = Some(child);
                inner.port = port;
                self.set(ServerStatus::Running { port });
                return Ok(port);
            }
        }
        let _ = child.kill().await;
        Err(self.fail(format!("did not respond within {READY_POLLS} s"), TorrentError::ServerTimeout))
    }

    /// Kills the child (waiting at most the grace period) and goes to `Stopped`. A stopped supervisor is a no-op.
    pub async fn stop(&self) -> Result<(), TorrentError> {
        let mut inner = self.inner.lock().await;
        if let Some(mut child) = inner.child.take() {
            let grace = inner.stop_grace;
            if tokio::time::timeout(grace, child.kill()).await.is_err() {
                return Err(TorrentError::Failed("the server did not stop in time".into()));
            }
        }
        inner.port = 0;
        self.set(ServerStatus::Stopped);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[cfg(unix)]
    fn config(port: u16, script: &str) -> SupervisorConfig {
        let mut c = SupervisorConfig::new("sh", std::env::temp_dir(), port);
        // the supervisor's own `--port N --path D` arguments land in $1.. and are ignored
        c.base_args = vec!["-c".into(), script.into(), "sh".into()];
        c.poll_interval = Duration::from_millis(20);
        c
    }

    fn supervisor(c: SupervisorConfig) -> TorrServerSupervisor {
        TorrServerSupervisor::new(move || c.clone(), reqwest::Client::new())
    }

    async fn echo_server(up: bool) -> MockServer {
        let s = MockServer::start().await;
        if up {
            Mock::given(path("/echo")).respond_with(ResponseTemplate::new(200).set_body_string("ok")).mount(&s).await;
        }
        s
    }

    #[cfg(unix)]
    fn alive(pid: &str) -> bool {
        std::path::Path::new(&format!("/proc/{}", pid.trim())).exists()
    }

    #[tokio::test]
    async fn an_already_answering_server_is_used_without_spawning() {
        let s = echo_server(true).await;
        let port = s.address().port();
        let sup = supervisor(SupervisorConfig::new("/nonexistent/torrserver", std::env::temp_dir(), port));
        assert_eq!(sup.ensure_running().await.unwrap(), port);
        assert_eq!(sup.status(), ServerStatus::Running { port });
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn concurrent_calls_start_one_child_and_share_the_port() {
        let s = echo_server(false).await;
        let port = s.address().port();
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("starts");
        let sup = Arc::new(supervisor(config(port, &format!("echo x >> {}; exec sleep 30", log.display()))));
        let calls: Vec<_> = (0..20)
            .map(|_| {
                let sup = sup.clone();
                tokio::spawn(async move { sup.ensure_running().await })
            })
            .collect();
        tokio::time::sleep(Duration::from_millis(150)).await;
        Mock::given(path("/echo")).respond_with(ResponseTemplate::new(200).set_body_string("ok")).mount(&s).await;
        for c in calls {
            assert_eq!(c.await.unwrap().unwrap(), port);
        }
        assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 1);
        sup.stop().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_child_that_exits_early_gives_failed() {
        let s = echo_server(false).await;
        let sup = supervisor(config(s.address().port(), "exit 3"));
        let err = sup.ensure_running().await.unwrap_err();
        assert!(matches!(err, TorrentError::Failed(_)), "{err:?}");
        assert!(matches!(sup.status(), ServerStatus::Failed(r) if r.contains("exited")));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn readiness_gives_up_after_11_polls_with_server_timeout() {
        let s = echo_server(false).await;
        let sup = supervisor(config(s.address().port(), "exec sleep 30"));
        assert_eq!(sup.ensure_running().await, Err(TorrentError::ServerTimeout));
        // one pre-start echo plus the 11 polls
        assert_eq!(s.received_requests().await.unwrap().len(), 1 + READY_POLLS as usize);
        assert!(matches!(sup.status(), ServerStatus::Failed(_)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn port_zero_hands_the_child_a_real_port() {
        let dir = tempfile::tempdir().unwrap();
        let arg_file = dir.path().join("port");
        // $1 is "--port" and $2 is its value
        let sup = supervisor(config(0, &format!("echo $2 > {}; exec sleep 30", arg_file.display())));
        assert_eq!(sup.ensure_running().await, Err(TorrentError::ServerTimeout));
        let given: u32 = std::fs::read_to_string(&arg_file).unwrap().trim().parse().unwrap();
        assert!(given > 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stop_kills_the_child_and_a_stopped_supervisor_stops_cleanly() {
        let s = echo_server(false).await;
        let port = s.address().port();
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let sup = Arc::new(supervisor(config(port, &format!("echo $$ > {}; exec sleep 30", pid_file.display()))));
        let starting = {
            let sup = sup.clone();
            tokio::spawn(async move { sup.ensure_running().await })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        Mock::given(path("/echo")).respond_with(ResponseTemplate::new(200)).mount(&s).await;
        starting.await.unwrap().unwrap();
        let pid = std::fs::read_to_string(&pid_file).unwrap();
        assert!(alive(&pid));
        let t = std::time::Instant::now();
        sup.stop().await.unwrap();
        assert!(t.elapsed() < Duration::from_secs(5));
        assert!(!alive(&pid));
        assert_eq!(sup.status(), ServerStatus::Stopped);
        sup.stop().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn proxy_and_trackers_come_from_the_provider_at_each_start() {
        let s = echo_server(false).await;
        let port = s.address().port();
        let dir = tempfile::tempdir().unwrap();
        let env_file = dir.path().join("env");
        let mut c = config(port, &format!("echo \"$PROXY_MODE $PROXY_URL\" > {}; exec sleep 30", env_file.display()));
        c.proxy_mode = Some("full".into());
        c.proxy_url = "socks5://h:1".into();
        let sup = Arc::new(supervisor(c));
        let starting = {
            let sup = sup.clone();
            tokio::spawn(async move { sup.ensure_running().await })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        Mock::given(path("/echo")).respond_with(ResponseTemplate::new(200)).mount(&s).await;
        starting.await.unwrap().unwrap();
        assert_eq!(std::fs::read_to_string(&env_file).unwrap().trim(), "full socks5://h:1");
        assert!(sup.client("udp://t:1").is_some());
        sup.stop().await.unwrap();
        assert!(sup.client("").is_none());
    }
}
