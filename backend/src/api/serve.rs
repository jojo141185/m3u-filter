use axum::body::Body;
use axum::extract::Request;
use axum::response::Response;
use futures::FutureExt;
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use log::{debug, error, trace};
use socket2::{SockRef, TcpKeepalive};
use std::convert::Infallible;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tower::{Service, ServiceExt};
use crate::api::model::ActiveUserManager;

#[derive(Debug)]
struct IncomingStream
{
    remote_addr: SocketAddr,
}

impl IncomingStream {
    /// Returns the remote address that this stream is bound to.
    pub fn remote_addr(&self) -> &SocketAddr {
        &self.remote_addr
    }
}

impl axum::extract::connect_info::Connected<IncomingStream> for SocketAddr {
    fn connect_info(target: IncomingStream) -> SocketAddr {
        *target.remote_addr()
    }
}

pub async fn serve(listener: tokio::net::TcpListener,
                   router: axum::Router<()>,
                   cancel_token: Option<CancellationToken>,
                   user_manager: Arc<ActiveUserManager>) {
    let (signal_tx, _signal_rx) = watch::channel(());
    let mut make_service = router.into_make_service_with_connect_info::<SocketAddr>();

    match cancel_token {
        Some(token) => {
            loop {
                tokio::select! {
                    () = token.cancelled() => {
                        break;
                    }
                    accept_result = listener.accept() => {
                        let Ok((socket, remote_addr)) = accept_result else { continue };
                        handle_connection(&mut make_service, &signal_tx, socket, remote_addr, Arc::clone(&user_manager)).await;
                    }
                }
            }
        }
        None => {
            loop {
                let Ok((socket, remote_addr)) = listener.accept().await else { continue };
                handle_connection(&mut make_service, &signal_tx, socket, remote_addr, Arc::clone(&user_manager)).await;
            }
        }
    }
}

async fn handle_connection<M, S>(
    make_service: &mut M,
    signal_tx: &watch::Sender<()>,
    socket: tokio::net::TcpStream,
    remote_addr: SocketAddr,
    user_manager: Arc<ActiveUserManager>,
)
where
    M: for<'a> Service<IncomingStream, Error=Infallible, Response=S> + Send + 'static,
    for<'a> <M as Service<IncomingStream>>::Future: Send,
    S: Service<Request, Response=Response, Error=Infallible> + Clone + Send + 'static,
    S::Future: Send,
{
    let Ok(tcp_stream_std) = socket.into_std() else { return; };
    //tcp_stream_std.set_nonblocking(true).ok(); // this is not necessary

    // Configure keep alive with socket2
    let sock_ref = SockRef::from(&tcp_stream_std);

    let keep_alive_first_probe = 10;
    let keep_alive_interval = 5;

    let mut keepalive = TcpKeepalive::new();
    keepalive = keepalive.with_time(Duration::from_secs(keep_alive_first_probe)) // Time until the first keepalive probe (idle time)
        .with_interval(Duration::from_secs(keep_alive_interval)); // Interval between keep alives
    #[cfg(not(target_os = "windows"))]
    {
        let keep_alive_retries = 3;
        keepalive = keepalive.with_retries(keep_alive_retries); // Number of failed probes before the connection is closed
    }

    if let Err(e) = sock_ref.set_tcp_keepalive(&keepalive) {
        error!("Failed to set keepalive for {remote_addr}: {e}");
    }

    let Ok(socket) = tokio::net::TcpStream::from_std(tcp_stream_std) else { return; };

    let io = TokioIo::new(socket);
    trace!("connection {remote_addr:?} accepted");

    make_service
        .ready()
        .await
        .unwrap_or_else(|err| match err {});

    let tower_service = make_service
        .call(IncomingStream {
            // io: &io,
            remote_addr,
        })
        .await
        .unwrap_or_else(|err| match err {})
        .map_request(|req: Request<Incoming>| req.map(Body::new));

    let hyper_service = TowerToHyperService::new(tower_service);
    let signal_tx = signal_tx.clone();
    let addr_str = remote_addr.to_string();

    tokio::spawn(async move {
        #[allow(unused_mut)]
        let mut builder = Builder::new(TokioExecutor::new());
        let mut conn = pin!(builder.serve_connection_with_upgrades(io, hyper_service));
        let mut signal_closed = pin!(signal_tx.closed().fuse());

        let user_manager_clone = Arc::clone(&user_manager);
        let mut addr_close_rx = user_manager_clone.get_close_connection_channel();
        let connection_release = user_manager.release_sender();

        debug!("Connection opened: {addr_str}");

        loop {
            tokio::select! {
                result = conn.as_mut() => {
                    if let Err(err) = result {
                        trace!("failed to serve connection: {err:#}");
                    }
                    if let Err(_err) = connection_release.send(remote_addr.to_string()) {
                        let addr = remote_addr.to_string();
                        user_manager_clone.remove_connection(&addr).await;
                    }
                    break;
                }
                () = &mut signal_closed => {
                    let addr = remote_addr.to_string();
                    if let Err(_err) = connection_release.send(addr.clone()) {
                         user_manager_clone.remove_connection(&addr).await;
                    }
                    debug!("Connection gracefully closed: {remote_addr}");
                    conn.as_mut().graceful_shutdown();
                }
                Ok(msg) = addr_close_rx.recv() => {
                    if msg == addr_str {
                        debug!("Forced client disconnect {msg}");
                        conn.as_mut().graceful_shutdown();
                        break;
                    }
                }
            }
        }
    });
}
