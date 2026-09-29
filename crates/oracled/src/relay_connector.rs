use std::net::Ipv4Addr;
use std::sync::Arc;

use nostro2_relay::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use nostro2_relay::rustls::crypto::{self, CryptoProvider};
use nostro2_relay::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use nostro2_relay::rustls::{ClientConfig, DigitallySignedStruct, Error, SignatureScheme};
use nostro2_relay::{DriverConfig, NostrRelay, ReconnectConfig, RelayTls, RelayUrl};

/// Accepts any server certificate while still checking handshake signatures.
///
/// This defeats authentication of the relay, so [`RelayConnector`] offers it
/// only for hosts on a private network.
#[derive(Debug)]
struct AcceptAnyCertificate {
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for AcceptAnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        crypto::verify_tls12_signature(
            message,
            cert,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        crypto::verify_tls13_signature(
            message,
            cert,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Opens the relay connection, optionally without certificate checking.
#[derive(Debug, Clone)]
pub struct RelayConnector {
    url: String,
    insecure: bool,
}

impl RelayConnector {
    #[must_use]
    pub fn new(url: impl Into<String>, insecure: bool) -> Self {
        Self {
            url: url.into(),
            insecure,
        }
    }

    /// Whether `host` is a literal address on a private or loopback range.
    #[must_use]
    pub fn is_private_host(host: &str) -> bool {
        host.parse::<Ipv4Addr>()
            .is_ok_and(|address| address.is_private() || address.is_loopback())
    }

    /// # Errors
    /// Returns a message if the URL is invalid, certificate checking is
    /// disabled for a host that is not on a private network, or the first
    /// connection fails.
    pub fn connect(&self, reconnect: ReconnectConfig) -> Result<NostrRelay, String> {
        let url = RelayUrl::parse(&self.url).map_err(|error| format!("relay url: {error}"))?;
        let mut config = DriverConfig::new(url.clone()).with_reconnect(reconnect);
        if self.insecure {
            if !Self::is_private_host(url.host()) {
                return Err(format!(
                    "--relay-insecure is limited to private-network addresses, not {}",
                    url.host()
                ));
            }
            config = config.with_tls(Self::unverified_tls()?);
        }
        NostrRelay::connect_blocking_config(config)
            .map_err(|error| format!("connect {}: {error}", self.url))
    }

    fn unverified_tls() -> Result<RelayTls, String> {
        let provider = Arc::new(crypto::ring::default_provider());
        let verifier = Arc::new(AcceptAnyCertificate {
            provider: Arc::clone(&provider),
        });
        let config = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|error| format!("tls versions: {error}"))?
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();
        Ok(RelayTls::from_config(config))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nostro2_relay::ReconnectConfig;

    use super::RelayConnector;

    #[test]
    fn private_and_loopback_addresses_are_private_hosts() {
        assert!(RelayConnector::is_private_host("192.168.1.239"));
        assert!(RelayConnector::is_private_host("10.0.0.7"));
        assert!(RelayConnector::is_private_host("172.16.4.1"));
        assert!(RelayConnector::is_private_host("127.0.0.1"));
    }

    #[test]
    fn public_addresses_and_names_are_not_private_hosts() {
        assert!(!RelayConnector::is_private_host("8.8.8.8"));
        assert!(!RelayConnector::is_private_host("172.32.0.1"));
        assert!(!RelayConnector::is_private_host("rof.illuminodes.com"));
        assert!(!RelayConnector::is_private_host("192.168.1.239.evil.com"));
        assert!(!RelayConnector::is_private_host(""));
    }

    #[test]
    fn disabling_verification_for_a_public_host_is_refused() {
        let error = RelayConnector::new("wss://rof.illuminodes.com", true)
            .connect(ReconnectConfig::disabled())
            .expect_err("refused");

        assert!(error.contains("private-network"), "{error}");
    }

    #[test]
    fn a_bad_url_is_reported() {
        let error = RelayConnector::new("not a url", false)
            .connect(ReconnectConfig {
                max_retries: 1,
                initial_delay: Duration::from_millis(10),
                max_delay: Duration::from_millis(10),
                backoff_multiplier: 1.0,
            })
            .expect_err("rejected");

        assert!(error.contains("relay url"), "{error}");
    }
}
