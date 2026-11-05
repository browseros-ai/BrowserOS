use std::convert::Infallible;
use std::fs::Permissions;
use std::io;
use std::net::SocketAddr;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};

use axum::extract::Request;
use axum::response::Response;
use axum::serve::IncomingStream;
use tokio::fs;
use tokio::net::{TcpListener, UnixListener, UnixStream};
use tokio_util::sync::CancellationToken;
use tower::Service;

use crate::config::HostApiConfig;

/// The host-facing sockets, bound: a Unix socket always, plus TCP when `CLAW_HOST_TCP` opts in
/// (design D1).
///
/// It binds and serves; who may call is decided entirely by the make-service it is given. In
/// the app that is browseros-service's `host_service(routes, auth)`, which carries the routes,
/// the constant-time bearer check, the peer-uid check and the connect info. claw-sdk writes no
/// auth and no connect-info layer of its own (decisions 12 and 16).
#[derive(Debug)]
pub(crate) struct HostListener {
    unix: UnixListener,
    socket: PathBuf,
    tcp: Option<TcpListener>,
}

/// Why the host sockets could not be bound. The start fails: a host that configured a token
/// expects the API to be there.
#[derive(Debug, thiserror::Error)]
pub(crate) enum BindError {
    /// A live server answers on the socket path. Neo never takes a socket from a running
    /// process, which may be another Neo on the same state directory.
    #[error("{} is in use by a live listener", .0.display())]
    SocketInUse(PathBuf),
    /// Something other than a socket sits at the path, so it was not left by a previous Neo.
    #[error("{} exists and is not a socket", .0.display())]
    NotASocket(PathBuf),
    /// Creating the directory, binding, or setting the socket's mode failed.
    #[error("{what}: {source}")]
    Io {
        /// The step that failed, with its path or address.
        what: String,
        /// The OS error.
        source: io::Error,
    },
}

impl HostListener {
    /// Binds the sockets, or returns `None` when no token file is configured: then there is no
    /// host API, and `run` still works.
    ///
    /// The socket is mode 0660, so a host in the socket's group (zuzu's shared
    /// `/run/mirror-neo/`) can connect; the service's auth decides the rest.
    pub(crate) async fn bind(config: &HostApiConfig) -> Result<Option<Self>, BindError> {
        if config.token_file.is_none() {
            return Ok(None);
        }
        let socket = &config.socket;
        if let Some(parent) = socket.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(failed(format!("creating {}", parent.display())))?;
        }
        remove_stale_socket(socket).await?;
        let unix =
            UnixListener::bind(socket).map_err(failed(format!("binding {}", socket.display())))?;
        fs::set_permissions(socket, Permissions::from_mode(0o660))
            .await
            .map_err(failed(format!("setting the mode of {}", socket.display())))?;
        let tcp = match config.tcp {
            Some(address) => Some(
                TcpListener::bind(address)
                    .await
                    .map_err(failed(format!("binding CLAW_HOST_TCP {address}")))?,
            ),
            None => None,
        };
        Ok(Some(Self {
            unix,
            socket: socket.clone(),
            tcp,
        }))
    }

    /// The bound TCP address, when `CLAW_HOST_TCP` opted in.
    pub(crate) fn tcp_addr(&self) -> io::Result<Option<SocketAddr>> {
        self.tcp.as_ref().map(TcpListener::local_addr).transpose()
    }

    /// Serves `make_service` on every bound socket until `shutdown`, then removes the socket
    /// file. The bounds are exactly what `axum::serve` needs for both listener kinds, so
    /// browseros-service's `host_service(routes, auth)` and a plain `Router` both fit.
    pub(crate) async fn serve<M, S>(
        self,
        make_service: M,
        shutdown: CancellationToken,
    ) -> io::Result<()>
    where
        M: for<'a> Service<IncomingStream<'a, UnixListener>, Error = Infallible, Response = S>
            + for<'a> Service<IncomingStream<'a, TcpListener>, Error = Infallible, Response = S>
            + Clone
            + Send
            + 'static,
        for<'a> <M as Service<IncomingStream<'a, UnixListener>>>::Future: Send,
        for<'a> <M as Service<IncomingStream<'a, TcpListener>>>::Future: Send,
        S: Service<Request, Response = Response, Error = Infallible> + Clone + Send + 'static,
        S::Future: Send,
    {
        let Self { unix, socket, tcp } = self;
        let unix_served = axum::serve(unix, make_service.clone())
            .with_graceful_shutdown(shutdown.clone().cancelled_owned())
            .into_future();
        let tcp_served = async move {
            match tcp {
                Some(tcp) => {
                    axum::serve(tcp, make_service)
                        .with_graceful_shutdown(shutdown.cancelled_owned())
                        .await
                }
                None => Ok(()),
            }
        };
        let served = tokio::try_join!(unix_served, tcp_served);
        let removed = match fs::remove_file(&socket).await {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        };
        served?;
        removed
    }
}

/// Removes a socket file nobody accepts on. Chromium stops Neo with SIGKILL, which skips every
/// cleanup, so a file left at the path is the normal case. It is removed only after a connect to
/// it is refused, which proves no process listens; a live listener or a non-socket file fails
/// the bind instead of being deleted.
async fn remove_stale_socket(path: &Path) -> Result<(), BindError> {
    let metadata = match fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(failed(format!("inspecting {}", path.display()))(error)),
    };
    if !metadata.file_type().is_socket() {
        return Err(BindError::NotASocket(path.to_path_buf()));
    }
    match UnixStream::connect(path).await {
        Ok(_live) => Err(BindError::SocketInUse(path.to_path_buf())),
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => fs::remove_file(path)
            .await
            .map_err(failed(format!("removing the stale {}", path.display()))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(failed(format!("probing {}", path.display()))(error)),
    }
}

fn failed(what: String) -> impl FnOnce(io::Error) -> BindError {
    move |source| BindError::Io { what, source }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::net::SocketAddr;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener as StdUnixListener;
    use std::path::Path;
    use std::{fs, io};

    use axum::Router;
    use axum::routing::get;
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
    use tokio::net::{TcpStream, UnixListener, UnixStream};
    use tokio::task::JoinHandle;
    use tokio_util::sync::CancellationToken;

    use super::{BindError, HostListener};
    use crate::config::HostApiConfig;

    type TestResult = Result<(), Box<dyn Error>>;

    fn config(dir: &Path, tcp: Option<SocketAddr>) -> HostApiConfig {
        HostApiConfig {
            socket: dir.join("host.sock"),
            token_file: Some(dir.join("token")),
            tcp,
        }
    }

    /// Stands in for browseros-service's `host_service`: any axum make-service must fit.
    fn routes() -> Router {
        Router::new().route("/ping", get(|| async { "pong" }))
    }

    async fn bound(config: &HostApiConfig) -> Result<HostListener, Box<dyn Error>> {
        Ok(HostListener::bind(config)
            .await?
            .ok_or("a token file is configured, so there is a listener")?)
    }

    fn serve(listener: HostListener) -> (CancellationToken, JoinHandle<io::Result<()>>) {
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(listener.serve(routes(), shutdown.clone()));
        (shutdown, task)
    }

    async fn ping<T: AsyncRead + AsyncWrite + Unpin>(mut stream: T) -> io::Result<String> {
        stream
            .write_all(b"GET /ping HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await?;
        let mut response = String::new();
        stream.read_to_string(&mut response).await?;
        Ok(response)
    }

    fn assert_pong(response: &str) {
        assert!(
            response.starts_with("HTTP/1.1 200") && response.ends_with("pong"),
            "the make-service's own response passes through unchanged: {response}"
        );
    }

    #[tokio::test]
    async fn no_token_file_means_no_listener() -> TestResult {
        let dir = tempfile::tempdir()?;
        let config = HostApiConfig {
            token_file: None,
            ..config(dir.path(), None)
        };
        assert!(
            HostListener::bind(&config).await?.is_none(),
            "without a token there is no host API to protect, so nothing listens"
        );
        assert!(!config.socket.exists(), "and no socket file is created");
        Ok(())
    }

    #[tokio::test]
    async fn serves_the_socket_at_mode_0660_without_tcp() -> TestResult {
        let dir = tempfile::tempdir()?;
        let config = config(dir.path(), None);
        let listener = bound(&config).await?;
        assert_eq!(listener.tcp_addr()?, None, "TCP is opt-in only");
        let mode = fs::metadata(&config.socket)?.permissions().mode() & 0o777;
        assert_eq!(mode, 0o660, "the host's group connects; others cannot");

        let (shutdown, task) = serve(listener);
        assert_pong(&ping(UnixStream::connect(&config.socket).await?).await?);
        shutdown.cancel();
        task.await??;
        assert!(
            !config.socket.exists(),
            "a clean stop removes the socket file"
        );
        Ok(())
    }

    #[tokio::test]
    async fn replaces_a_stale_socket_file() -> TestResult {
        let dir = tempfile::tempdir()?;
        let config = config(dir.path(), None);
        drop(StdUnixListener::bind(&config.socket)?);
        assert!(
            config.socket.exists(),
            "a closed listener leaves its file, as a SIGKILLed Neo does"
        );

        let (shutdown, task) = serve(bound(&config).await?);
        assert_pong(&ping(UnixStream::connect(&config.socket).await?).await?);
        shutdown.cancel();
        task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn never_takes_a_live_socket() -> TestResult {
        let dir = tempfile::tempdir()?;
        let config = config(dir.path(), None);
        let live = UnixListener::bind(&config.socket)?;

        let refused = HostListener::bind(&config).await;
        assert!(
            matches!(refused, Err(BindError::SocketInUse(_))),
            "another process listens there: {refused:?}"
        );
        let still_reachable = UnixStream::connect(&config.socket).await;
        assert!(
            still_reachable.is_ok(),
            "the live listener keeps its socket file"
        );
        drop(live);
        Ok(())
    }

    #[tokio::test]
    async fn never_deletes_a_file_that_is_not_a_socket() -> TestResult {
        let dir = tempfile::tempdir()?;
        let config = config(dir.path(), None);
        fs::write(&config.socket, "not a socket")?;

        let refused = HostListener::bind(&config).await;
        assert!(
            matches!(refused, Err(BindError::NotASocket(_))),
            "{refused:?}"
        );
        assert_eq!(
            fs::read_to_string(&config.socket)?,
            "not a socket",
            "a file Neo did not create is left alone"
        );
        Ok(())
    }

    #[tokio::test]
    async fn serves_tcp_too_when_opted_in() -> TestResult {
        let dir = tempfile::tempdir()?;
        let config = config(dir.path(), Some("127.0.0.1:0".parse()?));
        let listener = bound(&config).await?;
        let address = listener.tcp_addr()?.ok_or("CLAW_HOST_TCP was set")?;

        let (shutdown, task) = serve(listener);
        assert_pong(&ping(TcpStream::connect(address).await?).await?);
        assert_pong(&ping(UnixStream::connect(&config.socket).await?).await?);
        shutdown.cancel();
        task.await??;
        Ok(())
    }
}
