//! Reading the v2ray `geoip.dat` / `geosite.dat` format just far enough to list its
//! categories. Used to validate downloads and to check that presets will find the
//! categories they reference before a core refuses the config.

/// Category names (country codes or site lists), lower-cased, in file order.
///
/// Both files are a protobuf message with one repeated field (1); each entry's first
/// field (1) is its name.
pub fn categories(data: &[u8]) -> Result<Vec<String>, &'static str> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let key = varint(data, &mut i)?;
        if key != (1 << 3 | 2) {
            return Err("not a v2ray geodata file");
        }
        let len = varint(data, &mut i)? as usize;
        let entry = data.get(i..i + len).ok_or("truncated geodata file")?;
        i += len;
        let mut j = 0;
        if varint(entry, &mut j)? == (1 << 3 | 2) {
            let n = varint(entry, &mut j)? as usize;
            let name = entry.get(j..j + n).ok_or("truncated geodata entry")?;
            out.push(String::from_utf8_lossy(name).to_ascii_lowercase());
        }
    }
    if out.is_empty() {
        return Err("the geodata file has no categories");
    }
    Ok(out)
}

fn varint(b: &[u8], i: &mut usize) -> Result<u64, &'static str> {
    let mut result = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *b.get(*i).ok_or("truncated geodata file")?;
        *i += 1;
        result |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(result);
        }
    }
    Err("bad varint in geodata file")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, payload: &[u8]) -> Vec<u8> {
        let mut inner = vec![0x0a, name.len() as u8];
        inner.extend_from_slice(name.as_bytes());
        inner.extend_from_slice(payload);
        let mut e = vec![0x0a, inner.len() as u8];
        e.extend(inner);
        e
    }

    #[test]
    fn lists_categories() {
        let mut file = entry("RU", &[0x12, 0x02, 0x08, 0x01]);
        file.extend(entry("ru-blocked", &[]));
        assert_eq!(categories(&file).unwrap(), ["ru", "ru-blocked"]);
    }

    #[test]
    fn rejects_other_files() {
        assert!(categories(b"<!DOCTYPE html><html>").is_err());
        assert!(categories(&[]).is_err());
        assert!(categories(&[0x0a, 0x10, 0x0a]).is_err());
    }
}
