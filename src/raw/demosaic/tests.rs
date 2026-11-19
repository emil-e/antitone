use super::*;

unsafe extern "C" fn reject(
    context: *mut c_void,
    image: *mut u16,
    width: u32,
    height: u32,
    pattern: *const u8,
) -> c_int {
    // A deliberately failing accelerator must receive the prepared Bayer image and
    // leave it intact for CPU recovery. Keep assertions within the unwind boundary.
    match catch_unwind(AssertUnwindSafe(|| {
        assert_eq!([width, height], [64, 64]);
        assert!(!image.is_null());
        // SAFETY: this descriptor owns its counter and the bridge lends four CFA bytes.
        unsafe {
            assert_eq!(*pattern.cast::<[u8; 4]>(), [0, 1, 1, 2]);
            *context.cast::<usize>() += 1;
        }
    })) {
        Ok(()) => 1,
        Err(_) => 2,
    }
}

unsafe extern "C" fn replace(
    _context: *mut c_void,
    image: *mut u16,
    width: u32,
    height: u32,
    _pattern: *const u8,
) -> c_int {
    // Distinct missing-channel values cannot survive a second CFA interpolation.
    let image = unsafe {
        std::slice::from_raw_parts_mut(image.cast::<[u16; 4]>(), width as usize * height as usize)
    };
    for (i, pixel) in image.iter_mut().enumerate() {
        let x = i % width as usize;
        *pixel = [if x.is_multiple_of(2) { 999 } else { 111 }, 222, 333, 444];
    }
    0
}

#[test]
fn native_success_consumes_replacement_without_running_cpu_demosaic_again() {
    let file = crate::raw::sony_fixture::source_with_pixels(|_, _| 1024);
    let mut raw = RawFile::open(file.path()).unwrap();
    raw.ensure_unpacked().unwrap();
    let callback = NativeDemosaicer {
        context: std::ptr::null_mut(),
        run: replace,
    };
    let mut error = [0; 512];
    let mut view = std::mem::MaybeUninit::<NativeView>::zeroed();
    let status = unsafe {
        antitone_raw_develop_with_demosaic(
            raw.handle.as_ptr(),
            [1.; 4].as_ptr(),
            0,
            2,
            &callback,
            view.as_mut_ptr(),
            error.as_mut_ptr(),
            error.len(),
        )
    };
    assert_eq!(status, 0);
    let view = unsafe { view.assume_init() };
    assert_eq!(view.custom_demosaic, 1);
    assert!(view.rgb.is_null());
    let pixels = unsafe {
        std::slice::from_raw_parts(
            view.pixels.cast::<[u16; 4]>(),
            view.width as usize * view.height as usize,
        )
    };
    for (i, pixel) in pixels.iter().enumerate() {
        let x = i % view.width as usize;
        assert_eq!(
            *pixel,
            [if x.is_multiple_of(2) { 999 } else { 111 }, 222, 333, 444]
        );
    }
    raw.develop_camera([1.; 4]).unwrap();
    assert_eq!(
        raw.demosaic_backend(),
        Some(crate::raw::DemosaicBackend::LibRaw)
    );
}

#[test]
fn native_callback_failure_uses_cpu_and_borrow_is_cleared_before_next_development() {
    let file = crate::raw::sony_fixture::source_with_pixels(|x, y| {
        512 + [837, 537, 537, 133][((y % 2) * 2 + x % 2) as usize]
    });
    let mut reference = RawFile::open(file.path()).unwrap();
    reference.develop_camera([1.; 4]).unwrap();
    let expected = reference.camera_view.unwrap();
    let mut raw = RawFile::open(file.path()).unwrap();
    raw.ensure_unpacked().unwrap();
    let mut count = 0usize;
    let callback = NativeDemosaicer {
        context: std::ptr::from_mut(&mut count).cast(),
        run: reject,
    };
    let mut error = [0; 512];
    let mut view = std::mem::MaybeUninit::<NativeView>::zeroed();
    // SAFETY: raw, callback/counter and output storage outlive this synchronous call.
    let status = unsafe {
        antitone_raw_develop_with_demosaic(
            raw.handle.as_ptr(),
            [1.; 4].as_ptr(),
            0,
            2,
            &callback,
            view.as_mut_ptr(),
            error.as_mut_ptr(),
            error.len(),
        )
    };
    assert_eq!(status, 0);
    assert_eq!(
        count, 1,
        "the failing accelerator must actually have been called"
    );
    let actual = unsafe { view.assume_init() };
    assert_eq!(actual.custom_demosaic, 0);
    assert_eq!(
        [actual.width, actual.height],
        [expected.width, expected.height]
    );
    assert_eq!(actual.pre_mul, expected.pre_mul);
    assert_eq!(actual.maximum, expected.maximum);
    assert_eq!(actual.data_maximum, expected.data_maximum);
    assert!(actual.rgb.is_null() && expected.rgb.is_null());
    let count_pixels = actual.width as usize * actual.height as usize * 4;
    // SAFETY: both native owners retain their complete output through this comparison.
    unsafe {
        assert_eq!(
            std::slice::from_raw_parts(actual.pixels, count_pixels),
            std::slice::from_raw_parts(expected.pixels, count_pixels)
        );
    }
    // A subsequent native CPU call must not invoke the former borrowed descriptor.
    raw.develop_camera([1.; 4]).unwrap();
    assert_eq!(count, 1);
}
