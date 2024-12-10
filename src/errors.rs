use std::error;
use std::fmt;
use std::io;
use std::net::IpAddr;
use std::str;
#[cfg(feature = "aio")]
use std::string::FromUtf8Error;

#[cfg(feature = "aio")]
use tokio::time::error::Elapsed;

/// Errors that can occur when sending the request to the gateway.
#[derive(Debug, thiserror::Error)]
pub enum RequestError {
    #[error("IO Error: {0}")]
    /// I/O error
    IoError(#[from] io::Error),
    #[error("The response from the gateway could not be parsed: '{0}'")]
    /// Invalid response
    InvalidResponse(String),
    #[error("The gateway returned an unhandled error code {0} and description: {1}")]
    /// Unhandled error code
    ErrorCode(u16, String),
    #[error("Action is not supported by the gateway: {0}")]
    /// Unsupported action
    UnsupportedAction(String),
    #[error("Reqwest error: {0}")]
    /// Reqwest error
    ReqwestError(#[from] reqwest::Error),
    #[cfg(feature = "aio")]
    #[error("Error parsing HTTP body as utf-8: {0}")]
    /// Utf-8 parsning error
    Utf8Error(#[from] FromUtf8Error),
}

#[cfg(feature = "aio")]
impl From<Elapsed> for RequestError {
    fn from(_err: Elapsed) -> RequestError {
        RequestError::IoError(io::Error::new(io::ErrorKind::TimedOut, "timer failed"))
    }
}

/// Errors returned by `Gateway::get_external_ip`
#[derive(Debug)]
pub enum GetExternalIpError {
    /// The client is not authorized to perform the operation.
    ActionNotAuthorized,
    /// Some other error occured performing the request.
    RequestError(RequestError),
}

/// Errors returned by `Gateway::remove_port`
#[derive(Debug)]
pub enum RemovePortError {
    /// The client is not authorized to perform the operation.
    ActionNotAuthorized,
    /// No such port mapping.
    NoSuchPortMapping,
    /// Some other error occured performing the request.
    RequestError(RequestError),
}

/// Errors returned by `Gateway::add_any_port` and `Gateway::get_any_address`
#[derive(Debug)]
pub enum AddAnyPortError {
    /// The client is not authorized to perform the operation.
    ActionNotAuthorized,
    /// Can not add a mapping for local port 0.
    InternalPortZeroInvalid,
    /// The gateway does not have any free ports.
    NoPortsAvailable,
    /// The gateway can only map internal ports to same-numbered external ports
    /// and this external port is in use.
    ExternalPortInUse,
    /// The gateway only supports permanent leases (ie. a `lease_duration` of 0).
    OnlyPermanentLeasesSupported,
    /// The description was too long for the gateway to handle.
    DescriptionTooLong,
    /// Some other error occured performing the request.
    RequestError(RequestError),
}

impl From<RequestError> for AddAnyPortError {
    fn from(err: RequestError) -> AddAnyPortError {
        AddAnyPortError::RequestError(err)
    }
}

impl From<GetExternalIpError> for AddAnyPortError {
    fn from(err: GetExternalIpError) -> AddAnyPortError {
        match err {
            GetExternalIpError::ActionNotAuthorized => AddAnyPortError::ActionNotAuthorized,
            GetExternalIpError::RequestError(e) => AddAnyPortError::RequestError(e),
        }
    }
}

/// Errors returned by `Gateway::add_port`
#[derive(Debug)]
pub enum AddPortError {
    /// The client is not authorized to perform the operation.
    ActionNotAuthorized,
    /// Can not add a mapping for local port 0.
    InternalPortZeroInvalid,
    /// External port number 0 (any port) is considered invalid by the gateway.
    ExternalPortZeroInvalid,
    /// The requested mapping conflicts with a mapping assigned to another client.
    PortInUse,
    /// The gateway requires that the requested internal and external ports are the same.
    SamePortValuesRequired,
    /// The gateway only supports permanent leases (ie. a `lease_duration` of 0).
    OnlyPermanentLeasesSupported,
    /// The description was too long for the gateway to handle.
    DescriptionTooLong,
    /// Some other error occured performing the request.
    RequestError(RequestError),
}

impl fmt::Display for GetExternalIpError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            GetExternalIpError::ActionNotAuthorized => write!(f, "The client is not authorized to remove the port"),
            GetExternalIpError::RequestError(ref e) => write!(f, "Request Error. {}", e),
        }
    }
}

impl From<io::Error> for GetExternalIpError {
    fn from(err: io::Error) -> GetExternalIpError {
        GetExternalIpError::RequestError(RequestError::from(err))
    }
}

impl std::error::Error for GetExternalIpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

impl fmt::Display for RemovePortError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            RemovePortError::ActionNotAuthorized => write!(f, "The client is not authorized to remove the port"),
            RemovePortError::NoSuchPortMapping => write!(f, "The port was not mapped"),
            RemovePortError::RequestError(ref e) => write!(f, "Request error. {}", e),
        }
    }
}

impl std::error::Error for RemovePortError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

impl fmt::Display for AddAnyPortError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            AddAnyPortError::ActionNotAuthorized => {
                write!(f, "The client is not authorized to remove the port")
            }
            AddAnyPortError::InternalPortZeroInvalid => {
                write!(f, "Can not add a mapping for local port 0")
            }
            AddAnyPortError::NoPortsAvailable => {
                write!(f, "The gateway does not have any free ports")
            }
            AddAnyPortError::OnlyPermanentLeasesSupported => {
                write!(
                    f,
                    "The gateway only supports permanent leases (ie. a `lease_duration` of 0),"
                )
            }
            AddAnyPortError::ExternalPortInUse => {
                write!(
                    f,
                    "The gateway can only map internal ports to same-numbered external ports and this external port is in use."
                )
            }
            AddAnyPortError::DescriptionTooLong => {
                write!(f, "The description was too long for the gateway to handle.")
            }
            AddAnyPortError::RequestError(ref e) => write!(f, "Request error. {}", e),
        }
    }
}

impl std::error::Error for AddAnyPortError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

impl fmt::Display for AddPortError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            AddPortError::ActionNotAuthorized => write!(f, "The client is not authorized to map this port."),
            AddPortError::InternalPortZeroInvalid => write!(f, "Can not add a mapping for local port 0"),
            AddPortError::ExternalPortZeroInvalid => write!(
                f,
                "External port number 0 (any port) is considered invalid by the gateway."
            ),
            AddPortError::PortInUse => write!(
                f,
                "The requested mapping conflicts with a mapping assigned to another client."
            ),
            AddPortError::SamePortValuesRequired => write!(
                f,
                "The gateway requires that the requested internal and external ports are the same."
            ),
            AddPortError::OnlyPermanentLeasesSupported => write!(
                f,
                "The gateway only supports permanent leases (ie. a `lease_duration` of 0),"
            ),
            AddPortError::DescriptionTooLong => write!(f, "The description was too long for the gateway to handle."),
            AddPortError::RequestError(ref e) => write!(f, "Request error. {}", e),
        }
    }
}

impl std::error::Error for AddPortError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

/// Errors than can occur while trying to find the gateway.
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("Unable to process the response")]
    /// Invalid response
    InvalidResponse,
    #[error("IO Error: {0}")]
    /// I/O error
    IoError(#[from] io::Error),
    #[error("UTF-8 decoding error: {0}")]
    /// Utf-8 parsing error
    Utf8Error(#[from] str::Utf8Error),
    #[error("XML processing error: {0}")]
    /// Xml parsing error
    XmlError(#[from] xmltree::ParseError),
    #[error("Reqwest error: {0}")]
    /// Reqwest error
    ReqwestError(#[from] reqwest::Error),
    #[error("Error parsing URI: {0}")]
    /// Invalid uri
    InvalidUri(#[from] url::ParseError),
    #[error("The uri is missing the host: {0}")]
    /// Uri is missing host
    UrlMissingHost(reqwest::Url),
    #[error("Ip {src_ip} spoofed as {url_ip}")]
    /// IP spoofing error
    SpoofedIp {
        /// The IP from which packet was actually received
        src_ip: IpAddr,
        /// The IP which the receiving packet pretended to be from
        url_ip: IpAddr,
    },
    /// Uri spoofing detected error
    #[error("Ip {src_ip} spoofed in url as {url_host}")]
    /// Url host spoofing error
    SpoofedUrl {
        /// The IP from which packet was actually received
        src_ip: IpAddr,
        /// The IP which the receiving packet pretended to be from
        url_host: String,
    },
}

#[cfg(feature = "aio")]
impl From<Elapsed> for SearchError {
    fn from(_err: Elapsed) -> SearchError {
        SearchError::IoError(io::Error::new(io::ErrorKind::TimedOut, "search timed out"))
    }
}

/// Errors than can occur while getting a port mapping
#[derive(Debug)]
pub enum GetGenericPortMappingEntryError {
    /// The client is not authorized to perform the operation.
    ActionNotAuthorized,
    /// The specified array index is out of bounds.
    SpecifiedArrayIndexInvalid,
    /// Some other error occured performing the request.
    RequestError(RequestError),
}

impl From<RequestError> for GetGenericPortMappingEntryError {
    fn from(err: RequestError) -> GetGenericPortMappingEntryError {
        match err {
            RequestError::ErrorCode(606, _) => GetGenericPortMappingEntryError::ActionNotAuthorized,
            RequestError::ErrorCode(713, _) => GetGenericPortMappingEntryError::SpecifiedArrayIndexInvalid,
            other => GetGenericPortMappingEntryError::RequestError(other),
        }
    }
}

impl fmt::Display for GetGenericPortMappingEntryError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            GetGenericPortMappingEntryError::ActionNotAuthorized => {
                write!(f, "The client is not authorized to look up port mappings.")
            }
            GetGenericPortMappingEntryError::SpecifiedArrayIndexInvalid => {
                write!(f, "The provided index into the port mapping list is invalid.")
            }
            GetGenericPortMappingEntryError::RequestError(ref e) => e.fmt(f),
        }
    }
}

impl std::error::Error for GetGenericPortMappingEntryError {}

/// An error type that emcompasses all possible errors.
#[derive(Debug)]
pub enum Error {
    /// `AddAnyPortError`
    AddAnyPortError(AddAnyPortError),
    /// `AddPortError`
    AddPortError(AddPortError),
    /// `GetExternalIpError`
    GetExternalIpError(GetExternalIpError),
    /// `RemovePortError`
    RemovePortError(RemovePortError),
    /// `RequestError`
    RequestError(RequestError),
    /// `SearchError`
    SearchError(SearchError),
}

/// A result type where the error is `igd::Error`.
pub type Result<T = ()> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Error::AddAnyPortError(ref e) => e.fmt(f),
            Error::AddPortError(ref e) => e.fmt(f),
            Error::GetExternalIpError(ref e) => e.fmt(f),
            Error::RemovePortError(ref e) => e.fmt(f),
            Error::RequestError(ref e) => e.fmt(f),
            Error::SearchError(ref e) => e.fmt(f),
        }
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Error::AddAnyPortError(ref e) => Some(e),
            Error::AddPortError(ref e) => Some(e),
            Error::GetExternalIpError(ref e) => Some(e),
            Error::RemovePortError(ref e) => Some(e),
            Error::RequestError(ref e) => Some(e),
            Error::SearchError(ref e) => Some(e),
        }
    }
}

impl From<AddAnyPortError> for Error {
    fn from(err: AddAnyPortError) -> Error {
        Error::AddAnyPortError(err)
    }
}

impl From<AddPortError> for Error {
    fn from(err: AddPortError) -> Error {
        Error::AddPortError(err)
    }
}

impl From<GetExternalIpError> for Error {
    fn from(err: GetExternalIpError) -> Error {
        Error::GetExternalIpError(err)
    }
}

impl From<RemovePortError> for Error {
    fn from(err: RemovePortError) -> Error {
        Error::RemovePortError(err)
    }
}

impl From<RequestError> for Error {
    fn from(err: RequestError) -> Error {
        Error::RequestError(err)
    }
}

impl From<SearchError> for Error {
    fn from(err: SearchError) -> Error {
        Error::SearchError(err)
    }
}
