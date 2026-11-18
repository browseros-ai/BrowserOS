/// splitmix64: a seeded stream of 64-bit values. The tests' random batches draw from it, so a
/// failure names a seed that reproduces it.
pub(crate) struct SplitMix64(pub(crate) u64);

impl Iterator for SplitMix64 {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        Some(mixed ^ (mixed >> 31))
    }
}
