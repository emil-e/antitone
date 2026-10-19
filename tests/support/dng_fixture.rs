//! Authored SDK-adapter DNGs: explicit sensor codes and opcode bytes, no photographed input.

use std::io::Write;

pub struct Opcode {
    pub id: u32,
    pub version: u32,
    pub flags: u32,
    pub data: Vec<u8>,
}

impl Opcode {
    pub fn gain(value: f32, stage: usize) -> Self {
        let side: u32 = if stage == 0 { 64 } else { 56 };
        let mut data = Vec::new();
        for v in [0, 0, side, side, 0, 1, 1, 1, 2, 2] {
            data.extend(v.to_be_bytes());
        }
        for v in [1.0_f64, 1., 0., 0.] {
            data.extend(v.to_be_bytes());
        }
        data.extend(1u32.to_be_bytes());
        for _ in 0..4 {
            data.extend(value.to_be_bytes());
        }
        Self {
            id: 9,
            version: 0x01030000,
            flags: 0,
            data,
        }
    }

    pub fn warp(coefficients: &[[f64; 6]], center: [f64; 2]) -> Self {
        let mut data = (coefficients.len() as u32).to_be_bytes().to_vec();
        for v in coefficients.iter().flatten().chain(center.iter()) {
            data.extend(v.to_be_bytes());
        }
        Self {
            id: 1,
            version: 0x01030000,
            flags: 0,
            data,
        }
    }

    pub fn bad_pixels(constant: u32, phase: u32) -> Self {
        Self {
            id: 4,
            version: 0x01030000,
            flags: 0,
            data: [constant.to_be_bytes(), phase.to_be_bytes()].concat(),
        }
    }

    pub fn unknown(optional: bool) -> Self {
        Self {
            id: 999,
            version: 0x01030000,
            flags: u32::from(optional),
            data: Vec::new(),
        }
    }

    fn bytes(&self) -> Vec<u8> {
        let mut result = Vec::new();
        for v in [self.id, self.version, self.flags, self.data.len() as u32] {
            result.extend(v.to_be_bytes());
        }
        result.extend(&self.data);
        result
    }
}

pub struct Fixture {
    pub side: u32,
    pub lists: [Vec<Opcode>; 3],
    pub marker: Option<(u32, u32, u16)>,
    pub neutral: [(u32, u32); 3],
    pub orientation: u16,
    pub ramp: bool,
    pub bits: u16,
    pub lossless_jpeg: bool,
    pub pattern: [u8; 4],
    pub extra: Vec<(u16, u16, Vec<u8>)>,
}

impl Default for Fixture {
    fn default() -> Self {
        Self {
            side: 64,
            lists: Default::default(),
            marker: None,
            neutral: [(1, 1); 3],
            orientation: 1,
            ramp: false,
            bits: 16,
            lossless_jpeg: false,
            pattern: [0, 1, 1, 2],
            extra: Vec::new(),
        }
    }
}

impl Fixture {
    pub fn write(&self) -> tempfile::NamedTempFile {
        assert!(self.side >= 32);
        assert!((self.side * u32::from(self.bits)).is_multiple_of(8));
        let short = |v: u16| v.to_le_bytes().to_vec();
        let long = |v: u32| v.to_le_bytes().to_vec();
        let rationals = |v: &[(u32, u32)]| -> Vec<u8> {
            v.iter()
                .flat_map(|(n, d)| [n.to_le_bytes(), d.to_le_bytes()].concat())
                .collect()
        };
        // XYZ -> linear sRGB camera primaries; this is declared metadata, not a real camera profile.
        let matrix: [f64; 9] = [
            3.2404542, -1.5371385, -0.4985314, -0.9692660, 1.8760108, 0.0415560, 0.0556434,
            -0.2040259, 1.0572252,
        ];
        let matrix = matrix
            .into_iter()
            .flat_map(|v| {
                [
                    ((v * 1_000_000.) as i32).to_le_bytes(),
                    1_000_000i32.to_le_bytes(),
                ]
                .concat()
            })
            .collect();
        let mut entries = vec![
            (254, 4, long(0)),
            (256, 4, long(self.side)),
            (257, 4, long(self.side)),
            (258, 3, short(self.bits)),
            (259, 3, short(if self.lossless_jpeg { 7 } else { 1 })),
            (262, 3, short(32803)),
            (271, 2, b"Antitone\0".to_vec()),
            (272, 2, b"SDK adapter fixture\0".to_vec()),
            (273, 4, long(0)),
            (274, 3, short(self.orientation)),
            (277, 3, short(1)),
            (278, 4, long(self.side)),
            (
                279,
                4,
                long(self.side * self.side * u32::from(self.bits) / 8),
            ),
            (284, 3, short(1)),
            (33421, 3, [short(2), short(2)].concat()),
            (33422, 1, self.pattern.to_vec()),
            (50706, 1, vec![1, 7, 1, 0]),
            (50707, 1, vec![1, 3, 0, 0]),
            (50708, 2, b"Antitone SDK adapter fixture\0".to_vec()),
            (50710, 1, vec![0, 1, 2]),
            (50711, 3, short(1)),
            (50713, 3, [short(1), short(1)].concat()),
            (50714, 5, rationals(&[(64, 1)])),
            (50717, 4, long(4160)),
            (50718, 5, rationals(&[(1, 1), (1, 1)])),
            (50719, 4, [long(3), long(5)].concat()),
            (
                50720,
                4,
                [long(self.side - 16), long(self.side - 18)].concat(),
            ),
            (50721, 10, matrix),
            (50728, 5, rationals(&self.neutral)),
            (50778, 3, short(21)),
            (
                50829,
                4,
                [long(4), long(6), long(self.side - 4), long(self.side - 2)].concat(),
            ),
        ];
        for (stage, opcodes) in self.lists.iter().enumerate() {
            if !opcodes.is_empty() {
                let mut bytes = (opcodes.len() as u32).to_be_bytes().to_vec();
                for opcode in opcodes {
                    bytes.extend(opcode.bytes());
                }
                entries.push(([51008, 51009, 51022][stage], 7, bytes));
            }
        }
        for (tag, kind, data) in &self.extra {
            entries.retain(|(t, _, _)| t != tag);
            entries.push((*tag, *kind, data.clone()));
        }
        entries.sort_by_key(|entry| entry.0);
        let mut bytes = b"II\x2a\x00\x08\x00\x00\x00".to_vec();
        bytes.extend((entries.len() as u16).to_le_bytes());
        bytes.resize(10 + entries.len() * 12 + 4, 0);
        let mut strip_slot = 0;
        let mut strip_count_slot = 0;
        for (i, (tag, kind, data)) in entries.iter().enumerate() {
            let at = 10 + 12 * i;
            let size = match kind {
                3 => 2,
                4 => 4,
                5 | 10 => 8,
                _ => 1,
            };
            bytes[at..at + 2].copy_from_slice(&tag.to_le_bytes());
            bytes[at + 2..at + 4].copy_from_slice(&kind.to_le_bytes());
            bytes[at + 4..at + 8].copy_from_slice(&(data.len() as u32 / size).to_le_bytes());
            if *tag == 273 {
                strip_slot = at + 8;
            }
            if *tag == 279 {
                strip_count_slot = at + 8;
            }
            if data.len() <= 4 {
                bytes[at + 8..at + 8 + data.len()].copy_from_slice(data);
            } else {
                let position = bytes.len() as u32;
                bytes[at + 8..at + 12].copy_from_slice(&position.to_le_bytes());
                bytes.extend(data);
                if !bytes.len().is_multiple_of(2) {
                    bytes.push(0);
                }
            }
        }
        let position = bytes.len() as u32;
        bytes[strip_slot..strip_slot + 4].copy_from_slice(&position.to_le_bytes());
        let mut jpeg_samples = Vec::new();
        let mut packed = 0u32;
        let mut packed_bits = 0u16;
        for y in 0..self.side {
            for x in 0..self.side {
                let channel = self.pattern[(y % 2 * 2 + x % 2) as usize] as usize;
                let mut value = 64
                    + [1024_u16, 1536, 2048][channel]
                    + if self.ramp { (4 * x + 2 * y) as u16 } else { 0 };
                if let Some((mx, my, code)) = self.marker
                    && x == mx
                    && y == my
                {
                    value = code;
                }
                if self.lossless_jpeg {
                    jpeg_samples.push(value);
                } else if self.bits == 16 {
                    bytes.extend(value.to_le_bytes());
                } else {
                    // TIFF/DNG packed integer samples are MSB-first, independent of IFD byte order.
                    packed = (packed << self.bits) | u32::from(value);
                    packed_bits += self.bits;
                    while packed_bits >= 8 {
                        packed_bits -= 8;
                        bytes.push((packed >> packed_bits) as u8);
                    }
                    packed &= (1 << packed_bits) - 1;
                }
            }
        }
        if self.lossless_jpeg {
            assert_eq!(
                self.side, 64,
                "authored JPEG encoder supports only 64x64 frames"
            );
            assert_eq!(
                self.bits, 14,
                "authored JPEG encoder supports only 14-bit samples"
            );
            let jpeg = lossless_jpeg(&jpeg_samples);
            bytes[strip_count_slot..strip_count_slot + 4]
                .copy_from_slice(&(jpeg.len() as u32).to_le_bytes());
            bytes.extend(jpeg);
        }
        let mut file = tempfile::Builder::new().suffix(".dng").tempfile().unwrap();
        file.write_all(&bytes).unwrap();
        file.flush().unwrap();
        file
    }
}

// One-component 14-bit lossless JPEG: predictor 1, a fixed canonical DC table,
// no restarts or point transform. Independent of the SDK's writer/decoder.
fn lossless_jpeg(samples: &[u16]) -> Vec<u8> {
    fn segment(output: &mut Vec<u8>, marker: u8, payload: &[u8]) {
        output.extend([0xff, marker]);
        output.extend((payload.len() as u16 + 2).to_be_bytes());
        output.extend(payload);
    }
    assert_eq!(samples.len(), 64 * 64);
    let mut output = vec![0xff, 0xd8];
    segment(&mut output, 0xc3, &[14, 0, 64, 0, 64, 1, 1, 0x11, 0]);
    let mut table = vec![0; 17];
    table[4] = 15; // Categories 0..14 use the four-bit codes 0000..1110.
    table.extend(0_u8..15);
    segment(&mut output, 0xc4, &table);
    segment(&mut output, 0xda, &[1, 1, 0, 1, 0, 0]);
    let mut bits = Vec::new();
    let mut push = |value: u32, count: u32| {
        for bit in (0..count).rev() {
            bits.push(value & (1 << bit) != 0);
        }
    };
    for (i, value) in samples.iter().copied().enumerate() {
        assert!(value < 1 << 14);
        let predictor = if i == 0 {
            1 << 13
        } else if i % 64 == 0 {
            i32::from(samples[i - 64])
        } else {
            i32::from(samples[i - 1])
        };
        let difference = i32::from(value) - predictor;
        let category = u32::BITS - difference.unsigned_abs().leading_zeros();
        push(category, 4);
        let amplitude = if difference < 0 {
            (1 << category) - 1 + difference
        } else {
            difference
        };
        push(amplitude as u32, category);
    }
    while !bits.len().is_multiple_of(8) {
        bits.push(true);
    }
    for byte in bits.as_chunks::<8>().0 {
        let value = byte.iter().fold(0_u8, |v, bit| (v << 1) | u8::from(*bit));
        output.push(value);
        if value == 0xff {
            output.push(0);
        }
    }
    output.extend([0xff, 0xd9]);
    output
}
