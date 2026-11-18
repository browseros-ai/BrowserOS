//! Compares single-thread OpenH264 bitstreams across feature-search modes.

use super::{Backend, Encoder, I420, settings};
use std::env;
use std::error::Error;
use std::io::Write;
use std::process::{Command, Stdio};

const WIDTH: usize = 384;
const HEIGHT: usize = 512;
const FRAMES: usize = 10;
const CHILD_SEQUENCE: &str = "SLIMVIEWER_TEST_FME_SEQUENCE";

fn text_page(height: usize) -> Vec<u8> {
    let mut page = vec![235; WIDTH * height];
    let mut seed = 1_u32;
    // Each line needs distinct glyphs so the detector can identify its displacement.
    let glyphs = [
        [0b1111, 0b1000, 0b1110, 0b1000, 0b1111],
        [0b1001, 0b1001, 0b1111, 0b1001, 0b1001],
        [0b1110, 0b1001, 0b1110, 0b1010, 0b1001],
        [0b1000, 0b1000, 0b1000, 0b1000, 0b1111],
    ];
    for top in (4..height - 10).step_by(16) {
        for left in (8..WIDTH - 8).step_by(10) {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let glyph = glyphs[(seed >> 30) as usize];
            for (row, bits) in glyph.iter().enumerate() {
                for col in 0..4 {
                    if bits & (8 >> col) != 0 {
                        for y in top + 2 * row..top + 2 * row + 2 {
                            let x = y * WIDTH + left + 2 * col;
                            page[x..x + 2].fill(24);
                        }
                    }
                }
            }
        }
    }
    page
}

fn encode_sequence(step: usize) -> Result<(), Box<dyn Error>> {
    let page = text_page(HEIGHT + (FRAMES - 1) * step);
    let mut pic = I420::new(WIDTH, HEIGHT);
    let mut enc = Encoder::open(
        WIDTH as u16,
        HEIGHT as u16,
        &settings(Backend::OpenH264),
        24.0,
        0,
    )
    .ok_or("could not open OpenH264")?;
    let mut hash = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let input = hash.stdin.as_mut().ok_or("missing sha256sum input")?;
    let mut bytes = 0;
    for frame in 0..FRAMES {
        let offset = frame * step * WIDTH;
        pic.y
            .copy_from_slice(&page[offset..offset + WIDTH * HEIGHT]);
        if step == 0 && frame % 2 == 1 {
            for y in 32..48 {
                pic.y[y * WIDTH + 64..y * WIDTH + 72].fill(24);
            }
        }
        let out = enc
            .encode(&pic, frame == 0, None, frame as i64 * 33)
            .ok_or("OpenH264 dropped a test frame")?;
        input.write_all(&out.data)?;
        bytes += out.data.len();
    }
    let sum = hash.wait_with_output()?;
    assert!(sum.status.success(), "could not hash the encoded stream");
    let digest = std::str::from_utf8(&sum.stdout)?
        .split_whitespace()
        .next()
        .ok_or("sha256sum did not report a digest")?;
    eprintln!("bitstream {bytes} {digest}");
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct Bitstream {
    bytes: usize,
    digest: String,
}

fn bitstream(mode: Option<&str>, sequence: &str) -> Result<Bitstream, Box<dyn Error>> {
    // OpenH264 caches the mode on first use, even across encoder instances.
    let mut child = Command::new(env::current_exe()?);
    child
        .args([
            "--exact",
            "encoder::tests::feature_search::modes_follow_page_motion",
            "--nocapture",
        ])
        .env(CHILD_SEQUENCE, sequence)
        .env_remove("SV_OPENH264_FME");
    if let Some(mode) = mode {
        child.env("SV_OPENH264_FME", mode);
    }
    let out = child.output()?;
    let stderr = String::from_utf8(out.stderr)?;
    assert!(
        out.status.success(),
        "{mode:?} {sequence}: {stderr}{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let result = stderr
        .lines()
        .find_map(|line| line.strip_prefix("bitstream "))
        .ok_or("encoder child did not report a bitstream")?;
    let (bytes, digest) = result.split_once(' ').ok_or("invalid bitstream summary")?;
    eprintln!("{} {sequence}: {result}", mode.unwrap_or("default"));
    Ok(Bitstream {
        bytes: bytes.parse()?,
        digest: digest.to_owned(),
    })
}

#[test]
fn modes_follow_page_motion() -> Result<(), Box<dyn Error>> {
    if let Ok(sequence) = env::var(CHILD_SEQUENCE) {
        return match sequence.as_str() {
            "caret" => encode_sequence(0),
            "scrolling" => encode_sequence(96),
            _ => Err("unknown test sequence".into()),
        };
    }

    // Unpatched OpenH264 2.7.0 at 1bca6231 produced these SHA-256 digests on
    // arm64 and x86-64 with one encoder thread and the same shim and inputs.
    for (sequence, upstream) in [
        (
            "caret",
            "cb2db3556bfab7eb4d781d14ab2fb260f4a83816825687e57411b0cf7975874b",
        ),
        (
            "scrolling",
            "f2131f212f36134b45fd88cc79148424b41d7abe83d4cf61713cc5d3f0db4b93",
        ),
    ] {
        let always = bitstream(Some("always"), sequence)?;
        let scroll = bitstream(Some("scroll"), sequence)?;
        let never = bitstream(Some("never"), sequence)?;
        let default = bitstream(None, sequence)?;
        if cfg!(any(target_arch = "aarch64", target_arch = "x86_64")) {
            assert_eq!(
                always.digest, upstream,
                "always must reproduce upstream on {sequence}"
            );
        }
        assert_eq!(
            default, scroll,
            "the default should follow scroll mode on {sequence}"
        );
        if sequence == "caret" {
            assert_eq!(
                scroll, never,
                "a blinking caret should leave search disabled"
            );
        } else {
            assert_eq!(scroll, always, "detected scroll must match upstream search");
            assert!(
                never.bytes > always.bytes,
                "disabling search should cost bytes on scrolling text"
            );
        }
    }
    Ok(())
}
