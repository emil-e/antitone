//! Synthetic Sony YCbCr RAW: two constant lossless-JPEG tiles, not a camera photograph.
use crate::fixture_path::FixturePath;

// Each 64x64 tile has one luminance and neutral chroma. The second tile doubles
// signal above Sony's decoder black (1024), rather than doubling the black pedestal.
pub const LUMINANCE: [u16; 2] = [4096, 7168];
pub const WB: [f64; 4] = [2048., 1024., 1536., 1024.];

fn jpeg(y: u16) -> Vec<u8> {
    fn segment(bytes: &mut Vec<u8>, marker: u8, payload: &[u8]) {
        bytes.extend([255, marker]);
        bytes.extend(((payload.len() + 2) as u16).to_be_bytes());
        bytes.extend(payload);
    }
    let mut bytes = vec![255, 216];
    // One DC table: categories 0, 1 and 14 have codes 00, 01 and 10.
    let mut table = vec![0, 0, 3];
    table.extend([0; 14]);
    table.extend([0, 1, 14]);
    segment(&mut bytes, 0xc4, &table);
    segment(
        &mut bytes,
        0xc3,
        &[15, 0, 64, 0, 64, 3, 1, 0x22, 0, 2, 0x11, 0, 3, 0x11, 0],
    );
    segment(&mut bytes, 0xda, &[3, 1, 0, 2, 0, 3, 0, 1, 0, 0]);
    // The first Y predicts from 16384. Cb/Cr=16383 predict from that same midpoint.
    // Every subsequent difference is zero, including the other three Ys in each MCU.
    let amplitude = i32::from(y) - 16384 + 16383;
    let mut bits = format!("10{amplitude:014b}000000010010");
    bits.push_str(&"00".repeat((32 * 32 - 1) * 6));
    bits.push_str(&"1".repeat((8 - bits.len() % 8) % 8));
    for chunk in bits.as_bytes().as_chunks::<8>().0 {
        let value = chunk.iter().fold(0, |value, bit| value * 2 + (bit - b'0'));
        bytes.push(value);
        if value == 255 {
            bytes.push(0);
        }
    }
    bytes.extend([255, 217]);
    bytes
}

pub fn source() -> FixturePath {
    source_with_balance(Some([WB[0] as u32, WB[1] as u32, WB[2] as u32]))
}

pub fn source_with_balance(balance: Option<[u32; 3]>) -> FixturePath {
    source_with_profile(balance, false)
}

pub fn source_with_profile(balance: Option<[u32; 3]>, shading: bool) -> FixturePath {
    let short = |v: u16| v.to_le_bytes().to_vec();
    let long = |v: u32| v.to_le_bytes().to_vec();
    let tiles = LUMINANCE.map(jpeg);
    let mut entries = vec![
        (254u16, 4u16, long(0)),
        (256, 4, long(128)),
        (257, 4, long(64)),
        (258, 3, short(15)),
        (259, 3, short(6)),
        (262, 3, short(6)),
        (271, 2, b"SONY\0".to_vec()),
        (272, 2, b"ILCE-7RM5\0".to_vec()),
        (274, 3, short(1)),
        (277, 3, short(3)),
        (284, 3, short(1)),
        (322, 4, long(64)),
        (323, 4, long(64)),
        (324, 4, vec![0; 8]),
        (
            325,
            4,
            tiles.iter().flat_map(|t| long(t.len() as u32)).collect(),
        ),
    ];
    if let Some(balance) = balance {
        entries.push((
            50728,
            5,
            balance
                .into_iter()
                .flat_map(|gain| [long(1), long(gain)].concat())
                .collect(),
        ));
    }
    if shading {
        // Constant inverse shading gain sqrt(2) under the shared Sony profile interpretation.
        entries.push((0x7032, 8, [short(2), short(8192), short(8192)].concat()));
    }
    entries.sort_by_key(|entry| entry.0);
    let mut bytes = b"II\x2a\0\x08\0\0\0".to_vec();
    bytes.extend(short(entries.len() as u16));
    bytes.resize(10 + entries.len() * 12 + 4, 0);
    let mut tile_table = 0;
    for (index, (tag, kind, data)) in entries.into_iter().enumerate() {
        let offset = 10 + index * 12;
        let size = match kind {
            3 | 8 => 2,
            4 => 4,
            5 => 8,
            _ => 1,
        };
        bytes[offset..offset + 2].copy_from_slice(&tag.to_le_bytes());
        bytes[offset + 2..offset + 4].copy_from_slice(&kind.to_le_bytes());
        bytes[offset + 4..offset + 8].copy_from_slice(&((data.len() / size) as u32).to_le_bytes());
        if data.len() <= 4 {
            bytes[offset + 8..offset + 8 + data.len()].copy_from_slice(&data);
        } else {
            let position = bytes.len();
            bytes[offset + 8..offset + 12].copy_from_slice(&(position as u32).to_le_bytes());
            if tag == 324 {
                tile_table = position;
            }
            bytes.extend(data);
            if !bytes.len().is_multiple_of(2) {
                bytes.push(0);
            }
        }
    }
    for (index, tile) in tiles.into_iter().enumerate() {
        let offset = tile_table + index * 4;
        let position = bytes.len() as u32;
        bytes[offset..offset + 4].copy_from_slice(&position.to_le_bytes());
        bytes.extend(tile);
    }
    FixturePath::write("fixture.arw", &bytes)
}
