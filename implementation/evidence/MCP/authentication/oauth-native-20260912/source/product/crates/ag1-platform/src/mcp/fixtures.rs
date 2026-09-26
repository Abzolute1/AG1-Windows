//! Explicit diagnostic builds only. One native-installed literal-loopback HTTPS
//! peer; no renderer/project configuration, DNS exception, redirects or TLS bypass.
use super::invalid;
use ag1_core::DomainError;
use std::{net::IpAddr, sync::OnceLock};
static PEER: OnceLock<TlsPeer> = OnceLock::new();
pub struct TlsPeer {
    origin: String,
    address: IpAddr,
    certificate: Vec<u8>,
}
impl TlsPeer {
    pub fn install(origin: &str, certificate: &[u8]) -> Result<(), DomainError> {
        let url = super::oauth::url(origin, false)?;
        let address = url
            .host_str()
            .unwrap_or_default()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .map_err(|_| invalid("fixture TLS literal address"))?;
        if !address.is_loopback()
            || url.path() != "/"
            || url.origin().ascii_serialization() != origin
            || certificate.len() > 65536
        {
            return Err(invalid("fixture TLS origin"));
        }
        reqwest::Certificate::from_pem(certificate)
            .map_err(|_| invalid("fixture TLS certificate"))?;
        PEER.set(Self {
            origin: origin.into(),
            address,
            certificate: certificate.into(),
        })
        .map_err(|_| DomainError::Conflict)
    }
    pub(crate) fn get() -> Option<&'static Self> {
        PEER.get()
    }
    pub(crate) fn matches(&self, url: &reqwest::Url) -> bool {
        url.origin().ascii_serialization() == self.origin
    }
    pub(crate) fn addresses(&self, values: &[IpAddr]) -> bool {
        values == [self.address]
    }
    pub(crate) fn certificate(&self) -> &[u8] {
        &self.certificate
    }
    pub(crate) fn network(&self) -> super::oauth::Network {
        super::oauth::Network::with_policy(
            std::sync::Arc::new(crate::web_http::SystemResolver),
            crate::web_http::WebPolicy::with_private_origins(std::slice::from_ref(&self.origin))
                .expect("validated fixture origin"),
            std::slice::from_ref(&self.certificate),
        )
        .expect("validated fixture roots")
    }
}
