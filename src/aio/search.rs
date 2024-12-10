use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::timeout;

use crate::aio::Gateway;
use crate::common::{messages, parsing, SearchOptions};
use crate::errors::SearchError;
use crate::search::{check_is_ip_spoofed, validate_url};

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
        control_schema_url,
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
    let url: reqwest::Url = format!("http://{}{}", addr, path).parse()?;

    validate_url(addr.ip(), &url)?;

    debug!("requesting control url from: {:?}", url);
    let client = reqwest::Client::new();
    let resp = client.get(url).send().await?;

    debug!("handling control response from: {}", addr);
    let body = resp.bytes().await?;
    parsing::parse_control_urls(body.as_ref())
}

async fn get_control_schemas(
    addr: &SocketAddr,
    control_schema_url: &str,
) -> Result<HashMap<String, Vec<String>>, SearchError> {
    let url: reqwest::Url = format!("http://{}{}", addr, control_schema_url).parse()?;

    validate_url(addr.ip(), &url)?;

    debug!("requesting control schema from: {}", url);
    let client = reqwest::Client::new();
    let resp = client.get(url).send().await?;

    debug!("handling schema response from: {}", addr);

    let body = resp.bytes().await?;
    parsing::parse_schemas(body.as_ref())
}
