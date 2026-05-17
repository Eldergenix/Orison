//! JSON-RPC over LSP base protocol framing.
//!
//! The LSP base protocol prefixes every payload with HTTP-style headers and a
//! blank line. Only `Content-Length` and `Content-Type` are defined by the
//! specification; everything else is ignored. This module implements the
//! framing without relying on any external crate.

use std::io::{self, BufRead, Write};

/// Hard cap on payload size to avoid runaway allocations when the peer sends a
/// malicious or corrupt `Content-Length`. 10 MiB is generous for source files
/// that ship inside `textDocument/didOpen` and `didChange` notifications.
pub const MAX_PAYLOAD_BYTES: usize = 10 * 1024 * 1024;

/// Reads a single Content-Length-framed message from `reader`.
///
/// Returns `Ok(None)` on a clean end-of-stream before any header bytes have
/// been read. Returns `Ok(Some(bytes))` with the raw payload otherwise.
/// Returns `Err` for malformed headers, oversized payloads, partial reads at
/// EOF, or unrecoverable I/O failures.
pub fn read_message<R: BufRead>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut content_length: Option<usize> = None;
    let mut saw_any_header = false;

    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            if saw_any_header {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "eof reached while reading lsp header",
                ));
            }
            return Ok(None);
        }
        saw_any_header = true;

        // Headers must terminate with CRLF per the LSP spec. Be permissive and
        // also accept lone LF so test pipes do not have to pretend to be HTTP.
        let trimmed = line.trim_end_matches(['\r', '\n']);

        if trimmed.is_empty() {
            break;
        }

        let Some((name, value)) = trimmed.split_once(':') else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("malformed lsp header line: {trimmed:?}"),
            ));
        };

        let name = name.trim();
        let value = value.trim();

        if name.eq_ignore_ascii_case("content-length") {
            let parsed = value.parse::<usize>().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid content-length value: {value:?}"),
                )
            })?;
            if parsed > MAX_PAYLOAD_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("content-length {parsed} exceeds maximum {MAX_PAYLOAD_BYTES}"),
                ));
            }
            content_length = Some(parsed);
        }
        // All other headers are spec-defined as ignorable for our subset.
    }

    let Some(length) = content_length else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing content-length header",
        ));
    };

    let mut buffer = vec![0u8; length];
    read_exact_or_eof(reader, &mut buffer)?;
    Ok(Some(buffer))
}

/// Writes a Content-Length-framed message to `writer` and flushes it.
pub fn write_message<W: Write>(writer: &mut W, payload: &[u8]) -> io::Result<()> {
    let header = format!("Content-Length: {}\r\n\r\n", payload.len());
    writer.write_all(header.as_bytes())?;
    writer.write_all(payload)?;
    writer.flush()
}

fn read_exact_or_eof<R: BufRead>(reader: &mut R, buf: &mut [u8]) -> io::Result<()> {
    let mut offset = 0usize;
    while offset < buf.len() {
        match reader.read(&mut buf[offset..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "eof reached while reading lsp payload",
                ));
            }
            Ok(n) => offset += n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        }
    }
    Ok(())
}
