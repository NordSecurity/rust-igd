use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::net::{SocketAddrV4, UdpSocket};
use std::str::FromStr;

use crate::common::{messages, parsing, SearchOptions};
use crate::errors::SearchError;
use crate::gateway::Gateway;

/// Search gateway, using the given `SearchOptions`.
///
/// The default `SearchOptions` should suffice in most cases.
/// It can be created with `Default::default()` or `SearchOptions::default()`.
///
/// # Example
/// ```no_run
/// use igd::{search_gateway, SearchOptions, Result};
///
/// fn main() -> Result {
///     let gateway = search_gateway(Default::default())?;
///     let ip = gateway.get_external_ip()?;
///     println!("External IP address: {}", ip);
///     Ok(())
/// }
/// ```
pub fn search_gateway(options: SearchOptions) -> Result<Gateway, SearchError> {
    let socket = UdpSocket::bind(options.bind_addr)?;
    socket.set_read_timeout(options.timeout)?;

    socket.send_to(messages::SEARCH_REQUEST.as_bytes(), options.broadcast_address)?;

    let mut buf = [0u8; 1500];
    let (read, from) = socket.recv_from(&mut buf)?;
    let text = std::str::from_utf8(&buf[..read])?;

    let (addr, root_url) = parsing::parse_search_result(text)?;

    check_is_ip_spoofed(&from, &addr.into())?;

    let (control_schema_url, control_url) = get_control_urls(&addr, &root_url).map_err(|e| {
        debug!(
            "Error has occurred while getting control urls. error: {}, addr: {}, root_url: {}",
            e, addr, root_url
        );
        e
    })?;

    let control_schema = get_control_schemas(&addr, &control_schema_url).map_err(|e| {
        debug!(
            "Error has occurred while getting schemas. error: {}, addr: {}, control_schema_url: {}",
            e, addr, control_schema_url
        );
        e
    })?;

    let gateway = Gateway {
        addr,
        root_url,
        control_url,
        control_schema_url,
        control_schema,
    };

    let gateway_url = reqwest::Url::from_str(&format!("{gateway}"))?;

    validate_url((*addr.ip()).into(), &gateway_url)?;

    return Ok(gateway);
}

fn get_control_urls(addr: &SocketAddrV4, path: &str) -> Result<(String, String), SearchError> {
    let url: reqwest::Url = format!("http://{}{}", addr, path).parse()?;

    validate_url((*addr.ip()).into(), &url)?;

    debug!("requesting control url from: {:?}", url);
    let client = reqwest::blocking::Client::new();
    let resp = client.get(url).send()?;

    debug!("handling control response from: {}", addr);
    let body = resp.bytes()?;
    parsing::parse_control_urls(body.as_ref())
}

fn get_control_schemas(
    addr: &SocketAddrV4,
    control_schema_url: &str,
) -> Result<HashMap<String, Vec<String>>, SearchError> {
    let url: reqwest::Url = format!("http://{}{}", addr, control_schema_url).parse()?;

    validate_url((*addr.ip()).into(), &url)?;

    debug!("requesting control schema from: {}", url);
    let client = reqwest::blocking::Client::new();
    let resp = client.get(url).send()?;

    debug!("handling schema response from: {}", addr);

    let body = resp.bytes()?;
    parsing::parse_schemas(body.as_ref())
}

pub fn check_is_ip_spoofed(from: &SocketAddr, addr: &SocketAddr) -> Result<(), SearchError> {
    match (from, addr) {
        (SocketAddr::V4(src_ip), SocketAddr::V4(url_ip)) => {
            if src_ip.ip() != url_ip.ip() {
                return Err(SearchError::SpoofedIp {
                    src_ip: (*src_ip.ip()).into(),
                    url_ip: (*url_ip.ip()).into(),
                });
            }
        }
        (SocketAddr::V6(src_ip), SocketAddr::V6(url_ip)) => {
            if src_ip.ip() != url_ip.ip() {
                return Err(SearchError::SpoofedIp {
                    src_ip: (*src_ip.ip()).into(),
                    url_ip: (*url_ip.ip()).into(),
                });
            }
        }
        (SocketAddr::V6(src_ip), SocketAddr::V4(url_ip)) => {
            return Err(SearchError::SpoofedIp {
                src_ip: (*src_ip.ip()).into(),
                url_ip: (*url_ip.ip()).into(),
            })
        }
        (SocketAddr::V4(src_ip), SocketAddr::V6(url_ip)) => {
            return Err(SearchError::SpoofedIp {
                src_ip: (*src_ip.ip()).into(),
                url_ip: (*url_ip.ip()).into(),
            })
        }
    }
    Ok(())
}

pub fn validate_url(src_ip: IpAddr, url: &reqwest::Url) -> Result<(), SearchError> {
    match url.host_str() {
        Some(url_host) if url_host != src_ip.to_string() => Err(SearchError::SpoofedUrl {
            src_ip,
            url_host: url_host.to_owned(),
        }),
        None => Err(SearchError::UrlMissingHost(url.clone())),
        _ => Ok(()),
    }
}
