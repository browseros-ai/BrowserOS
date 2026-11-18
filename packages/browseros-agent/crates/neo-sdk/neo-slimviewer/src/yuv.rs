//! BGRX (X's 32-bit ZPixmap, little-endian) to I420, BT.709 limited range, for a band of rows.
//! Only the rows the screen changed in are converted; the rest of the picture stays as it was.

pub struct I420 {
    pub width: usize,
    pub height: usize,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}

impl I420 {
    /// Both sides must be even.
    pub fn new(width: usize, height: usize) -> I420 {
        let chroma = (width / 2) * (height / 2);
        I420 {
            width,
            height,
            y: vec![16; width * height],
            u: vec![128; chroma],
            v: vec![128; chroma],
        }
    }

    /// Converts rows `y0..y1` (rounded out to even rows) of `bgrx`, whose rows are `stride` bytes.
    pub fn convert(&mut self, bgrx: &[u8], stride: usize, y0: usize, y1: usize) {
        let (w, cw) = (self.width, self.width / 2);
        let y0 = y0 & !1;
        let y1 = ((y1 + 1) & !1).min(self.height);
        for pair in (y0..y1).step_by(2) {
            let top = &bgrx[pair * stride..pair * stride + w * 4];
            let bottom = &bgrx[(pair + 1) * stride..(pair + 1) * stride + w * 4];
            luma_row(top, &mut self.y[pair * w..(pair + 1) * w]);
            luma_row(bottom, &mut self.y[(pair + 1) * w..(pair + 2) * w]);
            let row = (pair / 2) * cw;
            let (us, vs) = (&mut self.u[row..row + cw], &mut self.v[row..row + cw]);
            for (i, (u, v)) in us.iter_mut().zip(vs.iter_mut()).enumerate() {
                let (a, b) = (&top[i * 8..i * 8 + 8], &bottom[i * 8..i * 8 + 8]);
                let blue = i32::from(a[0]) + i32::from(a[4]) + i32::from(b[0]) + i32::from(b[4]);
                let green = i32::from(a[1]) + i32::from(a[5]) + i32::from(b[1]) + i32::from(b[5]);
                let red = i32::from(a[2]) + i32::from(a[6]) + i32::from(b[2]) + i32::from(b[6]);
                // Sums of four pixels: shift by 10 instead of 8.
                *u = ((-26 * red - 86 * green + 112 * blue + (128 << 10) + 512) >> 10).clamp(0, 255)
                    as u8;
                *v = ((112 * red - 102 * green - 10 * blue + (128 << 10) + 512) >> 10).clamp(0, 255)
                    as u8;
            }
        }
    }
}

impl I420 {
    /// Like `convert`, into a picture half the screen's width and height: each output pixel is
    /// the mean of a 2×2 block (each chroma sample of a 4×4 block). Four times fewer pixels for
    /// the encoder while the screen moves under a tight CPU or link.
    pub fn convert_half(&mut self, bgrx: &[u8], stride: usize, y0: usize, y1: usize) {
        let (w, cw) = (self.width, self.width / 2);
        let r0 = (y0 / 4) * 2;
        let r1 = y1.div_ceil(4).saturating_mul(2).min(self.height);
        let (mut rs, mut gs, mut bs) = (vec![0i32; cw], vec![0i32; cw], vec![0i32; cw]);
        for out in (r0..r1).step_by(2) {
            rs.fill(0);
            gs.fill(0);
            bs.fill(0);
            for row in out..out + 2 {
                let a = &bgrx[2 * row * stride..2 * row * stride + 4 * w * 2];
                let b = &bgrx[(2 * row + 1) * stride..(2 * row + 1) * stride + 4 * w * 2];
                for x in 0..w {
                    let (p, q) = (&a[x * 8..x * 8 + 8], &b[x * 8..x * 8 + 8]);
                    let blue =
                        i32::from(p[0]) + i32::from(p[4]) + i32::from(q[0]) + i32::from(q[4]);
                    let green =
                        i32::from(p[1]) + i32::from(p[5]) + i32::from(q[1]) + i32::from(q[5]);
                    let red = i32::from(p[2]) + i32::from(p[6]) + i32::from(q[2]) + i32::from(q[6]);
                    self.y[row * w + x] =
                        ((47 * red + 157 * green + 16 * blue + (16 << 10) + 512) >> 10) as u8;
                    rs[x / 2] += red;
                    gs[x / 2] += green;
                    bs[x / 2] += blue;
                }
            }
            let crow = (out / 2) * cw;
            for i in 0..cw {
                let (red, green, blue) = (rs[i], gs[i], bs[i]);
                // Sums of sixteen pixels: shift by 12.
                self.u[crow + i] = ((-26 * red - 86 * green + 112 * blue + (128 << 12) + 2048)
                    >> 12)
                    .clamp(0, 255) as u8;
                self.v[crow + i] = ((112 * red - 102 * green - 10 * blue + (128 << 12) + 2048)
                    >> 12)
                    .clamp(0, 255) as u8;
            }
        }
    }
}

fn luma_row(bgrx: &[u8], out: &mut [u8]) {
    let (pixels, _) = bgrx.as_chunks::<4>();
    for (px, y) in pixels.iter().zip(out) {
        let (b, g, r) = (u32::from(px[0]), u32::from(px[1]), u32::from(px[2]));
        *y = ((47 * r + 157 * g + 16 * b + (16 << 8) + 128) >> 8) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_and_black_hit_the_limited_range_ends() {
        let mut pic = I420::new(2, 2);
        pic.convert(&[255u8; 16], 8, 0, 2);
        assert_eq!((pic.y[0], pic.u[0], pic.v[0]), (235, 128, 128));
        pic.convert(&[0u8; 16], 8, 0, 2);
        assert_eq!((pic.y[0], pic.u[0], pic.v[0]), (16, 128, 128));
    }

    #[test]
    fn half_size_averages_blocks() {
        // 4x4 screen, white left half, black right half → 2x2 picture: white, black.
        let mut screen = vec![0u8; 4 * 4 * 4];
        for row in 0..4 {
            for x in 0..2 {
                screen[row * 16 + x * 4..row * 16 + x * 4 + 4].fill(255);
            }
        }
        let mut pic = I420::new(2, 2);
        pic.convert_half(&screen, 16, 0, 4);
        assert_eq!(&pic.y[..], &[235, 16, 235, 16]);
        assert_eq!((pic.u[0], pic.v[0]), (128, 128));
    }
}
