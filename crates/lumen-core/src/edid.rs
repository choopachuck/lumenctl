//! Minimal EDID 1.x parser for monitor identification.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Edid {
    pub manufacturer_id: String,
    pub product_code: u16,
    pub serial_number: u32,
    pub week: u8,
    pub year: u16,
    pub name: Option<String>,
    pub serial_text: Option<String>,
    pub unspecified_text: Option<String>,
}

impl Edid {
    pub fn serial(&self) -> Option<String> {
        self.serial_text
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| (self.serial_number != 0).then(|| self.serial_number.to_string()))
    }
}

pub fn parse(data: &[u8]) -> Option<Edid> {
    const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
    if data.len() < 128 || data[..8] != HEADER {
        return None;
    }
    let m = u16::from_be_bytes([data[8], data[9]]);
    let letter = |v: u16| (b'A' + ((v & 0x1F) as u8).saturating_sub(1)) as char;
    let manufacturer_id: String = [letter(m >> 10), letter(m >> 5), letter(m)].iter().collect();

    let mut edid = Edid {
        manufacturer_id,
        product_code: u16::from_le_bytes([data[10], data[11]]),
        serial_number: u32::from_le_bytes([data[12], data[13], data[14], data[15]]),
        week: data[16],
        year: 1990 + data[17] as u16,
        ..Default::default()
    };

    for block in data[54..126].chunks_exact(18) {
        if block[0] != 0 || block[1] != 0 {
            continue;
        }
        let text = descriptor_text(&block[5..18]);
        match block[3] {
            0xFC => edid.name = Some(text),
            0xFF => edid.serial_text = Some(text),
            0xFE => edid.unspecified_text = Some(text),
            _ => {}
        }
    }
    Some(edid)
}

fn descriptor_text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0x0A).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header_and_descriptors() {
        let mut d = vec![0u8; 128];
        d[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
        // "PHL" = 16,8,12
        let m: u16 = (16 << 10) | (8 << 5) | 12;
        d[8..10].copy_from_slice(&m.to_be_bytes());
        d[10..12].copy_from_slice(&0x090Bu16.to_le_bytes());
        d[16] = 37;
        d[17] = 27;
        let name = b"PHL 328P6V\n  ";
        d[54..59].copy_from_slice(&[0, 0, 0, 0xFC, 0]);
        d[59..72].copy_from_slice(name);
        let e = parse(&d).unwrap();
        assert_eq!(e.manufacturer_id, "PHL");
        assert_eq!(e.product_code, 0x090B);
        assert_eq!(e.year, 2017);
        assert_eq!(e.week, 37);
        assert_eq!(e.name.as_deref(), Some("PHL 328P6V"));
    }
}
