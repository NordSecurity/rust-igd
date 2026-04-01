use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

use http::header::HOST;
use http::Uri;
use hyper::{client::conn::http1::Builder, Request, StatusCode};
use tokio::net::{TcpStream, UdpSocket};
use tokio::time::timeout;

use crate::aio::Gateway;
use crate::common::{messages, parsing, SearchOptions};
use crate::errors::SearchError;
use crate::search::{check_is_ip_spoofed, validate_url};

const MAX_HTTP_RESPONSE_SIZE: usize = 256 * 1024;
const MAX_RESPONSE_SIZE: usize = 1500;

/// Search for a gateway with the provided options
pub async fn search_gateway(options: SearchOptions) -> Result<Gateway, SearchError> {
    // Create socket for future calls
    let mut socket = UdpSocket::bind(&options.bind_addr).await?;

    send_search_request(&mut socket, options.broadcast_address).await?;

    let (response_body, from) = run_with_timeout(options.timeout, receive_search_response(&mut socket)).await??;

    let (addr, root_url) = handle_broadcast_resp(&from, &response_body)?;

    check_is_ip_spoofed(&from, &addr)?;

    let (control_schema_url, control_url) =
        run_with_timeout(options.http_timeout, get_control_urls(&addr, &root_url)).await??;
    let control_schema =
        run_with_timeout(options.http_timeout, get_control_schemas(&addr, &control_schema_url)).await??;

    let addr_v4 = match addr {
        SocketAddr::V4(a) => Ok(a),
        _ => {
            warn!("unsupported IPv6 gateway response from addr: {}", addr);
            Err(SearchError::InvalidResponse)
        }
    }?;

    let gateway = Gateway {
        addr: addr_v4,
        root_url,
        control_url,
        control_schema_url: control_schema_url.to_owned(),
        control_schema,
    };

    let gateway_url = reqwest::Url::from_str(&format!("{gateway}"))?;

    validate_url(addr.ip(), &gateway_url)?;

    Ok(gateway)
}

async fn run_with_timeout<F>(timeout_value: Option<Duration>, fut: F) -> Result<F::Output, SearchError>
where
    F: Future + Send,
{
    match timeout_value {
        Some(t) => Ok(timeout(t, fut).await?),
        None => Ok(fut.await),
    }
}

// Create a new search
async fn send_search_request(socket: &mut UdpSocket, addr: SocketAddr) -> Result<(), SearchError> {
    debug!(
        "sending broadcast request to: {} on interface: {:?}",
        addr,
        socket.local_addr()
    );
    Ok(socket
        .send_to(messages::SEARCH_REQUEST.as_bytes(), &addr)
        .await
        .map(|_| ())?)
}

async fn receive_search_response(socket: &mut UdpSocket) -> Result<(Vec<u8>, SocketAddr), SearchError> {
    let mut buff = [0u8; MAX_RESPONSE_SIZE];
    let (n, from) = socket.recv_from(&mut buff).await?;
    debug!("received broadcast response from: {}", from);
    Ok((buff[..n].to_vec(), from))
}

// Handle a UDP response message
fn handle_broadcast_resp(from: &SocketAddr, data: &[u8]) -> Result<(SocketAddr, String), SearchError> {
    debug!("handling broadcast response from: {}", from);

    // Convert response to text
    let text = std::str::from_utf8(data).map_err(SearchError::from)?;

    // Parse socket address and path
    let (addr, root_url) = parsing::parse_search_result(text)?;

    Ok((SocketAddr::V4(addr), root_url))
}

async fn get_control_urls(addr: &SocketAddr, path: &str) -> Result<(String, String), SearchError> {
    debug!("requesting control url from: http://{}{}", addr, path);
    let body = http_get_bounded(addr, path, MAX_HTTP_RESPONSE_SIZE).await?;
    debug!("handling control response from: {}", addr);
    parsing::parse_control_urls(std::io::Cursor::new(body))
}

async fn get_control_schemas(
    addr: &SocketAddr,
    control_schema_url: &str,
) -> Result<HashMap<String, Vec<String>>, SearchError> {
    debug!("requesting control schema from: http://{}{}", addr, control_schema_url);
    let body = http_get_bounded(addr, control_schema_url, MAX_HTTP_RESPONSE_SIZE).await?;
    debug!("handling schema response from: {}", addr);
    parsing::parse_schemas(std::io::Cursor::new(body))
}

async fn http_get_bounded(addr: &SocketAddr, path: &str, memory_upper_bound: usize) -> Result<Vec<u8>, SearchError> {
    use http_body_util::BodyExt;

    let authority = addr.to_string();
    let uri: Uri = format!("http://{}{}", addr, path).parse()?;

    let url: url::Url = uri.to_string().parse()?;
    validate_url(addr.ip(), &url)?;

    let stream = TcpStream::connect(addr)
        .await
        .map_err(|e| SearchError::HttpError(e.to_string()))?;
    let io = hyper_util::rt::TokioIo::new(stream);

    let (mut sender, connection) = Builder::new()
        .max_buf_size(memory_upper_bound)
        .handshake(io)
        .await
        .map_err(|e| SearchError::HttpError(e.to_string()))?;

    let req = Request::builder()
        .uri(&uri)
        .header(HOST, &authority)
        .body(http_body_util::Empty::<bytes::Bytes>::new())
        .map_err(|e| SearchError::HttpError(e.to_string()))?;

    tokio::spawn(async move {
        // See why we need to await connection:
        // https://docs.rs/hyper/latest/hyper/client/conn/http1/struct.Builder.html#method.handshake
        if let Err(e) = connection.await {
            error!("http connection failed: {e}");
        }
    });

    let resp = sender
        .send_request(req)
        .await
        .map_err(|e| SearchError::HttpError(e.to_string()))?;

    if resp.status() != StatusCode::OK {
        return Err(SearchError::HttpError(format!("unexpected status: {}", resp.status())));
    }

    let body = http_body_util::Limited::new(resp.into_body(), memory_upper_bound)
        .collect()
        .await
        .map_err(|e| SearchError::HttpError(e.to_string()))?
        .to_bytes();

    Ok(body.to_vec())
}

#[cfg(test)]
mod tests {
    use std::{
        convert::Infallible,
        net::{Ipv4Addr, SocketAddrV4},
    };

    use assert_matches::assert_matches;
    use http::Response;
    use http_body_util::StreamBody;
    use httptest::{matchers::request, responders::status_code, Expectation, ServerBuilder};
    use hyper::{
        body::{Bytes, Frame},
        server::conn::http1,
        service::service_fn,
    };
    use hyper_util::rt::TokioIo;
    use rand::{distributions::Alphanumeric, thread_rng, Rng};
    use tokio::net::TcpListener;
    use tokio_stream::wrappers::ReceiverStream;

    use super::*;

    fn generate_random_body(n: usize) -> Vec<u8> {
        let s: String = thread_rng()
            .sample_iter(&Alphanumeric)
            .take(n)
            .map(char::from)
            .collect();
        s.into_bytes()
    }

    #[tokio::test]
    async fn working_http_get_bounded() {
        // 8k is a minimum max buffer size allowed by http1 / hyper:
        // see: https://github.com/hyperium/hyper/blob/0d6c7d5469baa09e2fb127ee3758a79b3271a4f0/src/proto/h1/io.rs#L14-L18
        for memory_bound in [8 * 1024, 16 * 1024, 32 * 1024] {
            for body_size in (0..=memory_bound).step_by(512) {
                let bind_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));
                let server = ServerBuilder::new().bind_addr(bind_addr).run().unwrap();
                let addr = server.addr();
                let get_url = server.url("/get");
                let path = get_url.path();

                let test_body = generate_random_body(body_size);

                server.expect(
                    Expectation::matching(request::method_path("GET", "/get"))
                        .respond_with(status_code(200).body(test_body.clone())),
                );
                let body = http_get_bounded(&addr, path, memory_bound).await.unwrap();

                assert_eq!(test_body, body);
            }
        }
    }

    #[tokio::test]
    async fn failing_http_get_bounded() {
        const MEMORY_BOUND: usize = 16 * 1024;

        let bind_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));
        let server = ServerBuilder::new().bind_addr(bind_addr).run().unwrap();
        let addr = server.addr();
        let get_url = server.url("/get");
        let path = get_url.path();

        let test_body = generate_random_body(MEMORY_BOUND + 1);

        server.expect(
            Expectation::matching(request::method_path("GET", "/get"))
                .respond_with(status_code(200).body(test_body.clone())),
        );
        assert_matches!(
            http_get_bounded(&addr, path, MEMORY_BOUND).await,
            Err(SearchError::HttpError(m)) if m == "length limit exceeded"
        );
    }

    async fn infinite_body_handle(
        _req: Request<hyper::body::Incoming>,
    ) -> Result<Response<StreamBody<ReceiverStream<Result<Frame<Bytes>, Infallible>>>>, Infallible> {
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Frame<Bytes>, Infallible>>(2);

        tokio::spawn(async move {
            let chunk = Bytes::from(vec![b'A'; 4096]);
            loop {
                if tx.send(Ok(Frame::data(chunk.clone()))).await.is_err() {
                    break;
                }
            }
        });

        let stream = ReceiverStream::new(rx);
        let body = StreamBody::new(stream);

        Ok(Response::builder()
            .header("transfer-encoding", "chunked")
            .body(body)
            .unwrap())
    }

    async fn start_infinite_server() -> Result<SocketAddr, Box<dyn std::error::Error>> {
        let addr = SocketAddr::from(([127, 0, 0, 1], 0));
        let listener = TcpListener::bind(addr).await?;
        let addr = listener.local_addr().unwrap();
        eprintln!("Listening on http://{addr}");

        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let io = TokioIo::new(stream);

                tokio::spawn(async move {
                    if let Err(e) = http1::Builder::new()
                        .serve_connection(io, service_fn(infinite_body_handle))
                        .await
                    {
                        eprintln!("connection error: {e}");
                    }
                });
            }
        });

        Ok(addr)
    }

    #[tokio::test]
    async fn search_gateway_should_fail_for_infinite_http_get_body() {
        let http_addr = start_infinite_server().await.unwrap();
        let local_free_port = crate::common::tests::start_broadcast_reply_sender(format!("http://{http_addr}")).await;
        let options = crate::common::tests::default_options_with_using_free_port(local_free_port).await;

        assert_matches!(
            search_gateway(options).await,
            Err(SearchError::HttpError(m)) if m == "length limit exceeded"
        );
    }
}
