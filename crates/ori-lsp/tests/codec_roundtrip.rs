use std::io::Cursor;

use ori_lsp::codec::{read_message, write_message, MAX_PAYLOAD_BYTES};

#[test]
fn writes_then_reads_same_bytes() {
    let payload = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let mut buf: Vec<u8> = Vec::new();
    write_message(&mut buf, payload).expect("write");

    let mut reader = Cursor::new(buf);
    let read = read_message(&mut reader).expect("read").expect("payload");
    assert_eq!(read, payload);
}

#[test]
fn returns_none_on_clean_eof() {
    let mut reader = Cursor::new(Vec::<u8>::new());
    let read = read_message(&mut reader).expect("clean eof");
    assert!(read.is_none());
}

#[test]
fn rejects_missing_content_length() {
    let mut reader = Cursor::new(b"Content-Type: application/json\r\n\r\n".to_vec());
    let err = read_message(&mut reader).expect_err("must fail");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn rejects_oversized_payload() {
    let header = format!("Content-Length: {}\r\n\r\n", MAX_PAYLOAD_BYTES + 1);
    let mut reader = Cursor::new(header.into_bytes());
    let err = read_message(&mut reader).expect_err("must fail");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn rejects_malformed_header_line() {
    let mut reader = Cursor::new(b"GarbageLineWithoutColon\r\n\r\n".to_vec());
    let err = read_message(&mut reader).expect_err("must fail");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn rejects_invalid_content_length_value() {
    let mut reader = Cursor::new(b"Content-Length: not-a-number\r\n\r\n".to_vec());
    let err = read_message(&mut reader).expect_err("must fail");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn detects_short_payload() {
    let mut reader = Cursor::new(b"Content-Length: 10\r\n\r\nshort".to_vec());
    let err = read_message(&mut reader).expect_err("must fail");
    assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
}

#[test]
fn accepts_lone_lf_line_endings() {
    let mut reader = Cursor::new(b"Content-Length: 2\n\nhi".to_vec());
    let read = read_message(&mut reader).expect("read").expect("payload");
    assert_eq!(read, b"hi");
}
