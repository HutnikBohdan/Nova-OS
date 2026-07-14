#![no_std]
#![forbid(unsafe_code)]

//! Allocation-free TLS 1.3 primitives for Nova OS.
//!
//! This crate deliberately provides protocol and cryptographic mechanisms, not
//! policy. Callers supply entropy, persistent trust anchors, time, transport,
//! and certificate verification through [`TrustVerifier`].

mod aead;
mod handshake;
mod hash;
mod record;
mod x25519;

pub use aead::{open_in_place, seal_in_place};
pub use handshake::{
    CertificateEntry, CertificateMessage, ClientHello, ClientHelloConfig, HandshakeState,
    KeySchedule, ServerHello, ServerIdentity, TlsClient, TrustVerifier, build_client_hello,
    build_server_certificate_verify_input, parse_certificate, parse_client_hello,
    parse_server_hello,
};
pub use hash::{Sha256, hkdf_expand, hkdf_expand_label, hkdf_extract, hmac_sha256, sha256};
pub use record::{ContentType, RecordHeader, RecordProtector, decode_record, encode_record};
pub use x25519::{x25519, x25519_base};

pub const TLS_1_3: u16 = 0x0304;
pub const TLS_LEGACY_VERSION: u16 = 0x0303;
pub const TLS_AES_128_GCM_SHA256: u16 = 0x1301;
pub const TLS_CHACHA20_POLY1305_SHA256: u16 = 0x1303;
pub const X25519_GROUP: u16 = 0x001d;
pub const ED25519_SCHEME: u16 = 0x0807;
pub const MAX_RECORD_PLAINTEXT: usize = 16_384;
pub const MAX_RECORD_CIPHERTEXT: usize = MAX_RECORD_PLAINTEXT + 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    BufferTooSmall,
    Decode,
    UnexpectedMessage,
    UnsupportedVersion,
    UnsupportedCipher,
    UnsupportedGroup,
    InvalidLength,
    InvalidTag,
    InvalidPublicKey,
    InvalidCertificate,
    CertificateRejected,
    SequenceOverflow,
    State,
    OutputTooLarge,
}

pub type Result<T> = core::result::Result<T, Error>;

pub(crate) fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0u8;
    for index in 0..a.len() {
        difference |= a[index] ^ b[index];
    }
    difference == 0
}
