pub mod messages;
pub mod options;
pub mod parsing;
#[cfg(test)]
pub mod tests;

pub use self::options::SearchOptions;

use std::net::{IpAddr, SocketAddr};

use rand::{self, Rng};

use crate::errors::SearchError;

pub fn random_port() -> u16 {
    rand::rng().random_range(32_768_u16..65_535_u16)
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
