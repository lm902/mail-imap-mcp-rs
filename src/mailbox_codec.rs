//! IMAP mailbox name codec (modified UTF-7)
//!
//! IMAP mailbox names on the wire use modified UTF-7. This module converts
//! between user-facing UTF-8 strings and wire-format mailbox names.

use base64::Engine;

use crate::errors::{AppError, AppResult};

const BASE64_ENGINE: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// Encode UTF-8 mailbox name to IMAP modified UTF-7 wire format.
pub fn encode_mailbox_name(input: &str) -> AppResult<String> {
    let mut out = String::with_capacity(input.len());
    let mut utf16_chunk = Vec::new();

    for ch in input.chars() {
        if is_direct_char(ch) {
            flush_encoded_chunk(&mut out, &mut utf16_chunk)?;
            if ch == '&' {
                out.push_str("&-");
            } else {
                out.push(ch);
            }
        } else {
            let mut encoded = [0_u16; 2];
            let units = ch.encode_utf16(&mut encoded);
            utf16_chunk.extend(units.iter().copied());
        }
    }

    flush_encoded_chunk(&mut out, &mut utf16_chunk)?;
    Ok(out)
}

/// Decode IMAP modified UTF-7 wire-format mailbox name to UTF-8.
pub fn decode_mailbox_name(input: &str) -> AppResult<String> {
    let bytes = input.as_bytes();
    let mut i = 0usize;
    let mut out = String::with_capacity(input.len());

    while i < bytes.len() {
        if bytes[i] != b'&' {
            out.push(bytes[i] as char);
            i += 1;
            continue;
        }

        i += 1;
        let start = i;
        while i < bytes.len() && bytes[i] != b'-' {
            i += 1;
        }
        if i >= bytes.len() {
            return Err(AppError::InvalidInput(
                "invalid IMAP mailbox encoding: unterminated shift sequence".to_owned(),
            ));
        }

        if i == start {
            out.push('&');
            i += 1;
            continue;
        }

        let mut b64 = input[start..i].replace(',', "/");
        let rem = b64.len() % 4;
        if rem != 0 {
            b64.push_str(&"=".repeat(4 - rem));
        }
        let raw = BASE64_ENGINE.decode(b64.as_bytes()).map_err(|_| {
            AppError::InvalidInput("invalid IMAP mailbox encoding: bad base64".to_owned())
        })?;
        if raw.len() % 2 != 0 {
            return Err(AppError::InvalidInput(
                "invalid IMAP mailbox encoding: odd UTF-16 byte length".to_owned(),
            ));
        }
        let utf16: Vec<u16> = raw
            .chunks_exact(2)
            .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
            .collect();
        let decoded = String::from_utf16(&utf16).map_err(|_| {
            AppError::InvalidInput("invalid IMAP mailbox encoding: invalid UTF-16".to_owned())
        })?;
        out.push_str(&decoded);
        i += 1;
    }

    Ok(out)
}

/// Best-effort normalization for legacy message IDs.
///
/// If the mailbox looks like wire-format modified UTF-7 and decodes
/// successfully, return the decoded UTF-8 form. Otherwise return input.
pub fn normalize_mailbox_name(input: &str) -> String {
    if !input.is_ascii() || !input.contains('&') {
        return input.to_owned();
    }
    match decode_mailbox_name(input) {
        Ok(decoded) => decoded,
        Err(_) => input.to_owned(),
    }
}

fn is_direct_char(ch: char) -> bool {
    matches!(ch, '\u{20}'..='\u{7e}')
}

fn flush_encoded_chunk(out: &mut String, utf16_chunk: &mut Vec<u16>) -> AppResult<()> {
    if utf16_chunk.is_empty() {
        return Ok(());
    }

    let mut bytes = Vec::with_capacity(utf16_chunk.len() * 2);
    for unit in utf16_chunk.iter().copied() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    let b64 = BASE64_ENGINE.encode(bytes);
    let mut modified = b64.trim_end_matches('=').replace('/', ",");
    out.push('&');
    out.push_str(&modified);
    out.push('-');
    utf16_chunk.clear();
    modified.clear();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{decode_mailbox_name, encode_mailbox_name, normalize_mailbox_name};

    #[test]
    fn encodes_ascii_passthrough() {
        assert_eq!(encode_mailbox_name("INBOX/Archive").expect("encode"), "INBOX/Archive");
    }

    #[test]
    fn round_trips_non_ascii() {
        let original = "收件箱/旅行";
        let encoded = encode_mailbox_name(original).expect("encode");
        let decoded = decode_mailbox_name(&encoded).expect("decode");
        assert_eq!(decoded, original);
    }

    #[test]
    fn encodes_literal_ampersand() {
        assert_eq!(encode_mailbox_name("R&D").expect("encode"), "R&-D");
        assert_eq!(decode_mailbox_name("R&-D").expect("decode"), "R&D");
    }

    #[test]
    fn decode_rejects_malformed_sequence() {
        let err = decode_mailbox_name("Inbox&Jjo").expect_err("must fail");
        assert!(err.to_string().contains("unterminated shift sequence"));
    }

    #[test]
    fn normalize_decodes_legacy_encoded_mailbox() {
        assert_eq!(normalize_mailbox_name("&U,BTFw-/Travel"), "台北/Travel");
    }
}
