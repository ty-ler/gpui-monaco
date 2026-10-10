//! JSON-RPC 2.0 over the LSP base protocol: `Content-Length` framed
//! messages on a byte stream (usually the server's stdin and stdout).

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, anyhow};
use futures_lite::io::{
    AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use gpui::{BackgroundExecutor, Task};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// LSP's `RequestCancelled` error code.
pub const REQUEST_CANCELLED: i64 = -32800;
/// JSON-RPC's `MethodNotFound` error code.
pub const METHOD_NOT_FOUND: i64 = -32601;

/// An error response (JSON-RPC `ResponseError`).
#[derive(Clone, Debug, PartialEq)]
pub struct ResponseError {
    /// The error code.
    pub code: i64,
    /// The message.
    pub message: String,
}

impl std::fmt::Display for ResponseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "language server error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for ResponseError {}

/// A message from the server that isn't a response.
#[derive(Clone, Debug)]
pub enum Incoming {
    /// A notification.
    Notification {
        /// Its method.
        method: String,
        /// Its params (`null` if absent).
        params: Value,
    },
    /// A request the client must answer with [`Connection::respond`].
    Request {
        /// Its id, echoed in the response.
        id: Value,
        /// Its method.
        method: String,
        /// Its params (`null` if absent).
        params: Value,
    },
}

type ResponseSender = async_channel::Sender<Result<Value, ResponseError>>;

struct Inner {
    outgoing: async_channel::Sender<Vec<u8>>,
    pending: Mutex<HashMap<i64, ResponseSender>>,
    next_id: AtomicI64,
}

/// A connection to a language server. Cheap to clone; every clone sends
/// on the same stream.
#[derive(Clone)]
pub struct Connection {
    inner: Arc<Inner>,
}

impl Connection {
    /// Connects over `reader` (the server's output) and `writer` (its
    /// input). Returns the connection, the server's notifications and
    /// requests, and the I/O tasks, which stop when dropped.
    pub fn new<R, W>(
        reader: R,
        writer: W,
        executor: &BackgroundExecutor,
    ) -> (Self, async_channel::Receiver<Incoming>, Vec<Task<()>>)
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (outgoing, outgoing_rx) = async_channel::unbounded::<Vec<u8>>();
        let (incoming_tx, incoming) = async_channel::unbounded();
        let inner = Arc::new(Inner {
            outgoing,
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicI64::new(0),
        });
        let write = executor.spawn(async move {
            let mut writer = writer;
            while let Ok(message) = outgoing_rx.recv().await {
                let header = format!("Content-Length: {}\r\n\r\n", message.len());
                let written = async {
                    writer.write_all(header.as_bytes()).await?;
                    writer.write_all(&message).await?;
                    writer.flush().await
                };
                if let Err(err) = written.await {
                    log::warn!("language server write failed: {err}");
                    break;
                }
            }
        });
        let read_inner = inner.clone();
        let read = executor.spawn(async move {
            let mut reader = BufReader::new(reader);
            loop {
                let message = match read_message(&mut reader).await {
                    Ok(Some(message)) => message,
                    Ok(None) => break,
                    Err(err) => {
                        log::warn!("language server read failed: {err:#}");
                        break;
                    }
                };
                if let Some(incoming) = route(&read_inner, message)
                    && incoming_tx.send(incoming).await.is_err()
                {
                    break;
                }
            }
            // The server is gone: fail what's still waiting.
            let pending = std::mem::take(&mut *read_inner.pending.lock().unwrap());
            for (_, sender) in pending {
                sender
                    .try_send(Err(ResponseError {
                        code: REQUEST_CANCELLED,
                        message: "the language server exited".into(),
                    }))
                    .ok();
            }
        });
        (Self { inner }, incoming, vec![write, read])
    }

    /// Sends request `method`. Dropping the returned future before it
    /// completes cancels the request (`$/cancelRequest`).
    pub fn request_raw(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<Value>> + Send + 'static {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (sender, receiver) = async_channel::bounded(1);
        self.inner.pending.lock().unwrap().insert(id, sender);
        let sent = self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }));
        let guard = CancelOnDrop {
            id,
            inner: Some(self.inner.clone()),
        };
        let method = method.to_string();
        async move {
            let mut guard = guard;
            sent?;
            let response = receiver.recv().await;
            guard.inner = None;
            match response {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(error)) => Err(anyhow::Error::new(error).context(method)),
                Err(_) => Err(anyhow!("{method}: the language server exited")),
            }
        }
    }

    /// Sends a typed request (see [`Connection::request_raw`]).
    pub fn request<R: lsp_types::request::Request>(
        &self,
        params: R::Params,
    ) -> impl Future<Output = Result<R::Result>> + Send + 'static
    where
        R::Result: Send,
    {
        let params = serde_json::to_value(params);
        let request = params.map(|params| self.request_raw(R::METHOD, params));
        async move {
            let value = request?.await?;
            serde_json::from_value(value).with_context(|| format!("invalid {} result", R::METHOD))
        }
    }

    /// Sends a notification.
    pub fn notify_raw(&self, method: &str, params: Value) -> Result<()> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    /// Sends a typed notification.
    pub fn notify<N: lsp_types::notification::Notification>(
        &self,
        params: N::Params,
    ) -> Result<()> {
        self.notify_raw(N::METHOD, serde_json::to_value(params)?)
    }

    /// Answers a server request.
    pub fn respond(&self, id: Value, result: Result<Value, ResponseError>) -> Result<()> {
        let message = match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": error.code, "message": error.message },
            }),
        };
        self.send(&message)
    }

    fn send(&self, message: &impl Serialize) -> Result<()> {
        let bytes = serde_json::to_vec(message)?;
        self.inner
            .outgoing
            .try_send(bytes)
            .map_err(|_| anyhow!("the language server connection is closed"))
    }
}

/// Cancels a request whose future was dropped before the response came.
struct CancelOnDrop {
    id: i64,
    inner: Option<Arc<Inner>>,
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        let Some(inner) = self.inner.take() else {
            return;
        };
        if inner.pending.lock().unwrap().remove(&self.id).is_some() {
            Connection { inner }
                .notify_raw("$/cancelRequest", json!({ "id": self.id }))
                .ok();
        }
    }
}

/// Resolves responses; returns everything else.
fn route(inner: &Inner, message: Value) -> Option<Incoming> {
    let Value::Object(mut message) = message else {
        log::warn!("language server sent a non-object message");
        return None;
    };
    let params = message.remove("params").unwrap_or(Value::Null);
    match (message.remove("id"), message.remove("method")) {
        (Some(id), Some(Value::String(method))) => Some(Incoming::Request { id, method, params }),
        (None, Some(Value::String(method))) => Some(Incoming::Notification { method, params }),
        (Some(id), None) => {
            let id = id.as_i64()?;
            let sender = inner.pending.lock().unwrap().remove(&id)?;
            let result = match message.remove("error") {
                Some(error) => Err(ResponseError {
                    code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                    message: error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                }),
                None => Ok(message.remove("result").unwrap_or(Value::Null)),
            };
            sender.try_send(result).ok();
            None
        }
        _ => {
            log::warn!("language server sent an invalid message");
            None
        }
    }
}

/// Reads one framed message, or `None` at the end of the stream.
pub(crate) async fn read_message<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> Result<Option<Value>> {
    let mut length = None;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(None);
        }
        let header = line.trim_end();
        if header.is_empty() {
            if length.is_some() {
                break;
            }
            continue;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .context("bad Content-Length")?,
            );
        }
    }
    let mut body = vec![0; length.unwrap_or(0)];
    reader.read_exact(&mut body).await?;
    Ok(Some(serde_json::from_slice(&body)?))
}

/// Parses a typed value out of `params`.
pub(crate) fn params<T: DeserializeOwned>(params: Value) -> Result<T> {
    Ok(serde_json::from_value(params)?)
}
