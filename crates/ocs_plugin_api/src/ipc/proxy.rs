//! Request proxy framing for out-of-process plugins that need a worker
//! process (e.g. a Python child) to issue host requests.
//!
//! The proxy is intentionally simple and separate from the V2/V3 local-socket
//! framing. A plugin runner opens a listener, passes the port to its child, and
//! runs [`run_request_proxy`] in a background thread. The child creates a
//! [`ProxyPluginRequestSender`] over a stream and uses it as a
//! [`PluginRequestSender`].
//!
//! This module is additive to the V4 API and does not change any V2/V3 wire
//! formats or enum variants.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use crate::host::{PluginRequestError, PluginRequestSender};
use crate::ipc::protocol::{PluginRequest, PluginResponse};

const MAX_MESSAGE_SIZE: usize = 64 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Length of the pre-framing auth token exchanged when a child connects to
/// the request proxy. Any local process can see the port, so the token (shared
/// only with the spawned child via an environment variable) prevents arbitrary
/// peers from forwarding privileged host requests.
pub const PROXY_TOKEN_LEN: usize = 32;
/// Interval used by [`ProxyPluginRequestSender::request_with_poll`] between
/// response-read attempts so callers can check for signals/interrupts.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Errors that can occur on the proxy stream.
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Encode(#[from] bincode::Error),
    #[error("empty message")]
    Empty,
    #[error("message too large: {0} bytes")]
    TooLarge(usize),
}

impl From<ProxyError> for PluginRequestError {
    fn from(e: ProxyError) -> Self {
        PluginRequestError(e.to_string())
    }
}

/// Send a length-framed bincode message over a generic stream.
pub fn send_framed<W: Write>(writer: &mut W, msg: &impl serde::Serialize) -> Result<(), ProxyError> {
    let bytes = bincode::serialize(msg)?;
    if bytes.len() > MAX_MESSAGE_SIZE {
        return Err(ProxyError::TooLarge(bytes.len()));
    }
    let len = bytes.len() as u64;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

/// Receive a length-framed bincode message from a generic stream.
pub fn recv_framed<R: Read, T: serde::de::DeserializeOwned>(reader: &mut R) -> Result<T, ProxyError> {
    let mut len_buf = [0u8; 8];
    reader.read_exact(&mut len_buf)?;
    let len = u64::from_le_bytes(len_buf) as usize;
    if len == 0 {
        return Err(ProxyError::Empty);
    }
    if len > MAX_MESSAGE_SIZE {
        return Err(ProxyError::TooLarge(len));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    let msg = bincode::deserialize(&buf)?;
    Ok(msg)
}

/// Thread-safe [`PluginRequestSender`] implementation that forwards requests
/// over a single TCP stream to a request proxy server.
pub struct ProxyPluginRequestSender {
    stream: Arc<Mutex<TcpStream>>,
}

impl ProxyPluginRequestSender {
    /// Connect to a request proxy server at `host:port` and authenticate with
    /// `token`. The token must match the value the proxy was started with.
    pub fn connect_with_token(
        host: &str,
        port: u16,
        token: &[u8; PROXY_TOKEN_LEN],
    ) -> Result<Self, ProxyError> {
        let mut stream = TcpStream::connect((host, port))?;
        stream.write_all(token)?;
        stream.flush()?;
        stream.set_read_timeout(Some(REQUEST_TIMEOUT))?;
        Ok(Self {
            stream: Arc::new(Mutex::new(stream)),
        })
    }

    /// Create a second handle that shares the connection and its request lock.
    pub fn try_clone(&self) -> Result<Self, ProxyError> {
        Ok(Self {
            stream: Arc::clone(&self.stream),
        })
    }

    /// Send a request and wait for the response, calling `poll` every
    /// [`POLL_INTERVAL`] while waiting. This lets Python callers run
    /// `py.check_signals()` so that `Ctrl+C`/`KeyboardInterrupt` is raised
    /// promptly instead of waiting for the full socket timeout.
    pub fn request_with_poll(
        &self,
        req: PluginRequest,
        poll: &mut dyn FnMut() -> Result<(), PluginRequestError>,
    ) -> Result<PluginResponse, PluginRequestError> {
        use std::io::ErrorKind;
        let mut stream = self
            .stream
            .lock()
            .map_err(|e| PluginRequestError(e.to_string()))?;
        send_framed(&mut *stream, &req)?;
        let original_timeout = stream
            .read_timeout()
            .map_err(|e| PluginRequestError(e.to_string()))?;
        stream
            .set_read_timeout(Some(POLL_INTERVAL))
            .map_err(|e| PluginRequestError(e.to_string()))?;
        let result = loop {
            match recv_framed(&mut *stream) {
                Ok(resp) => break Ok(resp),
                Err(ProxyError::Io(e))
                    if e.kind() == ErrorKind::WouldBlock
                        || e.kind() == ErrorKind::TimedOut =>
                {
                    if let Err(e) = poll() {
                        let _ = stream.shutdown(Shutdown::Both);
                        break Err(e);
                    }
                }
                Err(e) => break Err(e.into()),
            }
        };
        let _ = stream.set_read_timeout(original_timeout);
        result
    }
}

impl PluginRequestSender for ProxyPluginRequestSender {
    fn request(&self, req: PluginRequest) -> Result<PluginResponse, PluginRequestError> {
        let mut stream = self
            .stream
            .lock()
            .map_err(|e| PluginRequestError(e.to_string()))?;
        send_framed(&mut *stream, &req)?;
        let resp: PluginResponse = recv_framed(&mut *stream)?;
        Ok(resp)
    }
}

/// Read the first [`PROXY_TOKEN_LEN`] bytes from `stream` and verify they
/// match `expected`. Returns `false` if the peer sent the wrong token or
/// disconnected.
fn verify_token(stream: &mut TcpStream, expected: &[u8; PROXY_TOKEN_LEN]) -> bool {
    let mut buf = [0u8; PROXY_TOKEN_LEN];
    match stream.read_exact(&mut buf) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("[plugin] proxy failed to read auth token: {e}");
            return false;
        }
    }
    if buf != *expected {
        eprintln!("[plugin] proxy auth token mismatch; closing connection");
        return false;
    }
    true
}

/// Run a request proxy server on `listener`. For each incoming connection a
/// thread is spawned that reads [`PluginRequest`]s, forwards them through
/// `sender`, and writes back [`PluginResponse`]s. The function returns only
/// when the listener is closed or encounters an error.
///
/// `token` must be sent by the client before the length-framed protocol begins.
pub fn run_request_proxy(
    listener: TcpListener,
    sender: Arc<dyn PluginRequestSender>,
    token: [u8; PROXY_TOKEN_LEN],
) -> std::io::Result<()> {
    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(e) => return Err(e),
        };
        if !verify_token(&mut stream, &token) {
            continue;
        }
        let sender = Arc::clone(&sender);
        std::thread::spawn(move || {
            loop {
                let req: PluginRequest = match recv_framed(&mut stream) {
                    Ok(r) => r,
                    Err(_) => break,
                };
                let resp = match sender.request(req) {
                    Ok(r) => r,
                    Err(e) => PluginResponse::Error(e.to_string()),
                };
                if send_framed(&mut stream, &resp).is_err() {
                    break;
                }
            }
        });
    }
    Ok(())
}

/// Run a request proxy server on `listener` with an explicit shutdown signal.
/// Same behavior as [`run_request_proxy`], but returns when `shutdown` receives
/// a value, closing the listener and terminating the server thread.
pub fn run_request_proxy_with_shutdown(
    listener: TcpListener,
    sender: Arc<dyn PluginRequestSender>,
    token: [u8; PROXY_TOKEN_LEN],
    shutdown: std::sync::mpsc::Receiver<()>,
) -> std::io::Result<()> {
    listener.set_nonblocking(true)?;
    let mut incoming = listener.incoming();
    loop {
        match incoming.next() {
            Some(Ok(mut stream)) => {
                // Accepted streams may inherit the listener's non-blocking mode;
                // force blocking mode so recv_framed/read_exact work.
                let _ = stream.set_nonblocking(false);
                if !verify_token(&mut stream, &token) {
                    continue;
                }
                let sender = Arc::clone(&sender);
                std::thread::spawn(move || {
                    let mut stream = stream;
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        loop {
                            let req: PluginRequest = match recv_framed(&mut stream) {
                                Ok(r) => {
                                    // Log only the variant; large payloads such
                                    // as AddEntities(10000 points) are expensive
                                    // to Debug-format and are not human-readable.
                                    eprintln!(
                                        "[plugin] proxy recv request: {:?}",
                                        std::mem::discriminant(&r)
                                    );
                                    r
                                }
                                Err(e) => {
                                    eprintln!("[plugin] proxy recv error: {e}");
                                    break;
                                }
                            };
                            let resp = match sender.request(req) {
                                Ok(r) => r,
                                Err(e) => PluginResponse::Error(e.to_string()),
                            };
                            eprintln!(
                                "[plugin] proxy forwarding response: {:?}",
                                std::mem::discriminant(&resp)
                            );
                            if let Err(e) = send_framed(&mut stream, &resp) {
                                eprintln!("[plugin] proxy send error: {e}");
                                break;
                            }
                        }
                    }));
                    if let Err(payload) = result {
                        let msg = payload
                            .downcast_ref::<&str>()
                            .map(|s| (*s).to_string())
                            .or_else(|| payload.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "proxy connection thread panicked".to_string());
                        eprintln!("[plugin] request proxy connection panic: {msg}");
                    }
                    eprintln!("[plugin] proxy connection handler exiting");
                });
            }
            Some(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if shutdown.try_recv().is_ok() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Some(Err(e)) => return Err(e),
            None => break,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    struct EchoSender;
    impl PluginRequestSender for EchoSender {
        fn request(&self, req: PluginRequest) -> Result<PluginResponse, PluginRequestError> {
            match req {
                PluginRequest::PushInfo(_) => Ok(PluginResponse::Ok),
                _ => Ok(PluginResponse::Bool(false)),
            }
        }
    }

    #[test]
    fn proxy_round_trips_plugin_request() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let token = [42u8; PROXY_TOKEN_LEN];
        thread::spawn(move || {
            let sender: Arc<dyn PluginRequestSender> = Arc::new(EchoSender);
            run_request_proxy(listener, sender, token).ok();
        });

        let sender = ProxyPluginRequestSender::connect_with_token("127.0.0.1", port, &token).unwrap();
        let resp = sender
            .request(PluginRequest::PushInfo("hello proxy".to_string()))
            .unwrap();
        assert!(matches!(resp, PluginResponse::Ok));
    }
}
