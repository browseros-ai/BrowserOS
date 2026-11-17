use super::*;

fn backends() -> impl Iterator<Item = Backend> {
    [Backend::X264, Backend::OpenH264]
        .into_iter()
        .filter(|backend| backend.available())
}

fn settings(backend: Backend) -> Settings {
    Settings {
        backend,
        preset: "ultrafast".into(),
        threads: 1,
        crf: 24.0,
    }
}

fn picture(width: usize, height: usize) -> I420 {
    let mut pic = I420::new(width, height);
    for (i, y) in pic.y.iter_mut().enumerate() {
        let (x, row) = (i % width, i / width);
        *y = if (x / 3 + row / 5).is_multiple_of(3) {
            30
        } else {
            220
        };
    }
    pic
}

fn nal_types(frame: &Encoded) -> Vec<u8> {
    frame
        .data
        .windows(4)
        .filter(|w| w[..3] == [0, 0, 1])
        .map(|w| w[3] & 31)
        .collect()
}

fn check_idr(frame: &Encoded) {
    assert!(frame.key);
    assert!(!frame.delayed);
    let nals = nal_types(frame);
    let sps = nals.iter().position(|n| *n == 7).expect("SPS");
    let pps = nals.iter().position(|n| *n == 8).expect("PPS");
    let idr = nals.iter().position(|n| *n == 5).expect("IDR");
    assert!(sps < pps && pps < idr);
}

#[test]
fn tests_that_only_built_encoders_can_be_selected() {
    assert!(Backend::default_name().parse::<Backend>().is_ok());
    assert!(
        "invalid"
            .parse::<Backend>()
            .unwrap_err()
            .contains("--encoder")
    );
    for (name, built) in [
        ("x264", cfg!(feature = "x264")),
        ("openh264", cfg!(feature = "openh264")),
    ] {
        let result = name.parse::<Backend>();
        assert_eq!(result.is_ok(), built);
        if let Err(error) = result {
            assert!(error.contains("not built in"));
        }
    }
}

#[test]
fn tests_that_phone_and_half_size_frames_have_headers_and_no_delay() {
    for backend in backends() {
        for (w, h, rate) in [(780, 1688, 0), (390, 844, 1200)] {
            let mut enc = Encoder::open(w, h, &settings(backend), 24.0, rate).unwrap();
            let mut pic = picture(w.into(), h.into());
            check_idr(&enc.encode(&pic, true, None, 500).unwrap());
            pic.y[0] = 100;
            let next = enc.encode(&pic, false, None, 534).unwrap();
            assert!(!next.key);
            assert!(!next.delayed);
            assert!(nal_types(&next).contains(&1));
            check_idr(&enc.encode(&pic, true, None, 568).unwrap());
        }
    }
}

#[test]
fn tests_that_still_refresh_and_live_rate_changes_keep_encoding() {
    for backend in backends() {
        let mut enc = Encoder::open(160, 128, &settings(backend), 36.0, 0).unwrap();
        let pic = picture(160, 128);
        check_idr(&enc.encode(&pic, true, None, 0).unwrap());
        let repeat = enc.encode(&pic, false, None, 33).unwrap();
        let sharp = enc.encode(&pic, false, Some(18), 66).unwrap();
        assert!(!sharp.delayed);
        assert!(sharp.data.len() > repeat.data.len());
        if backend == Backend::OpenH264 {
            check_idr(&sharp);
        }
        enc.set_rate(30.0, 900);
        let next = enc.encode(&pic, false, None, 100).unwrap();
        assert!(!next.key);
        assert!(!next.delayed);
        // OpenH264 bounds compressed-frame storage; QP 0 is tested on a flat image.
        check_idr(
            &enc.encode(&I420::new(160, 128), true, Some(0), 133)
                .unwrap(),
        );
    }
}

#[test]
fn tests_that_lower_quality_reduces_frame_size() {
    for backend in backends() {
        let pic = picture(160, 128);
        let encode = |quality| {
            Encoder::open(160, 128, &settings(backend), quality, 0)
                .unwrap()
                .encode(&pic, true, None, 0)
                .unwrap()
                .data
                .len()
        };
        assert!(encode(18.0) > encode(42.0), "{backend:?}");
    }
}

#[test]
fn tests_that_invalid_input_is_rejected_before_native_reads() {
    for backend in backends() {
        let settings = settings(backend);
        for (w, h, crf, rate) in [
            (0, 128, 24.0, 0),
            (159, 128, 24.0, 0),
            (160, 127, 24.0, 0),
            (160, 128, f32::NAN, 0),
            (160, 128, 52.0, 0),
            (160, 128, 24.0, u32::MAX),
        ] {
            assert!(Encoder::open(w, h, &settings, crf, rate).is_none());
        }
        let mut enc = Encoder::open(160, 128, &settings, 24.0, 0).unwrap();
        assert!(enc.encode(&picture(160, 126), true, None, 0).is_none());
        let mut pic = picture(160, 128);
        pic.u.pop();
        assert!(enc.encode(&pic, true, None, 0).is_none());
        pic.u.push(128);
        assert!(enc.encode(&pic, true, Some(52), 0).is_none());
        check_idr(&enc.encode(&pic, true, None, 0).unwrap());
    }
}

#[test]
fn tests_that_consecutive_noise_frames_report_their_actual_idr_type() {
    if !Backend::OpenH264.available() {
        return;
    }
    for (w, h, crf, rate) in [(780, 1688, 24.0, 0), (390, 844, 36.0, 3000)] {
        let mut enc = Encoder::open(w, h, &settings(Backend::OpenH264), crf, rate).unwrap();
        let mut pic = I420::new(w.into(), h.into());
        let mut bgra = vec![255; usize::from(w) * usize::from(h) * 4];
        let mut seed = 12345_u32;
        let mut keys = Vec::new();
        let mut sizes = Vec::new();
        for frame in 0..12 {
            for pixel in bgra.as_chunks_mut::<4>().0 {
                for channel in &mut pixel[..3] {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    *channel = (seed >> 24) as u8;
                }
            }
            pic.convert(&bgra, usize::from(w) * 4, 0, usize::from(h));
            let out = enc.encode(&pic, frame == 0, None, frame * 33).unwrap();
            if frame == 0 {
                check_idr(&out);
            }
            assert!(!out.delayed);
            assert_eq!(out.key, nal_types(&out).contains(&5));
            if out.key {
                keys.push(frame);
            }
            sizes.push(out.data.len());
        }
        eprintln!(
            "noise OpenH264 {w}x{h} crf={crf} rate={rate}: requested IDR=[0], actual IDR={keys:?}, bytes={sizes:?}"
        );
    }
}
