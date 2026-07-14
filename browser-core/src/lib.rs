#![no_std]

pub mod pipeline;

#[cfg(test)]
extern crate std;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Nova,
    Http,
    Https,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Url<'a> {
    pub scheme: Scheme,
    pub host: &'a str,
    pub port: u16,
    pub path: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlError {
    MissingScheme,
    UnsupportedScheme,
    MissingHost,
    InvalidPort,
}

impl<'a> Url<'a> {
    pub fn parse(input: &'a str) -> Result<Self, UrlError> {
        let (scheme_text, rest) = input.split_once("://").ok_or(UrlError::MissingScheme)?;
        let (scheme, default_port) = if scheme_text.eq_ignore_ascii_case("nova") {
            (Scheme::Nova, 0)
        } else if scheme_text.eq_ignore_ascii_case("http") {
            (Scheme::Http, 80)
        } else if scheme_text.eq_ignore_ascii_case("https") {
            (Scheme::Https, 443)
        } else {
            return Err(UrlError::UnsupportedScheme);
        };
        let boundary = rest.find('/').unwrap_or(rest.len());
        let authority = &rest[..boundary];
        let path = if boundary < rest.len() {
            &rest[boundary..]
        } else {
            "/"
        };
        if authority.is_empty() {
            return Err(UrlError::MissingHost);
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) if !host.is_empty() => {
                let port = parse_port(port)?;
                (host, port)
            }
            _ => (authority, default_port),
        };
        Ok(Self {
            scheme,
            host,
            port,
            path,
        })
    }
}

fn parse_port(value: &str) -> Result<u16, UrlError> {
    let mut port = 0u16;
    if value.is_empty() {
        return Err(UrlError::InvalidPort);
    }
    for byte in value.bytes() {
        if !byte.is_ascii_digit() {
            return Err(UrlError::InvalidPort);
        }
        port = port
            .checked_mul(10)
            .and_then(|v| v.checked_add((byte - b'0') as u16))
            .ok_or(UrlError::InvalidPort)?;
    }
    Ok(port)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HtmlToken<'a> {
    StartTag(&'a str),
    EndTag(&'a str),
    Text(&'a str),
}

pub struct HtmlTokenizer<'a> {
    remaining: &'a str,
}

impl<'a> HtmlTokenizer<'a> {
    pub const fn new(html: &'a str) -> Self {
        Self { remaining: html }
    }
}

impl<'a> Iterator for HtmlTokenizer<'a> {
    type Item = HtmlToken<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining.is_empty() {
            return None;
        }
        if let Some(after_open) = self.remaining.strip_prefix('<') {
            let close = after_open.find('>')?;
            let raw = &after_open[..close];
            self.remaining = &after_open[close + 1..];
            if let Some(name) = raw.strip_prefix('/') {
                Some(HtmlToken::EndTag(name.trim()))
            } else {
                let name = raw.split_ascii_whitespace().next().unwrap_or("");
                Some(HtmlToken::StartTag(name))
            }
        } else {
            let next_tag = self.remaining.find('<').unwrap_or(self.remaining.len());
            let text = &self.remaining[..next_tag];
            self.remaining = &self.remaining[next_tag..];
            Some(HtmlToken::Text(text))
        }
    }
}

// HTTP/1.1 wire primitives. They deliberately borrow the receive buffer and use
// caller-owned output storage, so the same code runs in the kernel without a heap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Head,
    Post,
}

impl HttpMethod {
    const fn wire(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Head => "HEAD",
            Self::Post => "POST",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpHeader<'a> {
    pub name: &'a str,
    pub value: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpError {
    MalformedStatus,
    InvalidStatus,
    MalformedHeader,
    TooManyHeaders,
    OutputTooSmall,
}

pub struct HttpRequest<'a> {
    pub method: HttpMethod,
    pub host: &'a str,
    pub path: &'a str,
    pub body: &'a [u8],
}

impl<'a> HttpRequest<'a> {
    pub fn write_http11(&self, output: &mut [u8]) -> Result<usize, HttpError> {
        let mut writer = ByteWriter::new(output);
        writer.str(self.method.wire())?;
        writer.str(" ")?;
        writer.str(self.path)?;
        writer.str(" HTTP/1.1\r\nHost: ")?;
        writer.str(self.host)?;
        writer.str("\r\nConnection: close\r\n")?;
        if !self.body.is_empty() {
            writer.str("Content-Length: ")?;
            writer.decimal(self.body.len())?;
            writer.str("\r\n")?;
        }
        writer.str("\r\n")?;
        writer.bytes(self.body)?;
        Ok(writer.len)
    }
}

struct ByteWriter<'a> {
    output: &'a mut [u8],
    len: usize,
}

impl<'a> ByteWriter<'a> {
    fn new(output: &'a mut [u8]) -> Self {
        Self { output, len: 0 }
    }
    fn bytes(&mut self, value: &[u8]) -> Result<(), HttpError> {
        let end = self
            .len
            .checked_add(value.len())
            .ok_or(HttpError::OutputTooSmall)?;
        if end > self.output.len() {
            return Err(HttpError::OutputTooSmall);
        }
        self.output[self.len..end].copy_from_slice(value);
        self.len = end;
        Ok(())
    }
    fn str(&mut self, value: &str) -> Result<(), HttpError> {
        self.bytes(value.as_bytes())
    }
    fn decimal(&mut self, mut value: usize) -> Result<(), HttpError> {
        let mut digits = [0u8; 20];
        let mut start = digits.len();
        loop {
            start -= 1;
            digits[start] = b'0' + (value % 10) as u8;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        self.bytes(&digits[start..])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpResponse<'a, const HEADERS: usize> {
    pub status: u16,
    pub reason: &'a str,
    headers: [Option<HttpHeader<'a>>; HEADERS],
    header_count: usize,
    pub body: &'a [u8],
}

impl<'a, const HEADERS: usize> HttpResponse<'a, HEADERS> {
    pub fn parse(input: &'a [u8]) -> Result<Self, HttpError> {
        let text = core::str::from_utf8(input).map_err(|_| HttpError::MalformedStatus)?;
        let boundary = text.find("\r\n\r\n").ok_or(HttpError::MalformedHeader)?;
        let head = &text[..boundary];
        let mut lines = head.split("\r\n");
        let status_line = lines.next().ok_or(HttpError::MalformedStatus)?;
        let mut status_parts = status_line.splitn(3, ' ');
        if status_parts.next() != Some("HTTP/1.1") {
            return Err(HttpError::MalformedStatus);
        }
        let status_text = status_parts.next().ok_or(HttpError::MalformedStatus)?;
        let status = parse_status(status_text)?;
        let reason = status_parts.next().unwrap_or("");
        let mut headers = [None; HEADERS];
        let mut header_count = 0;
        for line in lines {
            let (name, value) = line.split_once(':').ok_or(HttpError::MalformedHeader)?;
            if name.is_empty() {
                return Err(HttpError::MalformedHeader);
            }
            if header_count == HEADERS {
                return Err(HttpError::TooManyHeaders);
            }
            headers[header_count] = Some(HttpHeader {
                name,
                value: value.trim(),
            });
            header_count += 1;
        }
        Ok(Self {
            status,
            reason,
            headers,
            header_count,
            body: &input[boundary + 4..],
        })
    }

    pub fn headers(&self) -> impl Iterator<Item = HttpHeader<'a>> + '_ {
        self.headers[..self.header_count]
            .iter()
            .filter_map(|header| *header)
    }

    pub fn header(&self, wanted: &str) -> Option<&'a str> {
        self.headers()
            .find(|header| header.name.eq_ignore_ascii_case(wanted))
            .map(|header| header.value)
    }
}

fn parse_status(value: &str) -> Result<u16, HttpError> {
    if value.len() != 3 {
        return Err(HttpError::InvalidStatus);
    }
    let mut status = 0u16;
    for byte in value.bytes() {
        if !byte.is_ascii_digit() {
            return Err(HttpError::InvalidStatus);
        }
        status = status * 10 + (byte - b'0') as u16;
    }
    Ok(status)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind<'a> {
    Document,
    Element(&'a str),
    Text(&'a str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomNode<'a> {
    pub kind: NodeKind<'a>,
    pub parent: Option<usize>,
    pub first_child: Option<usize>,
    pub next_sibling: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomError {
    Capacity,
    Depth,
    UnexpectedClose,
    UnclosedElement,
}

pub struct Document<'a, const NODES: usize> {
    nodes: [Option<DomNode<'a>>; NODES],
    len: usize,
}

impl<'a, const NODES: usize> Document<'a, NODES> {
    pub fn parse(html: &'a str) -> Result<Self, DomError> {
        let mut document = Self {
            nodes: [None; NODES],
            len: 0,
        };
        let root = document.push(NodeKind::Document, None)?;
        let mut stack = [0usize; NODES];
        let mut depth = 1usize;
        stack[0] = root;
        for token in HtmlTokenizer::new(html) {
            match token {
                HtmlToken::StartTag(tag) => {
                    if depth == NODES {
                        return Err(DomError::Depth);
                    }
                    let node = document.push(NodeKind::Element(tag), Some(stack[depth - 1]))?;
                    stack[depth] = node;
                    depth += 1;
                }
                HtmlToken::EndTag(tag) => {
                    if depth <= 1 {
                        return Err(DomError::UnexpectedClose);
                    }
                    let open = document
                        .node(stack[depth - 1])
                        .ok_or(DomError::UnexpectedClose)?;
                    if open.kind != NodeKind::Element(tag) {
                        return Err(DomError::UnexpectedClose);
                    }
                    depth -= 1;
                }
                HtmlToken::Text(text) if !text.is_empty() => {
                    document.push(NodeKind::Text(text), Some(stack[depth - 1]))?;
                }
                HtmlToken::Text(_) => {}
            }
        }
        if depth != 1 {
            return Err(DomError::UnclosedElement);
        }
        Ok(document)
    }

    fn push(&mut self, kind: NodeKind<'a>, parent: Option<usize>) -> Result<usize, DomError> {
        if self.len == NODES {
            return Err(DomError::Capacity);
        }
        let index = self.len;
        self.nodes[index] = Some(DomNode {
            kind,
            parent,
            first_child: None,
            next_sibling: None,
        });
        self.len += 1;
        if let Some(parent_index) = parent {
            let first = self.nodes[parent_index].and_then(|node| node.first_child);
            if let Some(mut sibling) = first {
                loop {
                    let next = self.nodes[sibling].and_then(|node| node.next_sibling);
                    if let Some(next) = next {
                        sibling = next;
                    } else {
                        break;
                    }
                }
                if let Some(node) = self.nodes[sibling].as_mut() {
                    node.next_sibling = Some(index);
                }
            } else if let Some(node) = self.nodes[parent_index].as_mut() {
                node.first_child = Some(index);
            }
        }
        Ok(index)
    }

    pub fn len(&self) -> usize {
        self.len
    }
    pub fn node(&self, index: usize) -> Option<&DomNode<'a>> {
        self.nodes.get(index)?.as_ref()
    }
    pub fn nodes(&self) -> impl Iterator<Item = (usize, &DomNode<'a>)> {
        self.nodes[..self.len]
            .iter()
            .enumerate()
            .filter_map(|(index, node)| node.as_ref().map(|node| (index, node)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CssProperty {
    Color,
    BackgroundColor,
    Width,
    Height,
    Margin,
    Padding,
    Display,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CssValue<'a> {
    Pixels(i32),
    Color(u32),
    Keyword(&'a str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CssDeclaration<'a> {
    pub property: CssProperty,
    pub value: CssValue<'a>,
}

pub struct CssDeclarations<'a> {
    remaining: &'a str,
}

impl<'a> CssDeclarations<'a> {
    pub const fn new(input: &'a str) -> Self {
        Self { remaining: input }
    }
}

impl<'a> Iterator for CssDeclarations<'a> {
    type Item = CssDeclaration<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let (raw, rest) = self
                .remaining
                .split_once(';')
                .unwrap_or((self.remaining, ""));
            self.remaining = rest;
            let (name, value) = match raw.split_once(':') {
                Some(parts) => parts,
                None if self.remaining.is_empty() => return None,
                None => continue,
            };
            let property = match name.trim() {
                "color" => CssProperty::Color,
                "background-color" => CssProperty::BackgroundColor,
                "width" => CssProperty::Width,
                "height" => CssProperty::Height,
                "margin" => CssProperty::Margin,
                "padding" => CssProperty::Padding,
                "display" => CssProperty::Display,
                _ => CssProperty::Unknown,
            };
            let value = value.trim();
            let parsed = if let Some(px) = value.strip_suffix("px") {
                parse_i32(px.trim())
                    .map(CssValue::Pixels)
                    .unwrap_or(CssValue::Keyword(value))
            } else if let Some(hex) = value.strip_prefix('#') {
                parse_color(hex)
                    .map(CssValue::Color)
                    .unwrap_or(CssValue::Keyword(value))
            } else {
                CssValue::Keyword(value)
            };
            return Some(CssDeclaration {
                property,
                value: parsed,
            });
        }
    }
}

fn parse_i32(value: &str) -> Option<i32> {
    let (negative, digits) = value
        .strip_prefix('-')
        .map_or((false, value), |rest| (true, rest));
    if digits.is_empty() {
        return None;
    }
    let mut number = 0i32;
    for byte in digits.bytes() {
        if !byte.is_ascii_digit() {
            return None;
        }
        number = number.checked_mul(10)?.checked_add((byte - b'0') as i32)?;
    }
    Some(if negative { -number } else { number })
}

fn parse_color(value: &str) -> Option<u32> {
    if value.len() != 6 {
        return None;
    }
    let mut color = 0u32;
    for byte in value.bytes() {
        color = color.checked_mul(16)?
            + match byte {
                b'0'..=b'9' => (byte - b'0') as u32,
                b'a'..=b'f' => (byte - b'a' + 10) as u32,
                b'A'..=b'F' => (byte - b'A' + 10) as u32,
                _ => return None,
            };
    }
    Some(0xff00_0000 | color)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockStyle {
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub margin: i32,
    pub padding: i32,
}

pub struct BlockFlow {
    containing: Rect,
    cursor_y: i32,
}

impl BlockFlow {
    pub const fn new(containing: Rect) -> Self {
        Self {
            cursor_y: containing.y,
            containing,
        }
    }
    pub fn place(&mut self, style: BlockStyle, intrinsic_height: i32) -> Rect {
        let x = self.containing.x + style.margin;
        let width = style
            .width
            .unwrap_or(self.containing.width - style.margin * 2)
            .max(0);
        let height = style
            .height
            .unwrap_or(intrinsic_height + style.padding * 2)
            .max(0);
        let y = self.cursor_y + style.margin;
        self.cursor_y = y + height + style.margin;
        Rect {
            x,
            y,
            width,
            height,
        }
    }
    pub const fn cursor_y(&self) -> i32 {
        self.cursor_y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_https_url_and_port() {
        let url = Url::parse("https://nova.example:8443/docs/start").unwrap();
        assert_eq!(url.scheme, Scheme::Https);
        assert_eq!(url.host, "nova.example");
        assert_eq!(url.port, 8443);
        assert_eq!(url.path, "/docs/start");
    }

    #[test]
    fn tokenizes_basic_html() {
        let tokens: std::vec::Vec<_> = HtmlTokenizer::new("<h1>Nova</h1>").collect();
        assert_eq!(
            tokens,
            [
                HtmlToken::StartTag("h1"),
                HtmlToken::Text("Nova"),
                HtmlToken::EndTag("h1")
            ]
        );
    }

    #[test]
    fn writes_request_without_allocation() {
        let request = HttpRequest {
            method: HttpMethod::Get,
            host: "nova.local",
            path: "/start",
            body: b"",
        };
        let mut bytes = [0u8; 128];
        let len = request.write_http11(&mut bytes).unwrap();
        assert_eq!(
            core::str::from_utf8(&bytes[..len]).unwrap(),
            "GET /start HTTP/1.1\r\nHost: nova.local\r\nConnection: close\r\n\r\n"
        );
    }

    #[test]
    fn parses_http_response_and_headers() {
        let response = HttpResponse::<4>::parse(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nX-Nova: так\r\n\r\n<h1>Nova</h1>"
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.header("content-type"), Some("text/html"));
        assert_eq!(response.body, b"<h1>Nova</h1>");
    }

    #[test]
    fn builds_fixed_capacity_dom_tree() {
        let document = Document::<12>::parse("<main><h1>Nova</h1><p>Привіт</p></main>").unwrap();
        assert_eq!(document.len(), 6);
        assert_eq!(document.node(0).unwrap().first_child, Some(1));
        assert_eq!(document.node(1).unwrap().first_child, Some(2));
        assert_eq!(document.node(2).unwrap().next_sibling, Some(4));
        assert_eq!(document.node(5).unwrap().kind, NodeKind::Text("Привіт"));
    }

    #[test]
    fn rejects_malformed_dom_nesting() {
        assert_eq!(
            Document::<8>::parse("<p><b>x</p></b>").err(),
            Some(DomError::UnexpectedClose)
        );
    }

    #[test]
    fn parses_css_values_and_places_blocks() {
        let declarations: std::vec::Vec<_> =
            CssDeclarations::new("color: #4f8cff; width: 320px; display: block").collect();
        assert_eq!(declarations[0].value, CssValue::Color(0xff4f8cff));
        assert_eq!(declarations[1].value, CssValue::Pixels(320));
        let mut flow = BlockFlow::new(Rect {
            x: 10,
            y: 20,
            width: 800,
            height: 600,
        });
        let first = flow.place(
            BlockStyle {
                width: None,
                height: Some(40),
                margin: 8,
                padding: 4,
            },
            0,
        );
        let second = flow.place(
            BlockStyle {
                width: Some(320),
                height: None,
                margin: 8,
                padding: 10,
            },
            24,
        );
        assert_eq!(
            first,
            Rect {
                x: 18,
                y: 28,
                width: 784,
                height: 40
            }
        );
        assert_eq!(
            second,
            Rect {
                x: 18,
                y: 84,
                width: 320,
                height: 44
            }
        );
    }
}
