//! Appends an authored rendered IFD without relocating or changing the source sensor payload.
use crate::fixture_path::FixturePath;

pub fn append(
    base: &std::path::Path,
    dimensions: [u32; 2],
    data: &[u8],
    rgb: bool,
    colour: Option<u32>,
) -> FixturePath {
    append_oriented(base, dimensions, data, rgb, colour, None)
}

pub fn append_oriented(
    base: &std::path::Path,
    dimensions: [u32; 2],
    data: &[u8],
    rgb: bool,
    colour: Option<u32>,
    orientation: Option<u16>,
) -> FixturePath {
    let mut bytes = std::fs::read(base).unwrap();
    assert_eq!(&bytes[..4], b"II\x2a\0");
    let root = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let count = u16::from_le_bytes(bytes[root..root + 2].try_into().unwrap()) as usize;
    let link = root + 2 + count * 12;
    if !bytes.len().is_multiple_of(2) {
        bytes.push(0);
    }
    let start = bytes.len();
    let [width, height] = dimensions;
    let mut entries = vec![
        (254u16, 4u16, 1u32),
        (256, 4, width),
        (257, 4, height),
        (258, 3, 8),
        (259, 3, if rgb { 1 } else { 6 }),
        (262, 3, 2),
        (277, 3, 3),
        (278, 4, height),
        (284, 3, 1),
    ];
    if let Some(orientation) = orientation {
        entries.push((274, 3, u32::from(orientation)));
    }
    if let Some(space) = colour {
        entries.push((50970, 4, space));
    }
    let payload = (start + 2 + (entries.len() + 2) * 12 + 4) as u32;
    if rgb {
        entries.extend([(273, 4, payload), (279, 4, data.len() as u32)]);
    } else {
        entries.extend([(513, 4, payload), (514, 4, data.len() as u32)]);
    }
    entries.sort_by_key(|entry| entry.0);
    bytes[link..link + 4].copy_from_slice(&(start as u32).to_le_bytes());
    bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (tag, kind, value) in entries {
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(bytes.len(), payload as usize);
    bytes.extend_from_slice(data);
    FixturePath::write("preview.dng", &bytes)
}
