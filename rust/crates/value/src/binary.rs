//! Binary plist object decoder, including Python plistlib's null token.
use crate::{Dictionary, Uid, Value};
fn unsigned(bytes: &[u8]) -> Result<u64, String> {
    if bytes.is_empty() || bytes.len() > 8 {
        return Err("Invalid binary plist integer width".into());
    }
    Ok(bytes.iter().fold(0, |n, b| (n << 8) | u64::from(*b)))
}
fn index(bytes: &[u8]) -> Result<usize, String> {
    usize::try_from(unsigned(bytes)?).map_err(|_| "Binary plist index exceeds address space".into())
}
pub(super) fn parse(bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() < 40 || &bytes[..8] != b"bplist00" {
        return Err("Invalid binary plist header".into());
    }
    let trailer = &bytes[bytes.len() - 32..];
    let offset_size = usize::from(trailer[6]);
    let ref_size = usize::from(trailer[7]);
    if !(1..=8).contains(&offset_size) || !(1..=8).contains(&ref_size) {
        return Err("Invalid binary plist offset/reference width".into());
    }
    let count = index(&trailer[8..16])?;
    let root = index(&trailer[16..24])?;
    let table = index(&trailer[24..32])?;
    if count == 0
        || root >= count
        || table < 8
        || table > bytes.len() - 32
        || count > (bytes.len() - 32 - table) / offset_size
    {
        return Err("Invalid binary plist offset table".into());
    }
    let mut offsets = Vec::with_capacity(count);
    for i in 0..count {
        let offset = index(&bytes[table + i * offset_size..table + (i + 1) * offset_size])?;
        if !(8..table).contains(&offset) {
            return Err("Binary plist object offset outside object table".into());
        }
        offsets.push(offset);
    }
    let mut reader = Reader {
        bytes: &bytes[..table],
        offsets,
        ref_size,
        active: vec![false; count],
        cache: vec![None; count],
    };
    reader.object(root, 0).map(|(v, _)| v)
}
struct Reader<'a> {
    bytes: &'a [u8],
    offsets: Vec<usize>,
    ref_size: usize,
    active: Vec<bool>,
    cache: Vec<Option<(Value, usize)>>,
}
impl Reader<'_> {
    fn take(&self, pos: &mut usize, length: usize) -> Result<&[u8], String> {
        let end = pos
            .checked_add(length)
            .ok_or("Binary plist length overflow")?;
        let value = self
            .bytes
            .get(*pos..end)
            .ok_or("Truncated binary plist object")?;
        *pos = end;
        Ok(value)
    }
    fn count(&self, pos: &mut usize, low: u8) -> Result<usize, String> {
        if low != 15 {
            return Ok(low.into());
        }
        let token = self.take(pos, 1)?[0];
        if token >> 4 != 1 || token & 15 > 3 {
            return Err("Invalid binary plist object length".into());
        }
        index(self.take(pos, 1usize << (token & 15))?)
    }
    fn object(&mut self, id: usize, depth: usize) -> Result<(Value, usize), String> {
        if id >= self.offsets.len() {
            return Err("Binary plist reference is out of range".into());
        }
        if depth > 128 {
            return Err("Binary plist nesting exceeds 128 levels".into());
        }
        if self.active[id] {
            return Err("Cyclic binary plist object references".into());
        }
        if let Some(v) = &self.cache[id] {
            return Ok(v.clone());
        }
        self.active[id] = true;
        let mut pos = self.offsets[id];
        let token = self.take(&mut pos, 1)?[0];
        let low = token & 15;
        let mut weight = 1usize;
        let value = match token >> 4 {
            0 => match low {
                0 => Value::Null,
                8 => Value::Boolean(false),
                9 => Value::Boolean(true),
                _ => return Err("Unsupported binary plist simple value".into()),
            },
            1 => {
                if low > 4 {
                    return Err("Binary plist integer exceeds 128 bits".into());
                }
                let bytes = self.take(&mut pos, 1usize << low)?;
                if bytes.len() < 8 {
                    Value::Integer(unsigned(bytes)?.into())
                } else {
                    let mut padded = if bytes[0] & 128 != 0 {
                        [255; 16]
                    } else {
                        [0; 16]
                    };
                    padded[16 - bytes.len()..].copy_from_slice(bytes);
                    let n = i128::from_be_bytes(padded);
                    if let Ok(n) = i64::try_from(n) {
                        Value::Integer(n.into())
                    } else if let Ok(n) = u64::try_from(n) {
                        Value::Integer(n.into())
                    } else {
                        return Err("Binary plist integer exceeds native range".into());
                    }
                }
            }
            2 => match low {
                2 => Value::Real(
                    f32::from_be_bytes(self.take(&mut pos, 4)?.try_into().unwrap()).into(),
                ),
                3 => Value::Real(f64::from_be_bytes(
                    self.take(&mut pos, 8)?.try_into().unwrap(),
                )),
                _ => return Err("Invalid binary plist real width".into()),
            },
            3 => {
                if low != 3 {
                    return Err("Invalid binary plist date width".into());
                }
                // Delegate date range/NaN checks to the native decoder using a
                // one-object plist, avoiding unchecked SystemTime conversions.
                let date = self.take(&mut pos, 8)?;
                let mut single = b"bplist00\x33".to_vec();
                single.extend_from_slice(date);
                single.push(8);
                single.extend_from_slice(&[0, 0, 0, 0, 0, 0, 1, 1]);
                single.extend_from_slice(&1u64.to_be_bytes());
                single.extend_from_slice(&0u64.to_be_bytes());
                single.extend_from_slice(&17u64.to_be_bytes());
                native_plist::Value::from_reader(std::io::Cursor::new(single))
                    .map(Value::from)
                    .map_err(|e| e.to_string())?
            }
            4 => {
                let count = self.count(&mut pos, low)?;
                Value::Data(self.take(&mut pos, count)?.to_vec())
            }
            5 | 7 => {
                let count = self.count(&mut pos, low)?;
                let data = self.take(&mut pos, count)?;
                if token >> 4 == 5 && !data.is_ascii() {
                    return Err("Non-ASCII binary plist ASCII string".into());
                }
                Value::String(std::str::from_utf8(data).map_err(|e| e.to_string())?.into())
            }
            6 => {
                let count = self
                    .count(&mut pos, low)?
                    .checked_mul(2)
                    .ok_or("Binary plist string length overflow")?;
                let data = self.take(&mut pos, count)?;
                Value::String(
                    String::from_utf16(
                        &data
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|v| u16::from_be_bytes([v[0], v[1]]))
                            .collect::<Vec<_>>(),
                    )
                    .map_err(|e| e.to_string())?,
                )
            }
            8 => Value::Uid(Uid::new(unsigned(
                self.take(&mut pos, usize::from(low) + 1)?,
            )?)),
            10 | 13 => {
                let count = self.count(&mut pos, low)?;
                let references = count
                    .checked_mul(if token >> 4 == 13 { 2 } else { 1 })
                    .and_then(|n| n.checked_mul(self.ref_size))
                    .ok_or("Binary plist container size overflow")?;
                let ids = self
                    .take(&mut pos, references)?
                    .chunks_exact(self.ref_size)
                    .map(index)
                    .collect::<Result<Vec<_>, _>>()?;
                let mut values = Vec::with_capacity(ids.len());
                for id in ids {
                    let (value, size) = self.object(id, depth + 1)?;
                    weight = weight
                        .checked_add(size)
                        .filter(|n| *n <= 1_000_000)
                        .ok_or("Binary plist expanded value exceeds one million nodes")?;
                    values.push(value);
                }
                if token >> 4 == 10 {
                    Value::Array(values)
                } else {
                    let items = values.split_off(count);
                    let mut dictionary = Dictionary::new();
                    for (key, value) in values.into_iter().zip(items) {
                        dictionary.insert(
                            key.into_string()
                                .ok_or("Binary plist dictionary key is not a string")?,
                            value,
                        );
                    }
                    Value::Dictionary(dictionary)
                }
            }
            _ => return Err(format!("Unsupported binary plist object token {token:#x}")),
        };
        self.active[id] = false;
        self.cache[id] = Some((value.clone(), weight));
        Ok((value, weight))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Generated with the pinned reference interpreter's plistlib.dumps(...,
    // FMT_BINARY): unlike XML, binary plist input permits None/null tokens.
    const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/python-null.bplist");
    #[test]
    fn python_binary_null_and_all_scalar_types() {
        let d = Value::from_reader(std::io::Cursor::new(FIXTURE))
            .unwrap()
            .into_dictionary()
            .unwrap();
        assert!(d["null"].is_null());
        assert_eq!(
            d["booleans"].as_array().unwrap(),
            &vec![false.into(), true.into()]
        );
        assert_eq!(d["signed"].as_signed_integer(), Some(i64::MIN));
        assert_eq!(d["unsigned"].as_unsigned_integer(), Some(u64::MAX));
        assert_eq!(d["float"].as_real(), Some(1.25));
        assert!(d["negative_zero"].as_real().unwrap().is_sign_negative());
        assert_eq!(d["data"].as_data(), Some(b"\0\xffhello".as_slice()));
        assert_eq!(
            d["date"].as_date().unwrap().to_xml_format(),
            "2024-02-29T03:04:05Z"
        );
        assert_eq!(d["unicode"].as_string(), Some("café 日本語 🚀"));
        assert_eq!(d["uid"].as_uid().unwrap().get(), u64::MAX);
        let nested = d["nested"].as_array().unwrap();
        assert!(nested[0].is_null());
        assert!(nested[1].as_dictionary().unwrap()["null"].is_null());
        assert_eq!(
            d["many"].as_array().unwrap(),
            &(0..300).map(Value::from).collect::<Vec<_>>()
        );
    }
    fn single(object: &[u8]) -> Vec<u8> {
        let mut bytes = b"bplist00".to_vec();
        bytes.extend_from_slice(object);
        let table = bytes.len();
        bytes.push(8);
        bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, 1, 1]);
        bytes.extend_from_slice(&1u64.to_be_bytes());
        bytes.extend_from_slice(&0u64.to_be_bytes());
        bytes.extend_from_slice(&(table as u64).to_be_bytes());
        bytes
    }
    #[test]
    fn malformed_references_cycles_lengths_and_dates_are_errors() {
        for (object, reason) in [
            (vec![0xa1, 0], "Cyclic"),
            (vec![0xa1, 1], "out of range"),
            (
                vec![0x4f, 0x13, 255, 255, 255, 255, 255, 255, 255, 255],
                "overflow",
            ),
            (vec![0x33, 127, 240, 0, 0, 0, 0, 0, 0], "date"),
            (vec![0x61, 0xd8, 0x00], "utf-16"),
        ] {
            let error = parse(&single(&object)).unwrap_err();
            assert!(
                error.to_lowercase().contains(&reason.to_lowercase()),
                "{error}"
            );
        }
        let mut offset = single(&[0]);
        let length = offset.len();
        offset[length - 8..].copy_from_slice(&u64::MAX.to_be_bytes());
        assert!(parse(&offset).is_err());
        for length in [0, 7, 8, 31, 39, FIXTURE.len() - 1] {
            assert!(parse(&FIXTURE[..length]).is_err());
        }
    }
}
