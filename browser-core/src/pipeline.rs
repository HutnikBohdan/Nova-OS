//! Allocation-free browser navigation and policy pipeline.

use crate::{Scheme, Url};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityError {
    Full,
    TooLong,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertificateError {
    UnknownIssuer,
    Expired,
    NotYetValid,
    NameMismatch,
    Revoked,
    WeakSignature,
    InvalidChain,
    PinMismatch,
    ClockUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    Dns,
    Connect,
    Timeout,
    Closed,
    Protocol,
    Certificate(CertificateError),
}

/// The TLS implementation owns cryptography. The browser only drives a byte stream.
pub trait BrowserTransport {
    fn open(&mut self, scheme: Scheme, host: &str, port: u16) -> Result<(), TransportError>;
    fn write(&mut self, bytes: &[u8]) -> Result<usize, TransportError>;
    fn read(&mut self, output: &mut [u8]) -> Result<usize, TransportError>;
    fn close(&mut self);
}

pub fn same_origin(a: Url<'_>, b: Url<'_>) -> bool {
    a.scheme == b.scheme && a.port == b.port && a.host.eq_ignore_ascii_case(b.host)
}

/// Writes an absolute normalized URL into caller storage. Host and scheme become
/// lower-case, default ports disappear, fragments are removed and dot segments collapse.
pub fn normalize_url(input: &str, output: &mut [u8]) -> Result<usize, CapacityError> {
    let url = Url::parse(input).map_err(|_| CapacityError::TooLong)?;
    let mut w = Writer::new(output);
    let scheme = match url.scheme {
        Scheme::Nova => "nova",
        Scheme::Http => "http",
        Scheme::Https => "https",
    };
    w.lower(scheme)?;
    w.put(b"://")?;
    w.lower(url.host)?;
    let default = matches!(
        (url.scheme, url.port),
        (Scheme::Nova, 0) | (Scheme::Http, 80) | (Scheme::Https, 443)
    );
    if !default {
        w.byte(b':')?;
        w.number(url.port as usize)?;
    }
    let path = url.path.split('#').next().unwrap_or("/");
    let (path, query) = path
        .split_once('?')
        .map_or((path, None), |(p, q)| (p, Some(q)));
    normalize_path(path, &mut w)?;
    if let Some(query) = query {
        w.byte(b'?')?;
        w.put(query.as_bytes())?;
    }
    Ok(w.len)
}

fn normalize_path(path: &str, w: &mut Writer<'_>) -> Result<(), CapacityError> {
    w.byte(b'/')?;
    let mut first = true;
    let mut segments: [&str; 32] = [""; 32];
    let mut count: usize = 0;
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => count = count.saturating_sub(1),
            _ => {
                if count == segments.len() {
                    return Err(CapacityError::Full);
                }
                segments[count] = part;
                count += 1;
            }
        }
    }
    for part in &segments[..count] {
        if !first {
            w.byte(b'/')?;
        }
        first = false;
        w.put(part.as_bytes())?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    Strict,
    Lax,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cookie<const TEXT: usize> {
    name: Text<TEXT>,
    value: Text<TEXT>,
    domain: Text<TEXT>,
    path: Text<TEXT>,
    expires: Option<u64>,
    secure: bool,
    http_only: bool,
    same_site: SameSite,
    host_only: bool,
}

pub struct CookieJar<const COUNT: usize, const TEXT: usize> {
    entries: [Option<Cookie<TEXT>>; COUNT],
}

impl<const COUNT: usize, const TEXT: usize> CookieJar<COUNT, TEXT> {
    pub const fn new() -> Self {
        Self {
            entries: [None; COUNT],
        }
    }

    pub fn set(&mut self, origin: Url<'_>, header: &str, now: u64) -> Result<(), CapacityError> {
        let mut fields = header.split(';');
        let (name, value) = fields
            .next()
            .and_then(|v| v.trim().split_once('='))
            .ok_or(CapacityError::TooLong)?;
        let mut cookie = Cookie {
            name: Text::new(name.trim())?,
            value: Text::new(value.trim())?,
            domain: Text::new(origin.host)?,
            path: Text::new(default_cookie_path(origin.path))?,
            expires: None,
            secure: false,
            http_only: false,
            same_site: SameSite::Lax,
            host_only: true,
        };
        for field in fields {
            let field = field.trim();
            let (key, value) = field.split_once('=').map_or((field, ""), |v| v);
            if key.eq_ignore_ascii_case("secure") {
                cookie.secure = true;
            } else if key.eq_ignore_ascii_case("httponly") {
                cookie.http_only = true;
            } else if key.eq_ignore_ascii_case("domain") {
                let domain = value.trim().trim_start_matches('.');
                if !domain_matches(origin.host, domain) {
                    return Ok(());
                }
                cookie.domain = Text::new(domain)?;
                cookie.host_only = false;
            } else if key.eq_ignore_ascii_case("path") && value.starts_with('/') {
                cookie.path = Text::new(value)?;
            } else if key.eq_ignore_ascii_case("max-age") {
                if let Some(age) = parse_u64(value.trim()) {
                    cookie.expires = Some(now.saturating_add(age));
                }
            } else if key.eq_ignore_ascii_case("samesite") {
                cookie.same_site = if value.eq_ignore_ascii_case("strict") {
                    SameSite::Strict
                } else if value.eq_ignore_ascii_case("none") {
                    SameSite::None
                } else {
                    SameSite::Lax
                };
            }
        }
        // SameSite=None without Secure is rejected by modern policy.
        if cookie.same_site == SameSite::None && !cookie.secure {
            return Ok(());
        }
        for entry in &mut self.entries {
            if let Some(old) = entry {
                if old.name == cookie.name && old.domain == cookie.domain && old.path == cookie.path
                {
                    *entry = Some(cookie);
                    return Ok(());
                }
            }
        }
        let slot = self
            .entries
            .iter_mut()
            .find(|e| e.is_none())
            .ok_or(CapacityError::Full)?;
        *slot = Some(cookie);
        Ok(())
    }

    pub fn write_request(
        &self,
        target: Url<'_>,
        top: Url<'_>,
        top_level_safe: bool,
        now: u64,
        output: &mut [u8],
    ) -> Result<usize, CapacityError> {
        let mut w = Writer::new(output);
        let mut first = true;
        for cookie in self.entries.iter().flatten() {
            if cookie.expires.is_some_and(|e| e <= now)
                || (cookie.secure && target.scheme != Scheme::Https)
            {
                continue;
            }
            let host_ok = if cookie.host_only {
                target.host.eq_ignore_ascii_case(cookie.domain.as_str())
            } else {
                domain_matches(target.host, cookie.domain.as_str())
            };
            if !host_ok || !target.path.starts_with(cookie.path.as_str()) {
                continue;
            }
            let cross = !same_origin(target, top);
            if cross
                && (cookie.same_site == SameSite::Strict
                    || (cookie.same_site == SameSite::Lax && !top_level_safe))
            {
                continue;
            }
            if !first {
                w.put(b"; ")?;
            }
            first = false;
            w.put(cookie.name.bytes())?;
            w.byte(b'=')?;
            w.put(cookie.value.bytes())?;
        }
        Ok(w.len)
    }
}

impl<const COUNT: usize, const TEXT: usize> Default for CookieJar<COUNT, TEXT> {
    fn default() -> Self {
        Self::new()
    }
}

fn default_cookie_path(path: &str) -> &str {
    let end = path.rfind('/').unwrap_or(0);
    if end == 0 { "/" } else { &path[..end] }
}
fn domain_matches(host: &str, domain: &str) -> bool {
    host.eq_ignore_ascii_case(domain)
        || (host.len() > domain.len()
            && host.as_bytes()[host.len() - domain.len() - 1] == b'.'
            && host[host.len() - domain.len()..].eq_ignore_ascii_case(domain))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheMetadata<const TEXT: usize> {
    pub url: Text<TEXT>,
    pub vary: Text<TEXT>,
    pub etag: Text<TEXT>,
    pub expires: u64,
    pub last_modified: u64,
    pub body_len: usize,
}
impl<const TEXT: usize> CacheMetadata<TEXT> {
    pub fn fresh(&self, now: u64) -> bool {
        now < self.expires
    }
    pub fn matches(&self, url: &str, vary_key: &str) -> bool {
        self.url.as_str() == url && self.vary.as_str() == vary_key
    }
}

pub struct CacheIndex<const COUNT: usize, const TEXT: usize> {
    entries: [Option<CacheMetadata<TEXT>>; COUNT],
    hand: usize,
}
impl<const COUNT: usize, const TEXT: usize> CacheIndex<COUNT, TEXT> {
    pub const fn new() -> Self {
        Self {
            entries: [None; COUNT],
            hand: 0,
        }
    }
    pub fn lookup(&self, url: &str, vary: &str) -> Option<&CacheMetadata<TEXT>> {
        self.entries.iter().flatten().find(|e| e.matches(url, vary))
    }
    pub fn insert(&mut self, value: CacheMetadata<TEXT>) {
        self.entries[self.hand] = Some(value);
        self.hand = (self.hand + 1) % COUNT.max(1);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Document,
    Script,
    Style,
    Image,
    Font,
    Connect,
}

pub struct ContentSecurityPolicy<'a> {
    raw: &'a str,
}
impl<'a> ContentSecurityPolicy<'a> {
    pub const fn parse(raw: &'a str) -> Self {
        Self { raw }
    }
    pub fn allows(&self, kind: ResourceKind, document: Url<'_>, target: Url<'_>) -> bool {
        let directive = match kind {
            ResourceKind::Script => "script-src",
            ResourceKind::Style => "style-src",
            ResourceKind::Image => "img-src",
            ResourceKind::Font => "font-src",
            ResourceKind::Connect => "connect-src",
            ResourceKind::Document => "navigate-to",
        };
        let values = self
            .directive(directive)
            .or_else(|| self.directive("default-src"));
        let Some(values) = values else {
            return true;
        };
        for token in values.split_ascii_whitespace() {
            if token == "'none'" {
                return false;
            }
            if token == "*" || (token == "'self'" && same_origin(document, target)) {
                return true;
            }
            if let Ok(source) = Url::parse(token) {
                if same_origin(source, target) {
                    return true;
                }
            }
        }
        false
    }
    fn directive(&self, wanted: &str) -> Option<&'a str> {
        self.raw.split(';').find_map(|part| {
            let part = part.trim();
            let (name, rest) = part.split_once(char::is_whitespace).unwrap_or((part, ""));
            name.eq_ignore_ascii_case(wanted).then_some(rest.trim())
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyMode {
    None,
    ContentLength(usize),
    Chunked,
    UntilClose,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEvent<'a> {
    Headers { status: u16, mode: BodyMode },
    Body(&'a [u8]),
    Complete,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamError {
    HeaderTooLarge,
    Malformed,
    InvalidLength,
    UnexpectedEof,
}

pub struct HttpStream<const HEAD: usize> {
    head: [u8; HEAD],
    head_len: usize,
    mode: Option<BodyMode>,
    remaining: usize,
    chunk_remaining: usize,
    chunk_line: [u8; 16],
    chunk_line_len: usize,
    complete: bool,
}
impl<const HEAD: usize> HttpStream<HEAD> {
    pub const fn new() -> Self {
        Self {
            head: [0; HEAD],
            head_len: 0,
            mode: None,
            remaining: 0,
            chunk_remaining: 0,
            chunk_line: [0; 16],
            chunk_line_len: 0,
            complete: false,
        }
    }
    /// Consumes at most one semantic event, allowing callers to provide backpressure.
    pub fn feed<'a>(
        &mut self,
        input: &'a [u8],
    ) -> Result<(usize, Option<StreamEvent<'a>>), StreamError> {
        if self.complete {
            return Ok((0, Some(StreamEvent::Complete)));
        }
        if self.mode.is_none() {
            let mut used = 0;
            for &b in input {
                if self.head_len == HEAD {
                    return Err(StreamError::HeaderTooLarge);
                }
                self.head[self.head_len] = b;
                self.head_len += 1;
                used += 1;
                if self.head[..self.head_len].ends_with(b"\r\n\r\n") {
                    let (status, mode) = parse_head(&self.head[..self.head_len])?;
                    self.mode = Some(mode);
                    if let BodyMode::ContentLength(n) = mode {
                        self.remaining = n;
                    }
                    if mode == BodyMode::None || mode == BodyMode::ContentLength(0) {
                        self.complete = true;
                    }
                    return Ok((used, Some(StreamEvent::Headers { status, mode })));
                }
            }
            return Ok((used, None));
        }
        match self.mode.unwrap() {
            BodyMode::None => {
                self.complete = true;
                Ok((0, Some(StreamEvent::Complete)))
            }
            BodyMode::UntilClose => {
                if input.is_empty() {
                    Ok((0, None))
                } else {
                    Ok((input.len(), Some(StreamEvent::Body(input))))
                }
            }
            BodyMode::ContentLength(_) => {
                if self.remaining == 0 {
                    self.complete = true;
                    return Ok((0, Some(StreamEvent::Complete)));
                }
                let n = self.remaining.min(input.len());
                self.remaining -= n;
                Ok((n, (n > 0).then_some(StreamEvent::Body(&input[..n]))))
            }
            BodyMode::Chunked => self.feed_chunked(input),
        }
    }
    fn feed_chunked<'a>(
        &mut self,
        input: &'a [u8],
    ) -> Result<(usize, Option<StreamEvent<'a>>), StreamError> {
        if self.chunk_remaining > 0 {
            let n = self.chunk_remaining.min(input.len());
            self.chunk_remaining -= n;
            return Ok((n, (n > 0).then_some(StreamEvent::Body(&input[..n]))));
        }
        let mut used = 0;
        // consume CRLF after a chunk
        let mut data = input;
        if self.chunk_line_len == usize::MAX {
            if data.len() < 2 {
                return Ok((0, None));
            }
            if &data[..2] != b"\r\n" {
                return Err(StreamError::Malformed);
            }
            used = 2;
            data = &data[2..];
            self.chunk_line_len = 0;
        }
        for &b in data {
            used += 1;
            if b == b'\n' {
                if self.chunk_line_len == 0 || self.chunk_line[self.chunk_line_len - 1] != b'\r' {
                    return Err(StreamError::Malformed);
                }
                let size = parse_hex(&self.chunk_line[..self.chunk_line_len - 1])?;
                self.chunk_line_len = if size == 0 { 0 } else { usize::MAX };
                self.chunk_remaining = size;
                if size == 0 {
                    self.complete = true;
                    return Ok((used, Some(StreamEvent::Complete)));
                }
                return Ok((used, None));
            }
            if self.chunk_line_len >= self.chunk_line.len() {
                return Err(StreamError::Malformed);
            }
            self.chunk_line[self.chunk_line_len] = b;
            self.chunk_line_len += 1;
        }
        Ok((used, None))
    }
    pub fn eof(&mut self) -> Result<Option<StreamEvent<'static>>, StreamError> {
        if self.mode == Some(BodyMode::UntilClose) {
            self.complete = true;
            Ok(Some(StreamEvent::Complete))
        } else if self.complete {
            Ok(Some(StreamEvent::Complete))
        } else {
            Err(StreamError::UnexpectedEof)
        }
    }
}

fn parse_head(head: &[u8]) -> Result<(u16, BodyMode), StreamError> {
    let text = core::str::from_utf8(head).map_err(|_| StreamError::Malformed)?;
    let mut lines = text.split("\r\n");
    let status_line = lines.next().ok_or(StreamError::Malformed)?;
    let status = status_line
        .split_ascii_whitespace()
        .nth(1)
        .and_then(parse_u64)
        .filter(|s| *s <= 999)
        .ok_or(StreamError::Malformed)? as u16;
    if status / 100 == 1 || status == 204 || status == 304 {
        return Ok((status, BodyMode::None));
    }
    let mut length = None;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            length = Some(parse_u64(value.trim()).ok_or(StreamError::InvalidLength)? as usize);
        }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            chunked = value
                .split(',')
                .any(|v| v.trim().eq_ignore_ascii_case("chunked"));
        }
    }
    Ok((
        status,
        if chunked {
            BodyMode::Chunked
        } else if let Some(n) = length {
            BodyMode::ContentLength(n)
        } else {
            BodyMode::UntilClose
        },
    ))
}
fn parse_hex(bytes: &[u8]) -> Result<usize, StreamError> {
    let mut n = 0usize;
    for &b in bytes.split(|b| *b == b';').next().unwrap_or(bytes) {
        let v = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => return Err(StreamError::Malformed),
        };
        n = n
            .checked_mul(16)
            .and_then(|x| x.checked_add(v as usize))
            .ok_or(StreamError::InvalidLength)?;
    }
    Ok(n)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationState {
    Idle,
    Resolving,
    Connecting,
    Sending,
    ReceivingHeaders,
    ReceivingBody,
    Rendering,
    Complete,
    Failed(TransportError),
}
pub struct Navigator {
    state: NavigationState,
    redirects: u8,
    max_redirects: u8,
}
impl Navigator {
    pub const fn new(max_redirects: u8) -> Self {
        Self {
            state: NavigationState::Idle,
            redirects: 0,
            max_redirects,
        }
    }
    pub const fn state(&self) -> NavigationState {
        self.state
    }
    pub fn start(&mut self) {
        self.redirects = 0;
        self.state = NavigationState::Resolving;
    }
    pub fn resolved(&mut self) {
        if self.state == NavigationState::Resolving {
            self.state = NavigationState::Connecting;
        }
    }
    pub fn connected(&mut self) {
        if self.state == NavigationState::Connecting {
            self.state = NavigationState::Sending;
        }
    }
    pub fn request_sent(&mut self) {
        if self.state == NavigationState::Sending {
            self.state = NavigationState::ReceivingHeaders;
        }
    }
    pub fn headers(&mut self, status: u16) -> bool {
        if self.state != NavigationState::ReceivingHeaders {
            return false;
        }
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            if self.redirects >= self.max_redirects {
                self.state = NavigationState::Failed(TransportError::Protocol);
                false
            } else {
                self.redirects += 1;
                self.state = NavigationState::Resolving;
                true
            }
        } else {
            self.state = NavigationState::ReceivingBody;
            false
        }
    }
    pub fn body_complete(&mut self) {
        if self.state == NavigationState::ReceivingBody {
            self.state = NavigationState::Rendering;
        }
    }
    pub fn rendered(&mut self) {
        if self.state == NavigationState::Rendering {
            self.state = NavigationState::Complete;
        }
    }
    pub fn fail(&mut self, error: TransportError) {
        self.state = NavigationState::Failed(error);
    }
}

pub struct History<const COUNT: usize, const TEXT: usize> {
    entries: [Option<Text<TEXT>>; COUNT],
    len: usize,
    current: usize,
}
impl<const COUNT: usize, const TEXT: usize> History<COUNT, TEXT> {
    pub const fn new() -> Self {
        Self {
            entries: [None; COUNT],
            len: 0,
            current: 0,
        }
    }
    pub fn push(&mut self, url: &str) -> Result<(), CapacityError> {
        let text = Text::new(url)?;
        if self.len == COUNT {
            self.entries.rotate_left(1);
            self.entries[COUNT - 1] = Some(text);
            self.current = COUNT - 1;
        } else {
            if self.len > 0 {
                self.current += 1;
            }
            self.entries[self.current] = Some(text);
            self.len = self.current + 1;
        }
        Ok(())
    }
    pub fn back(&mut self) -> Option<&str> {
        if self.current == 0 {
            None
        } else {
            self.current -= 1;
            self.current()
        }
    }
    pub fn forward(&mut self) -> Option<&str> {
        if self.current + 1 >= self.len {
            None
        } else {
            self.current += 1;
            self.current()
        }
    }
    pub fn current(&self) -> Option<&str> {
        self.entries.get(self.current)?.as_ref().map(Text::as_str)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Text<const N: usize> {
    bytes: [u8; N],
    len: usize,
}
impl<const N: usize> Text<N> {
    pub fn new(value: &str) -> Result<Self, CapacityError> {
        if value.len() > N {
            return Err(CapacityError::TooLong);
        }
        let mut bytes = [0; N];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self {
            bytes,
            len: value.len(),
        })
    }
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(self.bytes()).unwrap_or("")
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

struct Writer<'a> {
    out: &'a mut [u8],
    len: usize,
}
impl<'a> Writer<'a> {
    fn new(out: &'a mut [u8]) -> Self {
        Self { out, len: 0 }
    }
    fn put(&mut self, bytes: &[u8]) -> Result<(), CapacityError> {
        let end = self
            .len
            .checked_add(bytes.len())
            .ok_or(CapacityError::TooLong)?;
        if end > self.out.len() {
            return Err(CapacityError::TooLong);
        }
        self.out[self.len..end].copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }
    fn byte(&mut self, b: u8) -> Result<(), CapacityError> {
        self.put(&[b])
    }
    fn lower(&mut self, s: &str) -> Result<(), CapacityError> {
        for b in s.bytes() {
            self.byte(b.to_ascii_lowercase())?
        }
        Ok(())
    }
    fn number(&mut self, mut n: usize) -> Result<(), CapacityError> {
        let mut b = [0; 20];
        let mut p = 20;
        loop {
            p -= 1;
            b[p] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        self.put(&b[p..])
    }
}
fn parse_u64(v: &str) -> Option<u64> {
    if v.is_empty() {
        return None;
    }
    let mut n = 0u64;
    for b in v.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as u64)?
    }
    Some(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalizes_urls() {
        let mut out = [0; 128];
        let n = normalize_url("HTTPS://Example.COM:443/a/./b/../c?q=1#x", &mut out).unwrap();
        assert_eq!(
            core::str::from_utf8(&out[..n]).unwrap(),
            "https://example.com/a/c?q=1"
        );
    }
    #[test]
    fn cookies_apply_security_and_scope() {
        let origin = Url::parse("https://app.example/account/login").unwrap();
        let mut jar = CookieJar::<4, 32>::new();
        jar.set(
            origin,
            "sid=abc; Secure; HttpOnly; SameSite=Strict; Path=/account",
            10,
        )
        .unwrap();
        let mut out = [0; 64];
        let n = jar
            .write_request(
                Url::parse("https://app.example/account/me").unwrap(),
                origin,
                true,
                11,
                &mut out,
            )
            .unwrap();
        assert_eq!(&out[..n], b"sid=abc");
        let n = jar
            .write_request(
                Url::parse("http://app.example/account/me").unwrap(),
                origin,
                true,
                11,
                &mut out,
            )
            .unwrap();
        assert_eq!(n, 0);
    }
    #[test]
    fn rejects_insecure_none_cookie() {
        let o = Url::parse("http://a.test/").unwrap();
        let mut j = CookieJar::<2, 16>::new();
        j.set(o, "x=y; SameSite=None", 0).unwrap();
        let mut b = [0; 16];
        assert_eq!(j.write_request(o, o, true, 0, &mut b).unwrap(), 0);
    }
    #[test]
    fn cache_varies_and_expires() {
        let mut c = CacheIndex::<2, 48>::new();
        c.insert(CacheMetadata {
            url: Text::new("https://a/").unwrap(),
            vary: Text::new("accept=html").unwrap(),
            etag: Text::new("v1").unwrap(),
            expires: 20,
            last_modified: 2,
            body_len: 9,
        });
        let e = c.lookup("https://a/", "accept=html").unwrap();
        assert!(e.fresh(19));
        assert!(!e.fresh(20));
        assert!(c.lookup("https://a/", "accept=json").is_none());
    }
    #[test]
    fn csp_enforces_self() {
        let p = ContentSecurityPolicy::parse("default-src 'none'; img-src 'self' https://cdn.test");
        let d = Url::parse("https://nova.test/a").unwrap();
        assert!(p.allows(
            ResourceKind::Image,
            d,
            Url::parse("https://nova.test/i").unwrap()
        ));
        assert!(p.allows(
            ResourceKind::Image,
            d,
            Url::parse("https://cdn.test/i").unwrap()
        ));
        assert!(!p.allows(ResourceKind::Script, d, d));
    }
    #[test]
    fn streams_content_length_across_packets() {
        let mut s = HttpStream::<128>::new();
        let h = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n";
        let (_, e) = s.feed(h).unwrap();
        assert_eq!(
            e,
            Some(StreamEvent::Headers {
                status: 200,
                mode: BodyMode::ContentLength(5)
            })
        );
        let (_, e) = s.feed(b"he").unwrap();
        assert_eq!(e, Some(StreamEvent::Body(b"he")));
        let (_, e) = s.feed(b"lloEXTRA").unwrap();
        assert_eq!(e, Some(StreamEvent::Body(b"llo")));
        assert_eq!(s.feed(b"").unwrap().1, Some(StreamEvent::Complete));
    }
    #[test]
    fn streams_chunked_body() {
        let mut s = HttpStream::<128>::new();
        s.feed(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
            .unwrap();
        assert_eq!(s.feed(b"4\r\n").unwrap(), (3, None));
        assert_eq!(s.feed(b"Wiki").unwrap().1, Some(StreamEvent::Body(b"Wiki")));
        assert_eq!(s.feed(b"\r\n5\r\n").unwrap().0, 5);
        assert_eq!(
            s.feed(b"pedia").unwrap().1,
            Some(StreamEvent::Body(b"pedia"))
        );
        assert_eq!(s.feed(b"\r\n0\r\n").unwrap().1, Some(StreamEvent::Complete));
    }
    #[test]
    fn navigation_limits_redirects() {
        let mut n = Navigator::new(1);
        n.start();
        n.resolved();
        n.connected();
        n.request_sent();
        assert!(n.headers(302));
        n.resolved();
        n.connected();
        n.request_sent();
        assert!(!n.headers(302));
        assert_eq!(n.state(), NavigationState::Failed(TransportError::Protocol));
    }
    #[test]
    fn history_is_bounded() {
        let mut h = History::<3, 32>::new();
        for u in ["https://a/", "https://b/", "https://c/", "https://d/"] {
            h.push(u).unwrap();
        }
        assert_eq!(h.current(), Some("https://d/"));
        assert_eq!(h.back(), Some("https://c/"));
        assert_eq!(h.back(), Some("https://b/"));
        assert_eq!(h.back(), None);
        assert_eq!(h.forward(), Some("https://c/"));
    }
}
