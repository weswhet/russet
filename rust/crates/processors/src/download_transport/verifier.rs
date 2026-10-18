//! Server certificate verification against PEM roots, with the decisions and
//! messages of curl's OpenSSL backend.
//!
//! webpki rejects a server certificate that is a CA, even when that exact
//! certificate is a trusted root. OpenSSL trusts such a certificate as its own
//! one-certificate chain, which is how self-signed local servers are trusted
//! with `--cacert` or `SSL_CERT_FILE`. OpenSSL also reports an untrusted
//! self-signed certificate as self-signed before checking anything else.
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, Error, OtherError, SignatureScheme};
use std::sync::Arc;
use x509_cert::der::oid::{AssociatedOid, ObjectIdentifier};
use x509_cert::der::Decode;
use x509_cert::ext::pkix::{ExtendedKeyUsage, KeyUsage, KeyUsages};
use x509_cert::Certificate;

const SERVER_AUTH: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.1");
/// Netscape Server Gated Crypto, which OpenSSL accepts for TLS servers.
const NETSCAPE_SGC: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.113730.4.1");

/// A certificate problem curl reports with OpenSSL's wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rejection {
    SelfSigned,
    LocalIssuer,
    Expired,
    NotYetValid,
    Purpose,
}

impl Rejection {
    /// OpenSSL's `X509_verify_cert_error_string`. OpenSSL 3 hyphenates
    /// "self-signed"; LibreSSL and OpenSSL 1.1 don't.
    pub(crate) fn message(self, openssl3: bool) -> &'static str {
        match self {
            Rejection::SelfSigned if openssl3 => "self-signed certificate",
            Rejection::SelfSigned => "self signed certificate",
            Rejection::LocalIssuer => "unable to get local issuer certificate",
            Rejection::Expired => "certificate has expired",
            Rejection::NotYetValid => "certificate is not yet valid",
            Rejection::Purpose => "unsupported certificate purpose",
        }
    }
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message(false))
    }
}

impl std::error::Error for Rejection {}

fn reject(rejection: Rejection) -> Error {
    Error::InvalidCertificate(CertificateError::Other(OtherError(Arc::new(rejection))))
}

/// Verifies servers against a fixed set of roots like OpenSSL.
#[derive(Debug)]
pub(crate) struct Verifier {
    inner: Arc<WebPkiServerVerifier>,
    anchors: Vec<CertificateDer<'static>>,
}

impl Verifier {
    pub(crate) fn new(
        roots: rustls::RootCertStore,
        anchors: Vec<CertificateDer<'static>>,
        provider: Arc<rustls::crypto::CryptoProvider>,
    ) -> Result<Self, String> {
        let inner = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Verifier { inner, anchors })
    }
}

/// Whether a certificate names itself as its issuer, which OpenSSL treats as
/// self-signed when it can't be chained.
fn self_issued(certificate: &Certificate) -> bool {
    certificate.tbs_certificate.subject == certificate.tbs_certificate.issuer
}

/// OpenSSL's checks for a server certificate that is itself a trusted root:
/// its validity period, then its purpose, as `X509_PURPOSE_SSL_SERVER`.
fn trusted_root(certificate: &Certificate, now: UnixTime) -> Result<(), Rejection> {
    let validity = &certificate.tbs_certificate.validity;
    if now.as_secs() < validity.not_before.to_unix_duration().as_secs() {
        return Err(Rejection::NotYetValid);
    }
    if now.as_secs() > validity.not_after.to_unix_duration().as_secs() {
        return Err(Rejection::Expired);
    }
    for extension in certificate.tbs_certificate.extensions.iter().flatten() {
        let value = extension.extn_value.as_bytes();
        if extension.extn_id == ExtendedKeyUsage::OID {
            let usage = ExtendedKeyUsage::from_der(value).map_err(|_| Rejection::Purpose)?;
            if !usage
                .0
                .iter()
                .any(|oid| *oid == SERVER_AUTH || *oid == NETSCAPE_SGC)
            {
                return Err(Rejection::Purpose);
            }
        } else if extension.extn_id == KeyUsage::OID {
            let usage = KeyUsage::from_der(value).map_err(|_| Rejection::Purpose)?;
            let tls =
                KeyUsages::DigitalSignature | KeyUsages::KeyEncipherment | KeyUsages::KeyAgreement;
            if (usage.0 & tls).is_empty() {
                return Err(Rejection::Purpose);
            }
        }
    }
    Ok(())
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let error = match self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Ok(verified) => return Ok(verified),
            Err(error) => error,
        };
        let Ok(certificate) = Certificate::from_der(end_entity.as_ref()) else {
            return Err(error);
        };
        if self.anchors.iter().any(|anchor| anchor == end_entity) {
            trusted_root(&certificate, now).map_err(reject)?;
            let parsed = rustls::server::ParsedCertificate::try_from(end_entity)?;
            rustls::client::verify_server_name(&parsed, server_name)?;
            return Ok(ServerCertVerified::assertion());
        }
        if self_issued(&certificate) {
            return Err(reject(Rejection::SelfSigned));
        }
        match error {
            Error::InvalidCertificate(CertificateError::UnknownIssuer) => {
                Err(reject(Rejection::LocalIssuer))
            }
            error => Err(error),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn self_signed(
        name: &str,
        configure: impl FnOnce(&mut rcgen::CertificateParams),
    ) -> (
        rcgen::CertificateParams,
        rcgen::KeyPair,
        CertificateDer<'static>,
    ) {
        let key = rcgen::KeyPair::generate().unwrap();
        let mut params = rcgen::CertificateParams::new(vec![name.to_string()]).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, name);
        configure(&mut params);
        let der = params.self_signed(&key).unwrap().der().clone();
        (params, key, der)
    }

    fn verify(
        anchors: &[&CertificateDer<'static>],
        leaf: &CertificateDer<'static>,
        name: &str,
        now: UnixTime,
    ) -> Result<(), Error> {
        let mut roots = rustls::RootCertStore::empty();
        roots.add_parsable_certificates(anchors.iter().map(|c| (*c).clone()));
        let anchors = anchors.iter().map(|c| (*c).clone()).collect();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier = Verifier::new(roots, anchors, provider).unwrap();
        let name = ServerName::try_from(name.to_string()).unwrap();
        verifier
            .verify_server_cert(leaf, &[], &name, &[], now)
            .map(|_| ())
    }

    fn rejection(result: Result<(), Error>) -> Option<Rejection> {
        match result {
            Err(Error::InvalidCertificate(CertificateError::Other(other))) => {
                other.0.downcast_ref::<Rejection>().copied()
            }
            _ => None,
        }
    }

    #[test]
    fn a_trusted_self_signed_ca_serves_as_its_own_chain_like_openssl() {
        let (_, _, leaf) = self_signed("localhost", |_| {});
        assert!(verify(&[&leaf], &leaf, "localhost", UnixTime::now()).is_ok());
        // The host name is still checked.
        let wrong = verify(&[&leaf], &leaf, "example.invalid", UnixTime::now());
        assert!(wrong.is_err() && rejection(wrong).is_none());
        // So are its validity period and purpose.
        let later = UnixTime::since_unix_epoch(std::time::Duration::from_secs(1 << 40));
        let expired = verify(&[&leaf], &leaf, "localhost", later);
        assert_eq!(rejection(expired), Some(Rejection::Expired));
        let (_, _, client) = self_signed("localhost", |params| {
            params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];
        });
        let purpose = verify(&[&client], &client, "localhost", UnixTime::now());
        assert_eq!(rejection(purpose), Some(Rejection::Purpose));
    }

    #[test]
    fn untrusted_certificates_are_reported_with_openssls_reason() {
        let (_, _, other) = self_signed("other", |_| {});
        let (params, key, leaf) = self_signed("localhost", |_| {});
        let untrusted = verify(&[&other], &leaf, "localhost", UnixTime::now());
        assert_eq!(rejection(untrusted), Some(Rejection::SelfSigned));
        let issuer = rcgen::Issuer::from_params(&params, &key);
        let server = rcgen::CertificateParams::new(vec!["localhost".to_string()])
            .unwrap()
            .signed_by(&rcgen::KeyPair::generate().unwrap(), &issuer)
            .unwrap()
            .der()
            .clone();
        let unknown = verify(&[&other], &server, "localhost", UnixTime::now());
        assert_eq!(rejection(unknown), Some(Rejection::LocalIssuer));
        assert!(verify(&[&leaf], &server, "localhost", UnixTime::now()).is_ok());
    }

    #[test]
    fn openssl_3_hyphenates_self_signed() {
        assert_eq!(
            Rejection::SelfSigned.message(true),
            "self-signed certificate"
        );
        assert_eq!(
            Rejection::SelfSigned.message(false),
            "self signed certificate"
        );
    }
}
