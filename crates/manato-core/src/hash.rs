/// Deterministic state hasher used for rollback checksums and tests.
///
/// Implementation is a 128-bit FNV-1a variant. It never hashes pointers or
/// allocation addresses, only logical state.
#[derive(Debug, Clone)]
pub struct StateHasher {
    a: u64,
    b: u64,
}

impl Default for StateHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl StateHasher {
    pub fn new() -> Self {
        Self {
            a: 0xcbf2_9ce4_8422_2325,
            b: 0x9e37_79b9_7f4a_7c15,
        }
    }

    #[inline]
    pub fn write_u8(&mut self, v: u8) {
        self.a ^= v as u64;
        self.a = self.a.wrapping_mul(0x100_0000_01b3);
        self.b ^= (v as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        self.b = self.b.rotate_left(27).wrapping_mul(0x94D0_49BB_1331_11EB);
    }

    #[inline]
    pub fn write_u32(&mut self, v: u32) {
        for b in v.to_le_bytes() {
            self.write_u8(b);
        }
    }

    #[inline]
    pub fn write_i32(&mut self, v: i32) {
        self.write_u32(v as u32);
    }

    #[inline]
    pub fn write_u64(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.write_u8(b);
        }
    }

    #[inline]
    pub fn write_i64(&mut self, v: i64) {
        self.write_u64(v as u64);
    }

    #[inline]
    pub fn write_f32(&mut self, v: f32) {
        // Normalise -0.0 and NaNs so equivalent states hash identically.
        let bits = if v == 0.0 {
            0.0f32.to_bits()
        } else if v.is_nan() {
            f32::NAN.to_bits()
        } else {
            v.to_bits()
        };
        self.write_u32(bits);
    }

    #[inline]
    pub fn write_bool(&mut self, v: bool) {
        self.write_u8(v as u8);
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u8(b);
        }
    }

    pub fn finish128(&self) -> u128 {
        ((self.a as u128) << 64) | self.b as u128
    }

    pub fn finish(&self) -> u64 {
        self.a ^ self.b
    }
}
