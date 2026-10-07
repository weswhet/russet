//! Converts BER, which Apple's CMS signatures use, to DER, which the `der`
//! crate requires: indefinite lengths become definite, and constructed
//! strings become primitive. Element order is kept, so DER content that
//! was signed keeps its exact bytes.

const MAX_DEPTH: usize = 64;

fn error(message: &str) -> String {
    format!("Invalid BER: {message}")
}

/// Converts one BER value at the start of `input` and returns it as DER,
/// with the number of input bytes it used.
pub(crate) fn to_der(input: &[u8]) -> Result<(Vec<u8>, usize), String> {
    let mut out = Vec::new();
    let used = value(input, 0, &mut out)?;
    Ok((out, used))
}

fn push_length(out: &mut Vec<u8>, length: usize) {
    if length < 0x80 {
        out.push(length as u8);
    } else {
        let bytes = length.to_be_bytes();
        let skip = bytes.iter().take_while(|b| **b == 0).count();
        out.push(0x80 | (bytes.len() - skip) as u8);
        out.extend_from_slice(&bytes[skip..]);
    }
}

/// Universal string types whose constructed (chunked) form DER forbids.
fn is_string(tag: u8) -> bool {
    matches!(tag & 0x1f, 0x03 | 0x04 | 0x0c | 0x12..=0x16 | 0x19..=0x1e) && tag & 0xc0 == 0
}

fn value(input: &[u8], depth: usize, out: &mut Vec<u8>) -> Result<usize, String> {
    if depth > MAX_DEPTH {
        return Err(error("nested too deeply"));
    }
    let mut at = 0;
    let first = *input.first().ok_or_else(|| error("truncated tag"))?;
    let mut tag = vec![first];
    at += 1;
    if first & 0x1f == 0x1f {
        loop {
            let b = *input.get(at).ok_or_else(|| error("truncated tag"))?;
            tag.push(b);
            at += 1;
            if b & 0x80 == 0 {
                break;
            }
            if tag.len() > 6 {
                return Err(error("tag too long"));
            }
        }
    }
    let constructed = first & 0x20 != 0;
    let length_byte = *input.get(at).ok_or_else(|| error("truncated length"))?;
    at += 1;
    let definite = if length_byte == 0x80 {
        if !constructed {
            return Err(error("indefinite primitive"));
        }
        None
    } else if length_byte & 0x80 != 0 {
        let count = (length_byte & 0x7f) as usize;
        if count > 4 {
            return Err(error("length too long"));
        }
        let mut length = 0usize;
        for _ in 0..count {
            length =
                (length << 8) | *input.get(at).ok_or_else(|| error("truncated length"))? as usize;
            at += 1;
        }
        Some(length)
    } else {
        Some(length_byte as usize)
    };
    if !constructed {
        let length = definite.unwrap();
        let content = input
            .get(at..at + length)
            .ok_or_else(|| error("truncated content"))?;
        out.extend_from_slice(&tag);
        push_length(out, length);
        out.extend_from_slice(content);
        return Ok(at + length);
    }
    // Convert the children, then decide how to write them.
    let mut children = Vec::new();
    let end = match definite {
        Some(length) => {
            let end = at + length;
            if end > input.len() {
                return Err(error("truncated content"));
            }
            while at < end {
                at += value(&input[at..end], depth + 1, &mut children)?;
            }
            end
        }
        None => loop {
            if input.get(at..at + 2) == Some(&[0, 0]) {
                break at + 2;
            }
            if at >= input.len() {
                return Err(error("missing end of contents"));
            }
            at += value(&input[at..], depth + 1, &mut children)?;
        },
    };
    if tag.len() == 1 && is_string(first) {
        // Join the chunks' contents into one primitive string.
        let mut joined = Vec::new();
        let mut rest = children.as_slice();
        while !rest.is_empty() {
            let (chunk, used) = primitive_content(rest)?;
            joined.extend_from_slice(chunk);
            rest = &rest[used..];
        }
        out.push(first & !0x20);
        push_length(out, joined.len());
        out.extend_from_slice(&joined);
    } else {
        out.extend_from_slice(&tag);
        push_length(out, children.len());
        out.extend_from_slice(&children);
    }
    Ok(end)
}

/// Reads a DER value's content (the chunks are already DER).
fn primitive_content(der: &[u8]) -> Result<(&[u8], usize), String> {
    let length_byte = *der.get(1).ok_or_else(|| error("truncated chunk"))?;
    let (length, header) = if length_byte & 0x80 != 0 {
        let count = (length_byte & 0x7f) as usize;
        let mut length = 0usize;
        for i in 0..count {
            length =
                (length << 8) | *der.get(2 + i).ok_or_else(|| error("truncated chunk"))? as usize;
        }
        (length, 2 + count)
    } else {
        (length_byte as usize, 2)
    };
    let content = der
        .get(header..header + length)
        .ok_or_else(|| error("truncated chunk"))?;
    Ok((content, header + length))
}

#[cfg(test)]
mod tests {
    use super::to_der;

    #[test]
    fn converts_indefinite_lengths_and_chunked_strings() {
        // SEQUENCE (indefinite) { OCTET STRING (constructed, indefinite) { "ab", "c" }, INTEGER 5 }
        let ber = [
            0x30, 0x80, 0x24, 0x80, 0x04, 0x02, b'a', b'b', 0x04, 0x01, b'c', 0x00, 0x00, 0x02,
            0x01, 0x05, 0x00, 0x00, 0xff,
        ];
        let (der, used) = to_der(&ber).unwrap();
        assert_eq!(
            der,
            [0x30, 0x08, 0x04, 0x03, b'a', b'b', b'c', 0x02, 0x01, 0x05]
        );
        assert_eq!(used, ber.len() - 1);
        assert!(to_der(&[0x30, 0x80, 0x04, 0x01]).is_err());
    }
}
