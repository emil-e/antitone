//! Test-only ready-to-encode sink. No file is opened and no codec/destination work is scored.
use crate::{
    Error,
    export::{self, ImageSize},
    rendering::OutputTarget,
};
use std::{cell::RefCell, time::Instant};

pub(crate) struct Pixels {
    pub size: ImageSize,
    pub target: OutputTarget,
    pub codes: Vec<[u16; 3]>,
    pub ready: Instant,
}

thread_local! {
    static ACTIVE: RefCell<Option<Option<Pixels>>> = const { RefCell::new(None) };
}

pub(crate) fn capture<T>(run: impl FnOnce() -> T) -> (T, Option<Pixels>) {
    ACTIVE.with(|active| {
        let mut active = active.borrow_mut();
        assert!(
            active.is_none(),
            "pixel captures must not overlap on one thread"
        );
        *active = Some(None);
    });
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            ACTIVE.with(|active| {
                active.borrow_mut().take();
            });
        }
    }
    let _reset = Reset;
    let result = run();
    let pixels = ACTIVE.with(|active| active.borrow_mut().take().unwrap());
    (result, pixels)
}

pub(crate) fn intercept(
    size: ImageSize,
    target: OutputTarget,
    order: export::BlockOrder,
    block: &mut impl FnMut([u32; 2], [u32; 2]) -> Result<Vec<[u16; 3]>, Error>,
    before_complete: impl FnOnce() -> Result<(), Error>,
) -> Option<Result<(), Error>> {
    if !ACTIVE.with(|active| active.borrow().is_some()) {
        return None;
    }
    Some((|| {
        let count = u64::from(size.width) * u64::from(size.height);
        if count * 6 > 1024 * 1024 * 1024 {
            return Err(Error::Export(
                "TIFF RGB16 buffer exceeds the current 1 GiB export budget".into(),
            ));
        }
        let mut codes = super::timed!(FrameAllocation, vec![[0; 3]; count as usize]);
        for (origin, extent) in export::ordered_blocks(size, order) {
            let encoded = block(origin, extent)?;
            if encoded.len() != (extent[0] * extent[1]) as usize {
                return Err(Error::InvalidParameter("invalid TIFF block length"));
            }
            super::timed!(FormatPack, {
                for (i, pixel) in encoded.into_iter().enumerate() {
                    let x = origin[0] + i as u32 % extent[0];
                    let y = origin[1] + i as u32 / extent[0];
                    codes[(y * size.width + x) as usize] = pixel;
                }
            });
        }
        before_complete()?;
        ACTIVE.with(|active| {
            let previous = active.borrow_mut().as_mut().unwrap().replace(Pixels {
                size,
                target,
                codes,
                ready: Instant::now(),
            });
            assert!(
                previous.is_none(),
                "only one ready-to-encode image per capture"
            );
        });
        Ok(())
    })())
}

impl Pixels {
    pub fn write(&self, path: &std::path::Path) -> Result<(), Error> {
        export::tiff_encoded(
            path,
            self.size,
            self.target,
            export::Overwrite::Confirmed,
            |origin, extent| {
                let mut block = Vec::with_capacity((extent[0] * extent[1]) as usize);
                for y in origin[1]..origin[1] + extent[1] {
                    let first = (y * self.size.width + origin[0]) as usize;
                    block.extend_from_slice(&self.codes[first..first + extent[0] as usize]);
                }
                Ok(block)
            },
        )
    }
}

#[test]
fn receipt_is_explicit_and_failure_or_unwind_cannot_publish_partial_pixels() {
    let size = ImageSize {
        width: 131,
        height: 3,
    };
    let target = OutputTarget::DisplayP3;
    let make = |origin: [u32; 2], extent: [u32; 2]| {
        Ok((0..extent[0] * extent[1])
            .map(|i| {
                [
                    origin[0] as u16 + (i % extent[0]) as u16,
                    (origin[1] + i / extent[0]) as u16,
                    7,
                ]
            })
            .collect())
    };
    assert!(
        intercept(
            size,
            target,
            export::BlockOrder::RowMajor,
            &mut |_, _| unreachable!(),
            || Ok(())
        )
        .is_none()
    );
    let started = Instant::now();
    let (result, pixels) = capture(|| {
        intercept(
            size,
            target,
            export::BlockOrder::Spatial2x2,
            &mut { make },
            || Ok(()),
        )
        .unwrap()
    });
    result.unwrap();
    let pixels = pixels.unwrap();
    assert!(pixels.ready >= started);
    assert_eq!((pixels.size.width, pixels.size.height), (131, 3));
    assert_eq!(pixels.target, target);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ready-codes.tif");
    pixels.write(&path).unwrap();
    let mut decoder =
        tiff::decoder::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()))
            .unwrap();
    assert_eq!(decoder.dimensions().unwrap(), (131, 3));
    assert_eq!(
        decoder
            .get_tag_u8_vec(tiff::tags::Tag::Unknown(34675))
            .unwrap(),
        crate::profiles::icc(target)
    );
    let tiff::decoder::DecodingResult::U16(decoded) = decoder.read_image().unwrap() else {
        panic!("RGB16 receipt changed precision");
    };
    assert!(
        decoded
            .iter()
            .copied()
            .eq(pixels.codes.iter().flatten().copied())
    );
    for (i, pixel) in pixels.codes.iter().enumerate() {
        assert_eq!(*pixel, [(i % 131) as u16, (i / 131) as u16, 7]);
    }
    let (result, pixels) = capture(|| {
        intercept(
            size,
            target,
            export::BlockOrder::RowMajor,
            &mut |_, _| Err(Error::Gpu("stop".into())),
            || Ok(()),
        )
        .unwrap()
    });
    assert!(result.is_err());
    assert!(pixels.is_none());
    assert!(std::panic::catch_unwind(|| capture(|| panic!("stop"))).is_err());
    assert!(!ACTIVE.with(|active| active.borrow().is_some()));
}
