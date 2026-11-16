//! TLS to an HTTPS preview target, spoken by the server over the Runner's
//! forward tunnel: the Runner only moves bytes, so the target sees an ordinary
//! TLS client and the browser sees an ordinary HTTPS preview.

use crate::{error::ApiError, i18n::Message};
use axum::http::StatusCode;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::rustls::{
    self, ClientConfig, DigitallySignedStruct, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{CryptoProvider, ring},
    pki_types::{CertificateDer, ServerName, UnixTime},
};

#[cfg(not(test))]
const HANDSHAKE: Duration = Duration::from_secs(15);
#[cfg(test)]
const HANDSHAKE: Duration = Duration::from_secs(5);

/// How Ferry talks to a preview target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tls {
    /// Plain HTTP.
    Plain,
    /// HTTPS, certificate verified against public roots.
    Verify,
    /// HTTPS, certificate accepted as presented (self-signed devices).
    Insecure,
}

impl Tls {
    pub(super) fn of(scheme: &str, insecure: bool) -> Self {
        match (scheme, insecure) {
            ("https", true) => Self::Insecure,
            ("https", false) => Self::Verify,
            _ => Self::Plain,
        }
    }
    /// One letter in the pool authority, so a changed setting never reuses
    /// an idle connection made under the old one.
    pub(super) fn code(self) -> char {
        match self {
            Self::Plain => 'p',
            Self::Verify => 's',
            Self::Insecure => 'i',
        }
    }
    pub(super) fn from_code(code: &str) -> Option<Self> {
        match code {
            "p" => Some(Self::Plain),
            "s" => Some(Self::Verify),
            "i" => Some(Self::Insecure),
            _ => None,
        }
    }
    pub(super) fn scheme(self) -> &'static str {
        if self == Self::Plain { "http" } else { "https" }
    }
}

/// Anything the hyper connection can run over: a plain pipe or TLS on it.
pub(super) trait Io: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Io for T {}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring::default_provider())
}

fn config(verifier: Option<Arc<dyn ServerCertVerifier>>) -> Arc<ClientConfig> {
    let builder = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions");
    let mut config = match verifier {
        Some(verifier) => builder
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth(),
        None => builder
            .with_root_certificates(rustls::RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            })
            .with_no_client_auth(),
    };
    // The proxy speaks HTTP/1.1 (WebSocket upgrades included).
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(config)
}

static VERIFIED: LazyLock<Arc<ClientConfig>> = LazyLock::new(|| config(None));
static UNVERIFIED: LazyLock<Arc<ClientConfig>> =
    LazyLock::new(|| config(Some(Arc::new(AcceptAnyCertificate(provider())))));

/// Accepts whatever certificate the target presents, but still checks that
/// the handshake is signed by it, so the session is bound to that key.
#[derive(Debug)]
struct AcceptAnyCertificate(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAnyCertificate {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// The name TLS uses for the target: its DNS name (sent as SNI), its IP (no
/// SNI is sent for IP literals), or `localhost` for the Runner's own loopback.
fn server_name(address: &str) -> Result<ServerName<'static>, ApiError> {
    let host = address.trim_start_matches('[').trim_end_matches(']');
    if host.is_empty() {
        return Ok(ServerName::try_from("localhost").expect("valid name"));
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return Ok(ServerName::IpAddress(ip.into()));
    }
    ServerName::try_from(host.to_owned()).map_err(|_| failed("holes.tlsHandshake", "invalid name"))
}

fn failed(key: &'static str, detail: &str) -> ApiError {
    ApiError::localized(
        StatusCode::BAD_GATEWAY,
        Message::new(key).with("detail", detail.chars().take(200).collect::<String>()),
    )
}

/// Turns a failed handshake into the reason a person can act on.
fn reason(error: &std::io::Error) -> ApiError {
    let inner = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>());
    match inner {
        Some(rustls::Error::InvalidCertificate(why)) => {
            failed("holes.tlsUntrusted", &format!("{why:?}"))
        }
        // An HTTP answer where a TLS record was expected.
        Some(rustls::Error::InvalidMessage(_)) => failed("holes.tlsPlainHttp", ""),
        Some(other) => failed("holes.tlsHandshake", &other.to_string()),
        None => failed("holes.tlsHandshake", &error.to_string()),
    }
}

/// Runs the TLS handshake with the target over `io`, bounded in time.
pub(super) async fn connect<T>(tls: Tls, address: &str, io: T) -> Result<Box<dyn Io>, ApiError>
where
    T: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let config = match tls {
        Tls::Plain => return Ok(Box::new(io)),
        Tls::Verify => VERIFIED.clone(),
        Tls::Insecure => UNVERIFIED.clone(),
    };
    let connector = tokio_rustls::TlsConnector::from(config);
    let name = server_name(address)?;
    match tokio::time::timeout(HANDSHAKE, connector.connect(name, io)).await {
        Ok(Ok(stream)) => Ok(Box::new(stream)),
        Ok(Err(error)) => Err(reason(&error)),
        Err(_) => Err(failed("holes.tlsHandshake", "timed out")),
    }
}
