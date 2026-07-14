use crate::{
    ED25519_SCHEME, Error, Result, Sha256, TLS_1_3, TLS_CHACHA20_POLY1305_SHA256,
    TLS_LEGACY_VERSION, X25519_GROUP, ct_eq, hkdf_expand_label, hkdf_extract, hmac_sha256, sha256,
    x25519,
};

const CLIENT_HELLO: u8 = 1;
const SERVER_HELLO: u8 = 2;
const ENCRYPTED_EXTENSIONS: u8 = 8;
const CERTIFICATE: u8 = 11;
const CERTIFICATE_VERIFY: u8 = 15;
const FINISHED: u8 = 20;
const EXT_SERVER_NAME: u16 = 0;
const EXT_SUPPORTED_GROUPS: u16 = 10;
const EXT_SIGNATURE_ALGORITHMS: u16 = 13;
const EXT_SUPPORTED_VERSIONS: u16 = 43;
const EXT_KEY_SHARE: u16 = 51;

struct Writer<'a> {
    out: &'a mut [u8],
    pos: usize,
}
impl<'a> Writer<'a> {
    fn new(out: &'a mut [u8]) -> Self {
        Self { out, pos: 0 }
    }
    fn put(&mut self, data: &[u8]) -> Result<()> {
        if self.out.len() - self.pos < data.len() {
            return Err(Error::BufferTooSmall);
        }
        self.out[self.pos..self.pos + data.len()].copy_from_slice(data);
        self.pos += data.len();
        Ok(())
    }
    fn u8(&mut self, v: u8) -> Result<()> {
        self.put(&[v])
    }
    fn u16(&mut self, v: u16) -> Result<()> {
        self.put(&v.to_be_bytes())
    }
    fn u24(&mut self, v: usize) -> Result<()> {
        if v > 0xff_ffff {
            return Err(Error::InvalidLength);
        }
        self.put(&[(v >> 16) as u8, (v >> 8) as u8, v as u8])
    }
    fn len(&self) -> usize {
        self.pos
    }
}
#[derive(Clone, Copy)]
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.data.len() - self.pos {
            return Err(Error::Decode);
        }
        let x = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(x)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        let x = self.take(2)?;
        Ok(u16::from_be_bytes([x[0], x[1]]))
    }
    fn u24(&mut self) -> Result<usize> {
        let x = self.take(3)?;
        Ok(((x[0] as usize) << 16) | ((x[1] as usize) << 8) | x[2] as usize)
    }
    fn done(&self) -> bool {
        self.pos == self.data.len()
    }
}
fn handshake_body(input: &[u8], kind: u8) -> Result<&[u8]> {
    let mut r = Reader::new(input);
    if r.u8()? != kind {
        return Err(Error::UnexpectedMessage);
    }
    let n = r.u24()?;
    let body = r.take(n)?;
    if !r.done() {
        return Err(Error::Decode);
    }
    Ok(body)
}

pub struct ClientHelloConfig<'a> {
    pub random: [u8; 32],
    pub session_id: &'a [u8],
    pub server_name: &'a [u8],
    pub x25519_public_key: [u8; 32],
}
#[derive(Debug, Eq, PartialEq)]
pub struct ClientHello<'a> {
    pub random: [u8; 32],
    pub session_id: &'a [u8],
    pub server_name: &'a [u8],
    pub x25519_public_key: [u8; 32],
}

fn extension(w: &mut Writer<'_>, kind: u16, data: &[u8]) -> Result<()> {
    w.u16(kind)?;
    w.u16(data.len() as u16)?;
    w.put(data)
}
pub fn build_client_hello(config: &ClientHelloConfig<'_>, output: &mut [u8]) -> Result<usize> {
    if config.session_id.len() > 32
        || config.server_name.is_empty()
        || config.server_name.len() > 253
    {
        return Err(Error::InvalidLength);
    }
    let mut body = [0u8; 1024];
    let mut w = Writer::new(&mut body);
    w.u16(TLS_LEGACY_VERSION)?;
    w.put(&config.random)?;
    w.u8(config.session_id.len() as u8)?;
    w.put(config.session_id)?;
    w.u16(2)?;
    w.u16(TLS_CHACHA20_POLY1305_SHA256)?;
    w.u8(1)?;
    w.u8(0)?;
    let ext_len_pos = w.len();
    w.u16(0)?;
    let ext_start = w.len();
    let mut sni = [0u8; 260];
    let sni_len = {
        let mut sw = Writer::new(&mut sni);
        sw.u16(3 + config.server_name.len() as u16)?;
        sw.u8(0)?;
        sw.u16(config.server_name.len() as u16)?;
        sw.put(config.server_name)?;
        sw.len()
    };
    extension(&mut w, EXT_SERVER_NAME, &sni[..sni_len])?;
    extension(&mut w, EXT_SUPPORTED_GROUPS, &[0, 2, 0, 0x1d])?;
    extension(&mut w, EXT_SIGNATURE_ALGORITHMS, &[0, 2, 0x08, 0x07])?;
    extension(&mut w, EXT_SUPPORTED_VERSIONS, &[2, 0x03, 0x04])?;
    let mut share = [0u8; 38];
    share[..2].copy_from_slice(&36u16.to_be_bytes());
    share[2..4].copy_from_slice(&X25519_GROUP.to_be_bytes());
    share[4..6].copy_from_slice(&32u16.to_be_bytes());
    share[6..].copy_from_slice(&config.x25519_public_key);
    extension(&mut w, EXT_KEY_SHARE, &share)?;
    let body_len = w.len();
    body[ext_len_pos..ext_len_pos + 2]
        .copy_from_slice(&((body_len - ext_start) as u16).to_be_bytes());
    let mut out = Writer::new(output);
    out.u8(CLIENT_HELLO)?;
    out.u24(body_len)?;
    out.put(&body[..body_len])?;
    Ok(out.len())
}

pub fn parse_client_hello(input: &[u8]) -> Result<ClientHello<'_>> {
    let body = handshake_body(input, CLIENT_HELLO)?;
    let mut r = Reader::new(body);
    if r.u16()? != TLS_LEGACY_VERSION {
        return Err(Error::UnsupportedVersion);
    }
    let mut random = [0; 32];
    random.copy_from_slice(r.take(32)?);
    let sid_len = r.u8()? as usize;
    if sid_len > 32 {
        return Err(Error::Decode);
    }
    let session_id = r.take(sid_len)?;
    let suites_len = r.u16()? as usize;
    if suites_len < 2 || suites_len & 1 != 0 {
        return Err(Error::Decode);
    }
    let mut suites = Reader::new(r.take(suites_len)?);
    let mut supported = false;
    while !suites.done() {
        supported |= suites.u16()? == TLS_CHACHA20_POLY1305_SHA256
    }
    if !supported {
        return Err(Error::UnsupportedCipher);
    }
    let compression_len = r.u8()? as usize;
    if compression_len == 0 || !r.take(compression_len)?.contains(&0) {
        return Err(Error::Decode);
    }
    let extensions_len = r.u16()? as usize;
    let mut exts = Reader::new(r.take(extensions_len)?);
    if !r.done() {
        return Err(Error::Decode);
    }
    let mut seen = 0u8;
    let mut name = None;
    let mut key = None;
    let mut version = false;
    while !exts.done() {
        let kind = exts.u16()?;
        let len = exts.u16()? as usize;
        let data = exts.take(len)?;
        match kind {
            EXT_SERVER_NAME => {
                if seen & 1 != 0 {
                    return Err(Error::Decode);
                }
                seen |= 1;
                let mut x = Reader::new(data);
                let list = x.u16()? as usize;
                let mut names = Reader::new(x.take(list)?);
                if !x.done() || names.u8()? != 0 {
                    return Err(Error::Decode);
                }
                let n = names.u16()? as usize;
                name = Some(names.take(n)?);
                if !names.done() {
                    return Err(Error::Decode);
                }
            }
            EXT_SUPPORTED_VERSIONS => {
                if seen & 2 != 0 {
                    return Err(Error::Decode);
                }
                seen |= 2;
                let mut x = Reader::new(data);
                let n = x.u8()? as usize;
                let mut versions = Reader::new(x.take(n)?);
                if !x.done() || n & 1 != 0 {
                    return Err(Error::Decode);
                }
                while !versions.done() {
                    version |= versions.u16()? == TLS_1_3
                }
            }
            EXT_KEY_SHARE => {
                if seen & 4 != 0 {
                    return Err(Error::Decode);
                }
                seen |= 4;
                let mut x = Reader::new(data);
                let n = x.u16()? as usize;
                let mut shares = Reader::new(x.take(n)?);
                if !x.done() {
                    return Err(Error::Decode);
                }
                while !shares.done() {
                    let group = shares.u16()?;
                    let l = shares.u16()? as usize;
                    let bytes = shares.take(l)?;
                    if group == X25519_GROUP {
                        if l != 32 || key.is_some() {
                            return Err(Error::Decode);
                        }
                        let mut p = [0; 32];
                        p.copy_from_slice(bytes);
                        key = Some(p)
                    }
                }
            }
            _ => {}
        }
    }
    if !version {
        return Err(Error::UnsupportedVersion);
    }
    Ok(ClientHello {
        random,
        session_id,
        server_name: name.ok_or(Error::Decode)?,
        x25519_public_key: key.ok_or(Error::UnsupportedGroup)?,
    })
}

#[derive(Debug, Eq, PartialEq)]
pub struct ServerHello<'a> {
    pub random: [u8; 32],
    pub session_id_echo: &'a [u8],
    pub x25519_public_key: [u8; 32],
}
pub fn parse_server_hello(input: &[u8]) -> Result<ServerHello<'_>> {
    let body = handshake_body(input, SERVER_HELLO)?;
    let mut r = Reader::new(body);
    if r.u16()? != TLS_LEGACY_VERSION {
        return Err(Error::UnsupportedVersion);
    }
    let mut random = [0; 32];
    random.copy_from_slice(r.take(32)?);
    let sid_len = r.u8()? as usize;
    if sid_len > 32 {
        return Err(Error::Decode);
    }
    let sid = r.take(sid_len)?;
    if r.u16()? != TLS_CHACHA20_POLY1305_SHA256 {
        return Err(Error::UnsupportedCipher);
    }
    if r.u8()? != 0 {
        return Err(Error::Decode);
    }
    let extensions_len = r.u16()? as usize;
    let mut exts = Reader::new(r.take(extensions_len)?);
    if !r.done() {
        return Err(Error::Decode);
    }
    let mut version = false;
    let mut key = None;
    let mut seen = 0u8;
    while !exts.done() {
        let kind = exts.u16()?;
        let extension_len = exts.u16()? as usize;
        let data = exts.take(extension_len)?;
        match kind {
            EXT_SUPPORTED_VERSIONS => {
                if seen & 1 != 0 {
                    return Err(Error::Decode);
                }
                seen |= 1;
                if data != TLS_1_3.to_be_bytes() {
                    return Err(Error::UnsupportedVersion);
                }
                version = true
            }
            EXT_KEY_SHARE => {
                if seen & 2 != 0 {
                    return Err(Error::Decode);
                }
                seen |= 2;
                let mut x = Reader::new(data);
                if x.u16()? != X25519_GROUP {
                    return Err(Error::UnsupportedGroup);
                }
                if x.u16()? != 32 {
                    return Err(Error::Decode);
                }
                let mut p = [0; 32];
                p.copy_from_slice(x.take(32)?);
                if !x.done() {
                    return Err(Error::Decode);
                }
                key = Some(p)
            }
            _ => {}
        }
    }
    if !version {
        return Err(Error::UnsupportedVersion);
    }
    Ok(ServerHello {
        random,
        session_id_echo: sid,
        x25519_public_key: key.ok_or(Error::UnsupportedGroup)?,
    })
}

pub struct CertificateMessage<'a> {
    context: &'a [u8],
    entries: &'a [u8],
}
impl<'a> CertificateMessage<'a> {
    pub fn context(&self) -> &'a [u8] {
        self.context
    }
    pub fn entries(&self) -> CertificateEntries<'a> {
        CertificateEntries {
            reader: Reader::new(self.entries),
            failed: false,
        }
    }
}
pub struct CertificateEntry<'a> {
    pub der: &'a [u8],
    pub extensions: &'a [u8],
}
pub struct CertificateEntries<'a> {
    reader: Reader<'a>,
    failed: bool,
}
impl<'a> Iterator for CertificateEntries<'a> {
    type Item = Result<CertificateEntry<'a>>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.reader.done() {
            return None;
        }
        let result = (|| {
            let n = self.reader.u24()?;
            if n == 0 {
                return Err(Error::InvalidCertificate);
            }
            let der = self.reader.take(n)?;
            let e = self.reader.u16()? as usize;
            let extensions = self.reader.take(e)?;
            Ok(CertificateEntry { der, extensions })
        })();
        if result.is_err() {
            self.failed = true
        }
        Some(result)
    }
}
pub fn parse_certificate(input: &[u8]) -> Result<CertificateMessage<'_>> {
    let body = handshake_body(input, CERTIFICATE)?;
    let mut r = Reader::new(body);
    let context_len = r.u8()? as usize;
    let context = r.take(context_len)?;
    let list_len = r.u24()?;
    let entries = r.take(list_len)?;
    if !r.done() || entries.is_empty() {
        return Err(Error::InvalidCertificate);
    }
    let message = CertificateMessage { context, entries };
    let mut count = 0;
    for entry in message.entries() {
        entry?;
        count += 1
    }
    if count == 0 {
        return Err(Error::InvalidCertificate);
    }
    Ok(message)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServerIdentity {
    pub verifier_handle: u32,
    pub leaf_sha256: [u8; 32],
}
pub trait TrustVerifier {
    fn verify_chain(
        &self,
        server_name: &[u8],
        unix_time: u64,
        chain: &CertificateMessage<'_>,
    ) -> Result<ServerIdentity>;
    fn verify_handshake_signature(
        &self,
        identity: &ServerIdentity,
        scheme: u16,
        signed_message: &[u8],
        signature: &[u8],
    ) -> Result<()>;
}

/// Builds the exact RFC 8446 input authenticated by a server CertificateVerify.
pub fn build_server_certificate_verify_input(
    transcript_hash: &[u8; 32],
    output: &mut [u8],
) -> Result<usize> {
    const CONTEXT: &[u8] = b"TLS 1.3, server CertificateVerify";
    let needed = 64 + CONTEXT.len() + 1 + 32;
    if output.len() < needed {
        return Err(Error::BufferTooSmall);
    }
    output[..64].fill(0x20);
    output[64..64 + CONTEXT.len()].copy_from_slice(CONTEXT);
    output[64 + CONTEXT.len()] = 0;
    output[65 + CONTEXT.len()..needed].copy_from_slice(transcript_hash);
    Ok(needed)
}

#[derive(Clone, Copy)]
pub struct KeySchedule {
    pub client_handshake_traffic: [u8; 32],
    pub server_handshake_traffic: [u8; 32],
    handshake_secret: [u8; 32],
    master_secret: [u8; 32],
    pub client_application_traffic: [u8; 32],
    pub server_application_traffic: [u8; 32],
}
impl KeySchedule {
    pub fn new(shared_secret: &[u8; 32], hello_hash: &[u8; 32]) -> Result<Self> {
        // RFC 8446 models an absent PSK as a Hash.length all-zero PSK.
        let early = hkdf_extract(&[0; 32], &[0; 32]);
        let mut derived = [0; 32];
        hkdf_expand_label(&early, b"derived", &sha256(&[]), &mut derived)?;
        let handshake_secret = hkdf_extract(&derived, shared_secret);
        let mut client = [0; 32];
        let mut server = [0; 32];
        hkdf_expand_label(&handshake_secret, b"c hs traffic", hello_hash, &mut client)?;
        hkdf_expand_label(&handshake_secret, b"s hs traffic", hello_hash, &mut server)?;
        let mut d = [0; 32];
        hkdf_expand_label(&handshake_secret, b"derived", &sha256(&[]), &mut d)?;
        let master_secret = hkdf_extract(&d, &[0; 32]);
        Ok(Self {
            client_handshake_traffic: client,
            server_handshake_traffic: server,
            handshake_secret,
            master_secret,
            client_application_traffic: [0; 32],
            server_application_traffic: [0; 32],
        })
    }
    pub fn derive_application(&mut self, transcript_hash: &[u8; 32]) -> Result<()> {
        hkdf_expand_label(
            &self.master_secret,
            b"c ap traffic",
            transcript_hash,
            &mut self.client_application_traffic,
        )?;
        hkdf_expand_label(
            &self.master_secret,
            b"s ap traffic",
            transcript_hash,
            &mut self.server_application_traffic,
        )
    }
    pub fn record_key_iv(traffic: &[u8; 32]) -> Result<([u8; 32], [u8; 12])> {
        let mut key = [0; 32];
        let mut iv = [0; 12];
        hkdf_expand_label(traffic, b"key", &[], &mut key)?;
        hkdf_expand_label(traffic, b"iv", &[], &mut iv)?;
        Ok((key, iv))
    }
    pub fn finished_key(traffic: &[u8; 32]) -> Result<[u8; 32]> {
        let mut key = [0; 32];
        hkdf_expand_label(traffic, b"finished", &[], &mut key)?;
        Ok(key)
    }
    pub fn handshake_secret(&self) -> &[u8; 32] {
        &self.handshake_secret
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandshakeState {
    Start,
    ClientHelloSent,
    ServerHelloReceived,
    EncryptedExtensionsReceived,
    CertificateReceived,
    CertificateVerified,
    CertificateVerifyReceived,
    Connected,
    Failed,
}
pub struct TlsClient {
    state: HandshakeState,
    transcript: Sha256,
    schedule: Option<KeySchedule>,
    identity: Option<ServerIdentity>,
}
impl TlsClient {
    pub const fn new() -> Self {
        Self {
            state: HandshakeState::Start,
            transcript: Sha256::new(),
            schedule: None,
            identity: None,
        }
    }
    pub fn state(&self) -> HandshakeState {
        self.state
    }
    pub fn start(&mut self, config: &ClientHelloConfig<'_>, out: &mut [u8]) -> Result<usize> {
        if self.state != HandshakeState::Start {
            return Err(Error::State);
        }
        let n = build_client_hello(config, out)?;
        self.transcript.update(&out[..n]);
        self.state = HandshakeState::ClientHelloSent;
        Ok(n)
    }
    pub fn receive_server_hello<'a>(
        &mut self,
        message: &'a [u8],
        private_key: &[u8; 32],
    ) -> Result<ServerHello<'a>> {
        if self.state != HandshakeState::ClientHelloSent {
            return Err(Error::State);
        }
        let hello = parse_server_hello(message)?;
        let shared = x25519(private_key, &hello.x25519_public_key)?;
        self.transcript.update(message);
        let hash = self.transcript.clone().finalize();
        self.schedule = Some(KeySchedule::new(&shared, &hash)?);
        self.state = HandshakeState::ServerHelloReceived;
        Ok(hello)
    }
    pub fn receive_encrypted_extensions(&mut self, message: &[u8]) -> Result<()> {
        if self.state != HandshakeState::ServerHelloReceived {
            return Err(Error::State);
        }
        handshake_body(message, ENCRYPTED_EXTENSIONS)?;
        self.transcript.update(message);
        self.state = HandshakeState::EncryptedExtensionsReceived;
        Ok(())
    }
    pub fn receive_certificate<V: TrustVerifier>(
        &mut self,
        message: &[u8],
        server_name: &[u8],
        unix_time: u64,
        verifier: &V,
    ) -> Result<()> {
        if self.state != HandshakeState::EncryptedExtensionsReceived {
            return Err(Error::State);
        }
        let cert = parse_certificate(message)?;
        self.state = HandshakeState::CertificateReceived;
        match verifier.verify_chain(server_name, unix_time, &cert) {
            Ok(identity) => {
                self.identity = Some(identity);
                self.transcript.update(message);
                self.state = HandshakeState::CertificateVerified;
                Ok(())
            }
            Err(e) => {
                self.state = HandshakeState::Failed;
                Err(e)
            }
        }
    }
    pub fn receive_certificate_verify<V: TrustVerifier>(
        &mut self,
        message: &[u8],
        verifier: &V,
    ) -> Result<()> {
        if self.state != HandshakeState::CertificateVerified {
            return Err(Error::State);
        }
        let body = handshake_body(message, CERTIFICATE_VERIFY)?;
        let mut r = Reader::new(body);
        let scheme = r.u16()?;
        if scheme != ED25519_SCHEME {
            return Err(Error::InvalidCertificate);
        }
        let signature_len = r.u16()? as usize;
        let signature = r.take(signature_len)?;
        if !r.done() {
            return Err(Error::Decode);
        }
        let hash = self.transcript.clone().finalize();
        let mut signed = [0u8; 130];
        let signed_len = build_server_certificate_verify_input(&hash, &mut signed)?;
        if let Err(e) = verifier.verify_handshake_signature(
            self.identity.as_ref().ok_or(Error::State)?,
            scheme,
            &signed[..signed_len],
            signature,
        ) {
            self.state = HandshakeState::Failed;
            return Err(e);
        }
        self.transcript.update(message);
        self.state = HandshakeState::CertificateVerifyReceived;
        Ok(())
    }
    pub fn receive_finished(&mut self, message: &[u8]) -> Result<()> {
        if self.state != HandshakeState::CertificateVerifyReceived {
            return Err(Error::State);
        }
        let verify = handshake_body(message, FINISHED)?;
        if verify.len() != 32 {
            return Err(Error::Decode);
        }
        let transcript = self.transcript.clone().finalize();
        let schedule = self.schedule.as_mut().ok_or(Error::State)?;
        let finished = KeySchedule::finished_key(&schedule.server_handshake_traffic)?;
        let expected = hmac_sha256(&finished, &transcript);
        if !ct_eq(verify, &expected) {
            self.state = HandshakeState::Failed;
            return Err(Error::InvalidTag);
        }
        self.transcript.update(message);
        schedule.derive_application(&self.transcript.clone().finalize())?;
        self.state = HandshakeState::Connected;
        Ok(())
    }
    pub fn key_schedule(&self) -> Option<&KeySchedule> {
        self.schedule.as_ref()
    }
}
impl Default for TlsClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex32(text: &str) -> [u8; 32] {
        let mut output = [0u8; 32];
        for (index, byte) in output.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).unwrap();
        }
        output
    }
    #[test]
    fn client_hello_roundtrip() {
        let cfg = ClientHelloConfig {
            random: [7; 32],
            session_id: &[1, 2, 3],
            server_name: b"nova.test",
            x25519_public_key: [9; 32],
        };
        let mut out = [0; 1024];
        let n = build_client_hello(&cfg, &mut out).unwrap();
        let parsed = parse_client_hello(&out[..n]).unwrap();
        assert_eq!(parsed.random, [7; 32]);
        assert_eq!(parsed.session_id, &[1, 2, 3]);
        assert_eq!(parsed.server_name, b"nova.test");
        assert_eq!(parsed.x25519_public_key, [9; 32]);
    }
    #[test]
    fn certificate_iterator_is_bounded() {
        let message = [11, 0, 0, 12, 0, 0, 0, 8, 0, 0, 3, 1, 2, 3, 0, 0];
        let cert = parse_certificate(&message).unwrap();
        let leaf = cert.entries().next().unwrap().unwrap();
        assert_eq!(leaf.der, &[1, 2, 3]);
        assert_eq!(leaf.extensions, &[])
    }
    #[test]
    fn invalid_transition() {
        let mut client = TlsClient::new();
        assert_eq!(
            client.receive_encrypted_extensions(&[8, 0, 0, 0]),
            Err(Error::State)
        );
    }
    #[test]
    fn rfc8448_handshake_traffic_secrets() {
        let shared = hex32("8bd4054fb55b9d63fdfbacf9f04b9f0d35e6d63f537563efd46272900f89492d");
        let transcript = hex32("860c06edc07858ee8e78f0e7428c58edd6b43f2ca3e6e95f02ed063cf0e1cad8");
        let schedule = KeySchedule::new(&shared, &transcript).unwrap();
        assert_eq!(
            schedule.client_handshake_traffic,
            hex32("b3eddb126e067f35a780b3abf45e2d8f3b1a950738f52e9600746a0e27a55a21")
        );
        assert_eq!(
            schedule.server_handshake_traffic,
            hex32("b67b7d690cc16c4e75e54213cb2d37b4e9c912bcded9105d42befd59d391ad38")
        );
    }
}
