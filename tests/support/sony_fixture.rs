//! Authored CFA TIFF with selected-IFD Sony lens data, not a proprietary decoder fixture.
#![allow(dead_code)]
use std::io::Write;

#[derive(Clone, Copy)]
pub(crate) enum Order {
    Little,
    Big,
}
impl Order {
    pub(crate) fn short(self, v: u16) -> Vec<u8> {
        match self {
            Self::Little => v.to_le_bytes().to_vec(),
            Self::Big => v.to_be_bytes().to_vec(),
        }
    }
    pub(crate) fn long(self, v: u32) -> Vec<u8> {
        match self {
            Self::Little => v.to_le_bytes().to_vec(),
            Self::Big => v.to_be_bytes().to_vec(),
        }
    }
    pub(crate) fn signed(self, values: &[i16]) -> Vec<u8> {
        values.iter().flat_map(|&v| self.short(v as u16)).collect()
    }
}
pub(crate) type Entry = (u16, u16, Vec<u8>);
pub(crate) fn profile_tags(order: Order) -> Vec<Entry> {
    vec![
        (0x7034, 3, order.short(1)),
        (0x7035, 8, order.signed(&[4, 16384, 16384, -32768, -32768])),
        (0x7036, 3, order.short(0)),
        (0x7037, 8, order.signed(&[2, -1024, -1024])),
    ]
}

/// The optional rendered IFD deliberately carries conflicting coefficients.
pub(crate) fn source(
    order: Order,
    tags: Vec<Entry>,
    dng: bool,
    decoy: bool,
) -> tempfile::NamedTempFile {
    let mut bytes = match order {
        Order::Little => b"II\x2a\0\x08\0\0\0".to_vec(),
        Order::Big => b"MM\0\x2a\0\0\0\x08".to_vec(),
    };
    let mut next_ifd = None;
    for rendered in [false, true].into_iter().take(if decoy { 2 } else { 1 }) {
        let ifd_offset = bytes.len() as u32;
        if let Some(offset) = next_ifd {
            bytes[offset..offset + 4].copy_from_slice(&order.long(ifd_offset));
        }
        let (side, samples, bits, photometric) = if rendered {
            (8, 3, 8, 2)
        } else {
            (64, 1, 16, 32803)
        };
        let mut entries = vec![
            (254, 4, order.long(u32::from(rendered))),
            (256, 4, order.long(side)),
            (257, 4, order.long(side)),
            (258, 3, order.short(bits)),
            (259, 3, order.short(1)),
            (262, 3, order.short(photometric)),
            (271, 2, b"SONY\0".to_vec()),
            (272, 2, b"ILCE-1M2\0".to_vec()),
            (273, 4, order.long(0)),
            (277, 3, order.short(samples)),
            (278, 4, order.long(side)),
            (
                279,
                4,
                order.long(side * side * u32::from(samples) * u32::from(bits / 8)),
            ),
            (284, 3, order.short(1)),
        ];
        if rendered {
            entries.extend([
                (0x7037, 8, order.signed(&[2, 8192, 8192])),
                (0x7032, 8, order.signed(&[2, 16384, 16384])),
            ]);
        } else {
            entries.extend([
                (33421, 3, [order.short(2), order.short(2)].concat()),
                (33422, 1, vec![0, 1, 1, 2]),
            ]);
            entries.extend(tags.clone());
            if dng {
                entries.extend([
                    (50706, 1, vec![1, 4, 0, 0]),
                    (50708, 2, b"SONY ILCE-1M2\0".to_vec()),
                    (50710, 1, vec![0, 1, 2]),
                    (50717, 4, order.long(65535)),
                ]);
            }
        }
        entries.sort_by_key(|e| e.0);
        bytes.extend(order.short(entries.len() as u16));
        let entry_start = bytes.len();
        next_ifd = Some(entry_start + entries.len() * 12);
        bytes.resize(entry_start + entries.len() * 12 + 4, 0);
        let mut strip_pointer = 0;
        let mut tile_table = None;
        for (i, (tag, kind, data)) in entries.into_iter().enumerate() {
            let offset = entry_start + i * 12;
            let size = match kind {
                3 | 8 => 2,
                4 => 4,
                _ => 1,
            };
            bytes[offset..offset + 2].copy_from_slice(&order.short(tag));
            bytes[offset + 2..offset + 4].copy_from_slice(&order.short(kind));
            bytes[offset + 4..offset + 8].copy_from_slice(&order.long((data.len() / size) as u32));
            if tag == 273 {
                strip_pointer = offset + 8;
            }
            if data.len() <= 4 {
                bytes[offset + 8..offset + 8 + data.len()].copy_from_slice(&data);
            } else {
                let position = bytes.len() as u32;
                bytes[offset + 8..offset + 12].copy_from_slice(&order.long(position));
                if tag == 324 {
                    assert_eq!(data.len(), 8);
                    tile_table = Some(position as usize);
                }
                bytes.extend(data);
                if !bytes.len().is_multiple_of(2) {
                    bytes.push(0);
                }
            }
        }
        let strip = bytes.len() as u32;
        bytes[strip_pointer..strip_pointer + 4].copy_from_slice(&order.long(strip));
        if let Some(table) = tile_table {
            bytes[table..table + 4].copy_from_slice(&order.long(strip));
            bytes[table + 4..table + 8].copy_from_slice(&order.long(strip + 4096));
        }
        if rendered {
            bytes.extend(std::iter::repeat_n(64, 8 * 8 * 3));
        } else {
            // The spatial ramp exposes resampling. LibRaw treats Sony slots as 14-bit despite
            // BitsPerSample=16; every declared sensor value fits that range.
            for y in 0..64 {
                for x in 0..64 {
                    let base = match (x % 2, y % 2) {
                        (0, 0) => 4000,
                        (1, 1) => 10000,
                        _ => 7000,
                    };
                    bytes.extend(order.short(base + x * 31 + y * 17));
                }
            }
        }
    }
    let mut file = tempfile::Builder::new().suffix(".arw").tempfile().unwrap();
    file.write_all(&bytes).unwrap();
    file.flush().unwrap();
    file
}
