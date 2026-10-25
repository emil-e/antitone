#[path = "fixture_path.rs"]
mod fixture_path;
pub use fixture_path::FixturePath;

pub const SIDE: u32 = 64;

/// Tiny authored integer-Bayer DNG; no private photographs or GPU work.
pub fn bayer_dng() -> FixturePath {
    let short = |value: u16| value.to_le_bytes().to_vec();
    let long = |value: u32| value.to_le_bytes().to_vec();
    let rational = |value: u32| [value.to_le_bytes(), 1u32.to_le_bytes()].concat();
    let matrix = [
        3.2404542, -1.5371385, -0.4985314, -0.9692660, 1.8760108, 0.0415560, 0.0556434, -0.2040259,
        1.0572252,
    ];
    let matrix: Vec<u8> = matrix
        .into_iter()
        .flat_map(|value| {
            [
                ((value * 1_000_000.0) as i32).to_le_bytes(),
                1_000_000i32.to_le_bytes(),
            ]
            .concat()
        })
        .collect();
    let mut entries = vec![
        (254u16, 4u16, long(0)),
        (256, 4, long(SIDE)),
        (257, 4, long(SIDE)),
        (258, 3, short(16)),
        (259, 3, short(1)),
        (262, 3, short(32803)),
        (271, 2, b"Antitone\0".to_vec()),
        (272, 2, b"Source fixture\0".to_vec()),
        (273, 4, long(0)),
        (277, 3, short(1)),
        (278, 4, long(SIDE)),
        (279, 4, long(SIDE * SIDE * 2)),
        (284, 3, short(1)),
        (33421, 3, [short(2), short(2)].concat()),
        (33422, 1, vec![0, 1, 1, 2]),
        (50706, 1, vec![1, 4, 0, 0]),
        (50707, 1, vec![1, 1, 0, 0]),
        (50708, 2, b"Antitone Source fixture\0".to_vec()),
        (50710, 1, vec![0, 1, 2]),
        (50711, 3, short(1)),
        (50714, 5, rational(0)),
        (50717, 4, long(65535)),
        (50721, 10, matrix),
        (50728, 5, [rational(1), rational(1), rational(1)].concat()),
        (50778, 3, short(21)),
    ];
    entries.sort_by_key(|entry| entry.0);
    let mut bytes = b"II\x2a\x00\x08\x00\x00\x00".to_vec();
    bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    let end = bytes.len() + entries.len() * 12 + 4;
    bytes.resize(end, 0);
    let mut strip_offset = 0;
    for (index, (tag, kind, data)) in entries.into_iter().enumerate() {
        let offset = 10 + index * 12;
        let size = match kind {
            3 => 2,
            4 => 4,
            5 | 10 => 8,
            _ => 1,
        };
        bytes[offset..offset + 2].copy_from_slice(&tag.to_le_bytes());
        bytes[offset + 2..offset + 4].copy_from_slice(&kind.to_le_bytes());
        bytes[offset + 4..offset + 8].copy_from_slice(&(data.len() as u32 / size).to_le_bytes());
        if tag == 273 {
            strip_offset = offset + 8;
        }
        if data.len() <= 4 {
            bytes[offset + 8..offset + 8 + data.len()].copy_from_slice(&data);
        } else {
            let position = bytes.len() as u32;
            bytes[offset + 8..offset + 12].copy_from_slice(&position.to_le_bytes());
            bytes.extend_from_slice(&data);
            if !bytes.len().is_multiple_of(2) {
                bytes.push(0);
            }
        }
    }
    let position = bytes.len() as u32;
    bytes[strip_offset..strip_offset + 4].copy_from_slice(&position.to_le_bytes());
    for y in 0..SIDE {
        for x in 0..SIDE {
            let value: u16 = match (x % 2, y % 2) {
                (0, 0) => 8000,
                (1, 1) => 24000,
                _ => 16000,
            };
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    FixturePath::write("fixture.dng", &bytes)
}
