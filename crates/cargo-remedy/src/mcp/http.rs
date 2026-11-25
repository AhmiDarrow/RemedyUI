//! Streamable HTTP on loopback, for clients that do not start the server themselves.
//!
//! The server listens on 127.0.0.1 only, accepts only loopback `Host` headers (against DNS
//! rebinding), and answers only requests carrying `Authorization: Bearer <token>`, compared in
//! constant time. Each client session gets its own app and devtools connections.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio::net::TcpListener;

use crate::mcp::{Server, ToolError};

/// The path the server answers on.
pub const PATH: &str = "/mcp";

/// A listening HTTP server.
#[derive(Debug)]
pub struct HttpServer {
    listener: TcpListener,
    addr: SocketAddr,
    token: String,
}

/// Whether `req` carries `token` as its bearer token.
fn authorized<B>(req: &Request<B>, token: &str) -> bool {
    req.headers()
        .get(hyper::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|given| remedy_ui_core::token::same_secret(given.trim(), token))
}

fn plain(
    status: StatusCode,
    text: &'static str,
) -> Response<http_body_util::combinators::BoxBody<Bytes, Infallible>> {
    let mut r = Response::new(Full::new(Bytes::from_static(text.as_bytes())).boxed());
    *r.status_mut() = status;
    if status == StatusCode::UNAUTHORIZED {
        r.headers_mut()
            .insert(hyper::header::WWW_AUTHENTICATE, hyper::header::HeaderValue::from_static("Bearer"));
    }
    r
}

impl HttpServer {
    /// Listens on loopback `port` (0 picks one). `token` is what clients must send; it must
    /// not be empty.
    pub async fn bind(port: u16, token: String) -> Result<HttpServer, ToolError> {
        if token.trim().is_empty() {
            return Err(ToolError::new("the HTTP transport needs a token"));
        }
        let listener = TcpListener::bind(("127.0.0.1", port))
            .await
            .map_err(|e| ToolError::new(format!("could not listen on 127.0.0.1:{port}: {e}")))?;
        let addr = listener.local_addr().map_err(|e| ToolError::new(e.to_string()))?;
        Ok(HttpServer { listener, addr, token })
    }

    /// Where it listens.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The URL clients use.
    pub fn url(&self) -> String {
        format!("http://{}{PATH}", self.addr)
    }

    /// Serves `server` until the process ends.
    pub async fn serve(self, server: Server) -> Result<(), ToolError> {
        let service = StreamableHttpService::new(
            move || Ok(server.fresh_session()),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        );
        let token: Arc<str> = Arc::from(self.token.as_str());
        loop {
            // One failed accept (a client gone before it was taken, no free handles) is not the
            // server's end; the pause keeps a lasting failure from spinning.
            let stream = match self.listener.accept().await {
                Ok((stream, _)) => stream,
                Err(e) => {
                    eprintln!("remedy-mcp: accepting a connection failed: {e}");
                    let _ = tokio::task::spawn_blocking(|| {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    })
                    .await;
                    continue;
                }
            };
            let (service, token) = (service.clone(), token.clone());
            tokio::spawn(async move {
                let handler = service_fn(move |req: Request<Incoming>| {
                    let (mut service, token) = (service.clone(), token.clone());
                    async move {
                        if !authorized(&req, &token) {
                            return Ok::<_, Infallible>(plain(
                                StatusCode::UNAUTHORIZED,
                                "a bearer token is required",
                            ));
                        }
                        if req.uri().path() != PATH {
                            return Ok(plain(StatusCode::NOT_FOUND, "the MCP endpoint is /mcp"));
                        }
                        tower_service::Service::call(&mut service, req).await
                    }
                });
                let _ = http1::Builder::new().serve_connection(TokioIo::new(stream), handler).await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_right_bearer_token_is_authorized() {
        let req = |h: Option<&str>| {
            let mut b = Request::builder().uri("/mcp");
            if let Some(h) = h {
                b = b.header("authorization", h);
            }
            b.body(()).unwrap_or_else(|e| unreachable!("{e}"))
        };
        assert!(authorized(&req(Some("Bearer s3cret")), "s3cret"));
        assert!(!authorized(&req(Some("Bearer wrong")), "s3cret"));
        assert!(!authorized(&req(Some("s3cret")), "s3cret"));
        assert!(!authorized(&req(None), "s3cret"));
    }
}
