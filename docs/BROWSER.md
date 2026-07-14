# Nova Browser

Nova Browser is a native Rust browser for Nova OS, not an embedded host browser.

## Verified now

- `no_std` URL parser for `nova://`, `http://` and `https://`
- explicit host, port and path parsing
- streaming HTML start-tag, end-tag and text tokenizer
- fixed-capacity DOM, CSS declarations and block layout engine
- own Ethernet/IPv4/UDP/TCP packet writers/parsers and retransmit primitives
- booted e1000 MMIO/DMA driver with DHCP bind and ARP
- real TCP three-way handshake and HTTP/1.1 response in QEMU (`NOVA_HTTP_RESPONSE_OK`)
- native Nova Browser shell at `F6`
- built-in `nova://start` page rendered by the framebuffer UI

## Not implemented yet

- persistent socket service and asynchronous driver interrupts
- external DNS runtime gate (DHCP/ARP work; the current QEMU DNS proxy did not answer)
- TLS certificate validation
- HTTP cache, redirects, compression and streaming body integration
- image codecs, forms, tabs, history and downloads
- JavaScript runtime and per-site sandbox

HTTP transport now passes an integration test against a separate server through
QEMU NAT. The browser remains unfinished until HTTPS/TLS, certificates, cache,
forms, media and sandboxed scripting pass their own booted-image gates.
