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

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::Full;
    use hyper::{body::Bytes, service::service_fn, Request, Response};
    use hyper_util::rt::TokioIo;
    use std::convert::Infallible;
    use std::{
        net::{Ipv4Addr, SocketAddrV4},
        time::Duration,
    };
    use test_log::test;
    use tokio::net::TcpListener;

    async fn start_broadcast_reply_sender(location: String) -> u16 {
        let port = {
            // Not 100% reliable way to find a free port number, but should be good enough
            let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await.unwrap();
            sock.local_addr().unwrap().port()
        };

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;

            let sock = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();

            sock.send_to(format!("location: {location}").as_bytes(), (Ipv4Addr::LOCALHOST, port))
                .await
                .unwrap();
        });
        port
    }

    fn default_options_with_using_free_port(port: u16) -> SearchOptions {
        SearchOptions {
            bind_addr: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port)),
            timeout: Some(Duration::from_secs(5)),
            http_timeout: Some(Duration::from_secs(1)),
            ..Default::default()
        }
    }

    #[test(tokio::test)]
    async fn ip_spoofing_in_broadcast_response() {
        let port = start_broadcast_reply_sender("http://1.2.3.4:5".to_owned()).await;

        let options = default_options_with_using_free_port(port);

        let result = search_gateway(options).await;
        if let Err(SearchError::SpoofedIp { src_ip, url_ip }) = result {
            assert_eq!(src_ip, Ipv4Addr::LOCALHOST);
            assert_eq!(url_ip, Ipv4Addr::new(1, 2, 3, 4));
        } else {
            panic!("Unexpected result: {result:?}");
        }
    }

    const RESP: &'static str = r#"<?xml version="1.0" ?>
    <root xmlns="urn:schemas-upnp-org:device-1-0">
        <device>
            <deviceList>
                <device>
                    <deviceList>
                        <device>
                            <serviceList>
                                <service>
                                    <serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType>
                                    <controlURL>/igdupnp/control/WANIPConn1</controlURL>
                                    <SCPDURL>:aaa@example.com/exec_cmd?cmd=touch%20%2ftmp%2frce</SCPDURL>
                                </service>
                            </serviceList>
                        </device>
                    </deviceList>
                </device>
            </deviceList>
        </device>
    </root>
    "#;
    const RESP2: &'static str = r#"<?xml version="1.0" ?>
    <root xmlns="urn:schemas-upnp-org:device-1-0">
        <device>
            <deviceList>
                <device>
                    <deviceList>
                        <device>
                            <serviceList>
                                <service>
                                    <serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType>
                                    <controlURL>:aaa@example.com/exec_cmd?cmd=touch%20%2ftmp%2frce</controlURL>
                                    <SCPDURL>/igdupnp/control/WANIPConn1</SCPDURL>
                                </service>
                            </serviceList>
                        </device>
                    </deviceList>
                </device>
            </deviceList>
        </device>
    </root>
    "#;
    const CONTROL_SCHEMA: &'static str = r#"<?xml version="1.0" ?>
    <root xmlns="urn:schemas-upnp-org:device-1-0">
    <actionList>
        <action>
        </action>
    </actionList>
    </root>
    "#;

    async fn start_http_server(responses: Vec<String>) -> u16 {
        let addr = SocketAddr::from(([0, 0, 0, 0], 0));

        // We create a TcpListener and bind it to 0.0.0.0:0
        let listener = TcpListener::bind(addr).await.unwrap();

        let ret = listener.local_addr().unwrap().port();

        tokio::task::spawn(async move {
            for resp in responses {
                let (stream, _) = listener.accept().await.unwrap();

                // Use an adapter to access something implementing `tokio::io` traits as if they implement
                // `hyper::rt` IO traits.
                let io = TokioIo::new(stream);

                let hello_fn = move |r: Request<hyper::body::Incoming>| -> Result<Response<Full<Bytes>>, Infallible> {
                    println!("Request: {r:?}");
                    Ok(Response::new(Full::new(Bytes::from(resp.clone()))))
                };

                // Finally, we bind the incoming connection to our `hello` service
                if let Err(err) = hyper::server::conn::http1::Builder::new()
                    // `service_fn` converts our function in a `Service`
                    .serve_connection(io, service_fn(|r| async { hello_fn(r) }))
                    .await
                {
                    eprintln!("Error serving connection: {:?}", err);
                }
            }
        });

        ret
    }

    #[test(tokio::test)]
    async fn ip_spoofing_in_getxml_body() {
        let http_port = start_http_server(vec![RESP.to_owned()]).await;

        let port = start_broadcast_reply_sender(format!("http://127.0.0.1:{http_port}")).await;

        println!("http server port: {http_port}, udp port: {port}");

        let options = default_options_with_using_free_port(port);

        let result = search_gateway(options).await;
        if let Err(SearchError::SpoofedUrl { src_ip, url_host }) = result {
            assert_eq!(src_ip, Ipv4Addr::LOCALHOST);
            assert_eq!(url_host, "example.com");
        } else {
            panic!("Unexpected result: {result:?}");
        }
    }

    #[test(tokio::test)]
    async fn ip_spoofing_in_getxml_body_control_url() {
        let http_port = start_http_server(vec![RESP2.to_owned(), CONTROL_SCHEMA.to_owned()]).await;

        let port = start_broadcast_reply_sender(format!("http://127.0.0.1:{http_port}")).await;

        println!("http server port: {http_port}, udp port: {port}");

        let options = default_options_with_using_free_port(port);

        let result = search_gateway(options).await;

        if let Err(SearchError::SpoofedUrl { src_ip, url_host }) = result {
            assert_eq!(src_ip, Ipv4Addr::LOCALHOST);
            assert_eq!(url_host, "example.com");
        } else {
            panic!("Unexpected result: {result:?}");
        }
    }
}
