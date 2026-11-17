/// `Math.round(x)`: the integer closest to `x`, a half going up toward +∞ (`2.5` to `3`, `-2.5`
/// to `-2`), a zero result keeping `x`'s sign (`-0.4` to `-0`), and `NaN`, the infinities and the
/// zeros unchanged. Rust's `f64::round` takes a half away from zero, so `-2.5` would be `-3`.
pub(crate) fn math_round(x: f64) -> f64 {
    let floor = x.floor();
    // Not `(x + 0.5).floor()`: that sum rounds 0.49999999999999994 up to 1, and an odd integer
    // past 2^52 up to the next even one.
    let up = if x - floor >= 0.5 { floor + 1.0 } else { floor };
    // Math.round's result always has `x`'s sign: a negative `x` above -0.5 rounds to -0.
    up.copysign(x)
}
