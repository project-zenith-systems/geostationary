use std::sync::Arc;
use std::time::Duration;

use quinn::{ClientConfig, IdleTimeout, ServerConfig, TransportConfig};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

pub(crate) fn generate_self_signed_cert()
-> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), Box<dyn std::error::Error>> {
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()])?;
    let cert_der = cert.der().clone();
    let key_der = PrivateKeyDer::try_from(signing_key.serialize_der())?;
    Ok((cert_der, key_der))
}

pub(crate) fn build_server_config() -> Result<ServerConfig, Box<dyn std::error::Error>> {
    let (cert, key) = generate_self_signed_cert()?;
    let mut server_config = ServerConfig::with_single_cert(vec![cert], key)?;
    server_config.transport_config(Arc::new(transport_config()));
    Ok(server_config)
}

pub(crate) fn build_client_config() -> Result<ClientConfig, Box<dyn std::error::Error>> {
    let crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(SkipServerVerification::new())
        .with_no_client_auth();

    let mut client_config = ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(crypto)?,
    ));
    client_config.transport_config(Arc::new(transport_config()));
    Ok(client_config)
}

fn transport_config() -> TransportConfig {
    let mut transport = TransportConfig::default();
    transport.datagram_receive_buffer_size(Some(1024 * 1024));
    transport.keep_alive_interval(Some(Duration::from_secs(4)));
    transport.max_idle_timeout(Some(
        IdleTimeout::try_from(Duration::from_secs(10)).expect("valid idle timeout"),
    ));
    transport
}

/// Dev-only: accepts any server certificate without validation.
#[derive(Debug)]
struct SkipServerVerification(Arc<rustls::crypto::CryptoProvider>);

impl SkipServerVerification {
    fn new() -> Arc<Self> {
        Arc::new(Self(Arc::new(rustls::crypto::ring::default_provider())))
    }
}

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
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
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
