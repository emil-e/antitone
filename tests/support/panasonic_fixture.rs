//! Authored uncompressed RW2 carrying a native PanasonicRaw IFD.
use crate::fixture_path::FixturePath;

pub(crate) fn distortion() -> Vec<u8> {
    let mut words = [0_i16; 16];
    words[7] = -24575; // Flag 1 in the low nibble, with nonzero upper bits.
    words[5] = -16384;
    words[8] = -2048;
    words[4] = 1024;
    words[11] = 512;
    words.into_iter().flat_map(i16::to_le_bytes).collect()
}

pub(crate) fn source(payload: Option<Vec<u8>>, square: bool) -> FixturePath {
    let mut tags: Vec<(u16, u16, Vec<u8>)> = [
        (2, 128_u16),
        (3, 96),
        (4, 4),
        (5, 8),
        (6, 92),
        (7, 120),
        (9, 1),
        (10, 12),
        (0x1c, 128),
        (0x1d, 128),
        (0x1e, 128),
        (0x24, 512),
        (0x25, 256),
        (0x26, 768),
        (0x2f, 4),
        (0x30, if square { 20 } else { 8 }),
        (0x31, 92),
        (0x32, if square { 108 } else { 120 }),
    ]
    .into_iter()
    .map(|(tag, value)| (tag, 3, value.to_le_bytes().to_vec()))
    .collect();
    tags.insert(0, (1, 1, b"0110".to_vec()));
    tags.extend([
        (0x010f, 2, b"Panasonic\0".to_vec()),
        (0x0110, 2, b"DC-G9\0".to_vec()),
        (0x0111, 4, vec![0; 4]),
    ]);
    if let Some(payload) = payload {
        tags.push((0x0119, 7, payload));
    }

    let mut bytes = b"II\x55\0\x18\0\0\0".to_vec();
    bytes.resize(24, 0);
    bytes.extend((tags.len() as u16).to_le_bytes());
    let table = bytes.len();
    bytes.resize(table + tags.len() * 12 + 4, 0);
    let mut strip = 0;
    for (i, (tag, kind, data)) in tags.into_iter().enumerate() {
        let entry = table + i * 12;
        bytes[entry..entry + 2].copy_from_slice(&tag.to_le_bytes());
        bytes[entry + 2..entry + 4].copy_from_slice(&kind.to_le_bytes());
        let element_size = match kind {
            3 => 2,
            4 => 4,
            _ => 1,
        };
        let count = (data.len() / element_size) as u32;
        bytes[entry + 4..entry + 8].copy_from_slice(&count.to_le_bytes());
        let value = entry + 8;
        if tag == 0x0111 {
            strip = value;
        } else if data.len() <= 4 {
            bytes[value..value + data.len()].copy_from_slice(&data);
        } else {
            let offset = (bytes.len() as u32).to_le_bytes();
            bytes[value..value + 4].copy_from_slice(&offset);
            bytes.extend(data);
        }
    }
    let offset = (bytes.len() as u32).to_le_bytes();
    bytes[strip..strip + 4].copy_from_slice(&offset);
    for y in 0..96_u16 {
        for x in 0..128_u16 {
            let code = 600 + 5 * x + 3 * y + 500 * (y % 2) + 500 * (x % 2);
            bytes.extend((code << 4).to_le_bytes());
        }
    }
    FixturePath::write("fixture.RW2", &bytes)
}
