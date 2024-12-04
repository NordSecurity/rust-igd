use crate::{search_gateway, SearchError, SearchOptions};
use http_body_util::Full;
use hyper::{body::Bytes, service::service_fn, Request, Response};
use hyper_util::rt::TokioIo;
use std::{
    convert::Infallible,
    future::Future,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    time::Duration,
};
use test_log::test;
use tokio::net::{TcpListener, UdpSocket};

async fn start_broadcast_reply_sender(location: String) -> u16 {
    let local_free_port = {
        // Not 100% reliable way to find a free port number, but should be good enough
        let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await.unwrap();
        let ret = sock.local_addr().unwrap().port();
        ret
    };

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let sock = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();

        sock.send_to(
            format!("location: {location}").as_bytes(),
            (Ipv4Addr::LOCALHOST, local_free_port),
        )
        .await
        .unwrap();
    });
    local_free_port
}

fn default_options_with_using_free_port(port: u16) -> SearchOptions {
    SearchOptions {
        bind_addr: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port)),
        timeout: Some(Duration::from_secs(5)),
        http_timeout: Some(Duration::from_secs(1)),
        ..Default::default()
    }
}

const RESP_SPOOFED_SCPDURL: &'static str = r#"<?xml version="1.0" ?>
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

const RESP_SPOOFED_CONTROL_URL: &'static str = r#"<?xml version="1.0" ?>
    <root xmlns="urn:schemas-upnp-org:device-1-0">
        <device>
            <deviceList>
                <device>
                    <deviceList>
                        <device>
                            <serviceList>
                                <service>
                                    <serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType>
                                    <controlURL>:aaa@example2.com/exec_cmd?cmd=touch%20%2ftmp%2frce</controlURL>
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

const RESP_CONTROL_SCHEMA: &'static str = r#"<?xml version="1.0" ?>
    <root xmlns="urn:schemas-upnp-org:device-1-0">
    <actionList>
        <action>
        </action>
    </actionList>
    </root>
    "#;

async fn start_http_server(responses: Vec<String>) -> u16 {
    let listener = TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], 0))).await.unwrap();
    let local_port = listener.local_addr().unwrap().port();

    tokio::task::spawn(async move {
        for resp in responses {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);

            let handler = move |_r: Request<hyper::body::Incoming>| -> Result<Response<Full<Bytes>>, Infallible> {
                Ok(Response::new(Full::new(Bytes::from(resp.clone()))))
            };

            if let Err(err) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service_fn(|r| async { handler(r) }))
                .await
            {
                eprintln!("Error serving connection: {:?}", err);
            }
        }
    });

    local_port
}

#[test(tokio::test)]
async fn ip_spoofing_in_broadcast_response() {
    async fn aux<F, Fut>(search_gateway: F)
    where
        Fut: Future<Output = Result<(), SearchError>>,
        F: Fn(SearchOptions) -> Fut,
    {
        let local_free_port = start_broadcast_reply_sender("http://1.2.3.4:5".to_owned()).await;

        let options = default_options_with_using_free_port(local_free_port);

        let result = search_gateway(options).await;
        if let Err(SearchError::SpoofedIp { src_ip, url_ip }) = result {
            assert_eq!(src_ip, Ipv4Addr::LOCALHOST);
            assert_eq!(url_ip, Ipv4Addr::new(1, 2, 3, 4));
        } else {
            panic!("Unexpected result: {result:?}");
        }
    }

    aux(|opt| async {
        tokio::task::spawn_blocking(|| search_gateway(opt).map(|_| ()))
            .await
            .unwrap()
    })
    .await;
    #[cfg(feature = "aio")]
    aux(|opt| async { crate::aio::search_gateway(opt).await.map(|_| ()) }).await;
}

#[test(tokio::test)]
async fn ip_spoofing_in_getxml_body() {
    async fn aux<F, Fut>(search_gateway: F)
    where
        Fut: Future<Output = Result<(), SearchError>>,
        F: Fn(SearchOptions) -> Fut,
    {
        let http_port = start_http_server(vec![RESP_SPOOFED_SCPDURL.to_owned()]).await;

        let local_free_port = start_broadcast_reply_sender(format!("http://127.0.0.1:{http_port}")).await;

        println!("http server port: {http_port}, udp port: {local_free_port}");

        let options = default_options_with_using_free_port(local_free_port);

        let result = search_gateway(options).await;
        if let Err(SearchError::SpoofedUrl { src_ip, url_host }) = result {
            assert_eq!(src_ip, Ipv4Addr::LOCALHOST);
            assert_eq!(url_host, "example.com");
        } else {
            panic!("Unexpected result: {result:?}");
        }
    }
    aux(|opt| async {
        tokio::task::spawn_blocking(|| search_gateway(opt).map(|_| ()))
            .await
            .unwrap()
    })
    .await;
    #[cfg(feature = "aio")]
    aux(|opt| async { crate::aio::search_gateway(opt).await.map(|_| ()) }).await;
}

#[test(tokio::test)]
async fn ip_spoofing_in_getxml_body_control_url() {
    async fn aux<F, Fut>(search_gateway: F)
    where
        Fut: Future<Output = Result<(), SearchError>>,
        F: Fn(SearchOptions) -> Fut,
    {
        let http_port = start_http_server(vec![
            RESP_SPOOFED_CONTROL_URL.to_owned(),
            RESP_CONTROL_SCHEMA.to_owned(),
        ])
        .await;

        let local_free_port = start_broadcast_reply_sender(format!("http://127.0.0.1:{http_port}")).await;

        let options = default_options_with_using_free_port(local_free_port);

        let result = search_gateway(options).await;

        if let Err(SearchError::SpoofedUrl { src_ip, url_host }) = result {
            assert_eq!(src_ip, Ipv4Addr::LOCALHOST);
            assert_eq!(url_host, "example2.com");
        } else {
            panic!("Unexpected result: {result:?}");
        }
    }
    aux(|opt| async {
        tokio::task::spawn_blocking(|| search_gateway(opt).map(|_| ()))
            .await
            .unwrap()
    })
    .await;
    #[cfg(feature = "aio")]
    aux(|opt| async { crate::aio::search_gateway(opt).await.map(|_| ()) }).await;
}
