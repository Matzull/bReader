//! Reed-Solomon error correction over binary extension fields GF(2^m) with
//! `u16` symbols (m up to 12), as used by Aztec Code (ISO/IEC 24778).
//!
//! Block layout: `block[0]` is the highest-degree coefficient; the last
//! `ec_len` symbols are check symbols. The generator polynomial is
//! `g(x) = (x - a^f)(x - a^(f+1))...(x - a^(f + ec_len - 1))` with `a` the
//! root of the field polynomial and `f` the first root exponent; every valid
//! block evaluates to zero at those points.

use crate::{Correction, RsError};

/// Binary extension field with `Q = 2^m` elements, built from a primitive
/// polynomial, with generator polynomial roots starting at `a^first_root`.
#[derive(Debug)]
pub struct Gf2m<const Q: usize> {
    exp: [u16; Q],
    log: [u16; Q],
    first_root: usize,
}

/// Aztec mode message field: GF(16), x^4 + x + 1.
pub static AZTEC_PARAM: Gf2m<16> = Gf2m::new(0x13, 1);
/// Aztec data field for 6-bit codewords: GF(64), x^6 + x + 1.
pub static AZTEC_DATA_6: Gf2m<64> = Gf2m::new(0x43, 1);
/// Aztec data field for 8-bit codewords: GF(256), x^8 + x^5 + x^3 + x^2 + 1.
pub static AZTEC_DATA_8: Gf2m<256> = Gf2m::new(0x12d, 1);
/// Aztec data field for 10-bit codewords: GF(1024), x^10 + x^3 + 1.
pub static AZTEC_DATA_10: Gf2m<1024> = Gf2m::new(0x409, 1);
/// Aztec data field for 12-bit codewords: GF(4096), x^12 + x^6 + x^5 + x^3 + 1.
pub static AZTEC_DATA_12: Gf2m<4096> = Gf2m::new(0x1069, 1);

impl<const Q: usize> Gf2m<Q> {
    const ORDER: usize = Q - 1;

    /// Builds the field from `primitive`, the field polynomial including its
    /// leading term (e.g. `0x13` for x^4 + x + 1). Panics (a compile error in
    /// a static) if the polynomial is not primitive.
    pub const fn new(primitive: u32, first_root: usize) -> Self {
        assert!(Q.is_power_of_two() && Q >= 4 && Q <= 1 << 15);
        assert!(primitive as usize >> Q.trailing_zeros() == 1);
        let mut exp = [0u16; Q];
        let mut log = [0u16; Q];
        let mut x: u32 = 1;
        let mut i = 0;
        while i < Q - 1 {
            assert!(i == 0 || x != 1, "polynomial is not primitive");
            exp[i] = x as u16;
            log[x as usize] = i as u16;
            x <<= 1;
            if x as usize & Q != 0 {
                x ^= primitive;
            }
            i += 1;
        }
        assert!(x == 1, "polynomial is not primitive");
        Self {
            exp,
            log,
            first_root,
        }
    }

    #[inline]
    fn mul(&self, a: u16, b: u16) -> u16 {
        if a == 0 || b == 0 {
            0
        } else {
            let s = usize::from(self.log[usize::from(a)]) + usize::from(self.log[usize::from(b)]);
            self.exp[s % Self::ORDER]
        }
    }

    /// `a / b` for nonzero `b` (0 otherwise).
    #[inline]
    fn div(&self, a: u16, b: u16) -> u16 {
        if a == 0 || b == 0 {
            0
        } else {
            let s = usize::from(self.log[usize::from(a)]) + Self::ORDER
                - usize::from(self.log[usize::from(b)]);
            self.exp[s % Self::ORDER]
        }
    }

    /// alpha^e for any exponent.
    #[inline]
    fn alpha(&self, e: usize) -> u16 {
        self.exp[e % Self::ORDER]
    }

    /// Syndromes `S_j = block(a^(first_root + j))`; returns true if all are zero.
    fn syndromes(&self, block: &[u16], s: &mut [u16]) -> bool {
        let mut zero = true;
        for (j, out) in s.iter_mut().enumerate() {
            let root = self.alpha(self.first_root + j);
            let mut acc = 0u16;
            for &c in block {
                acc = self.mul(acc, root) ^ c;
            }
            *out = acc;
            zero &= acc == 0;
        }
        zero
    }

    /// Computes `ec_len` check symbols for `data` into `out`
    /// (`out.len() == ec_len`). Symbols must be below `Q`.
    pub fn encode(&self, data: &[u16], ec_len: usize, out: &mut [u16]) -> Result<(), RsError> {
        if out.len() != ec_len
            || data.len() + ec_len > Self::ORDER
            || data.iter().any(|&d| usize::from(d) >= Q)
        {
            return Err(RsError::InvalidInput);
        }
        if ec_len == 0 {
            return Ok(());
        }
        // g[i] = coefficient of x^i, monic of degree ec_len.
        let mut g = vec![0u16; ec_len + 1];
        g[0] = 1;
        for deg in 0..ec_len {
            let root = self.alpha(self.first_root + deg);
            for i in (0..=deg + 1).rev() {
                let lower = if i > 0 { g[i - 1] } else { 0 };
                g[i] = self.mul(g[i], root) ^ lower;
            }
        }
        out.fill(0);
        for &d in data {
            let fb = d ^ out[0];
            out.copy_within(1.., 0);
            out[ec_len - 1] = 0;
            if fb != 0 {
                for (k, o) in out.iter_mut().enumerate() {
                    *o ^= self.mul(fb, g[ec_len - 1 - k]);
                }
            }
        }
        Ok(())
    }

    /// Corrects `block` in place; the last `ec_len` symbols are check symbols
    /// and `erasures` lists known-bad positions. `block` is left untouched on
    /// failure.
    ///
    /// The result reports errors and erasures separately so that callers can
    /// apply a stricter bound than the code's own (`2 * errors + erasures <=
    /// ec_len`). Allocation is proportional to the block length.
    pub fn decode(
        &self,
        block: &mut [u16],
        ec_len: usize,
        erasures: &[usize],
    ) -> Result<Correction, RsError> {
        let n = block.len();
        if n > Self::ORDER
            || ec_len > n
            || erasures.len() > ec_len
            || block.iter().any(|&c| usize::from(c) >= Q)
        {
            return Err(RsError::InvalidInput);
        }
        let mut is_erasure = vec![false; n];
        for &e in erasures {
            if e >= n || is_erasure[e] {
                return Err(RsError::InvalidInput);
            }
            is_erasure[e] = true;
        }
        if ec_len == 0 {
            return Ok(Correction::default());
        }
        let mut s = vec![0u16; ec_len];
        if self.syndromes(block, &mut s) {
            return Ok(Correction::default());
        }
        let f = erasures.len();
        let l_max = ec_len + 1;

        // Erasure locator: prod (1 - X_k x), X_k = a^(n-1-pos).
        let mut lambda = vec![0u16; l_max];
        lambda[0] = 1;
        for (k, &pos) in erasures.iter().enumerate() {
            let x = self.alpha(n - 1 - pos);
            for i in (1..=k + 1).rev() {
                lambda[i] ^= self.mul(x, lambda[i - 1]);
            }
        }

        // Berlekamp-Massey seeded with the erasure locator.
        let mut b = lambda.clone();
        let mut l = f;
        let mut last_d = 1u16;
        let mut m = 1usize;
        for r in f..ec_len {
            let mut d = 0u16;
            for i in 0..=l.min(r) {
                d ^= self.mul(lambda[i], s[r - i]);
            }
            if d == 0 {
                m += 1;
                continue;
            }
            let coef = self.div(d, last_d);
            let prev = lambda.clone();
            for (i, &bi) in b.iter().enumerate() {
                if bi == 0 {
                    continue;
                }
                if i + m >= l_max {
                    return Err(RsError::Uncorrectable);
                }
                lambda[i + m] ^= self.mul(coef, bi);
            }
            if 2 * l <= r + f {
                l = r + 1 + f - l;
                b = prev;
                last_d = d;
                m = 1;
            } else {
                m += 1;
            }
        }

        let deg = lambda.iter().rposition(|&c| c != 0).unwrap_or(0);
        if deg == 0 || deg < f || deg > n || 2 * (deg - f) + f > ec_len {
            return Err(RsError::Uncorrectable);
        }

        // Chien search: position i has locator X = a^(n-1-i); root of lambda at X^-1.
        let mut locs = Vec::with_capacity(deg);
        for i in 0..n {
            let xinv = self.alpha(Self::ORDER - (n - 1 - i) % Self::ORDER);
            let mut acc = 0u16;
            for &c in lambda[..=deg].iter().rev() {
                acc = self.mul(acc, xinv) ^ c;
            }
            if acc == 0 {
                if locs.len() == deg {
                    return Err(RsError::Uncorrectable);
                }
                locs.push(i);
            }
        }
        if locs.len() != deg {
            return Err(RsError::Uncorrectable);
        }

        // Omega = S * lambda mod x^ec_len.
        let mut omega = vec![0u16; ec_len];
        for (k, o) in omega.iter_mut().enumerate() {
            let mut acc = 0u16;
            for i in 0..=k.min(deg) {
                acc ^= self.mul(lambda[i], s[k - i]);
            }
            *o = acc;
        }

        // Forney: e = X^(1 - first_root) * omega(X^-1) / lambda'(X^-1).
        let mut fixed = block.to_vec();
        let mut correction = Correction::default();
        let base = self.first_root % Self::ORDER;
        for &i in &locs {
            let p = n - 1 - i;
            let xinv = self.alpha(Self::ORDER - p % Self::ORDER);
            let mut num = 0u16;
            for &c in omega.iter().rev() {
                num = self.mul(num, xinv) ^ c;
            }
            // Formal derivative: only odd-degree terms survive.
            let mut den = 0u16;
            let mut k = 1;
            while k <= deg {
                den ^= self.mul(
                    lambda[k],
                    self.alpha((k - 1) * (Self::ORDER - p % Self::ORDER)),
                );
                k += 2;
            }
            if den == 0 {
                return Err(RsError::Uncorrectable);
            }
            let scale = self.alpha(p * (Self::ORDER + 1 - base));
            let delta = self.mul(scale, self.div(num, den));
            if delta == 0 {
                // A located position without a change means the locator is
                // spurious, unless it is an erasure that was received intact.
                if is_erasure[i] {
                    continue;
                }
                return Err(RsError::Uncorrectable);
            }
            fixed[i] ^= delta;
            if is_erasure[i] {
                correction.erasures += 1;
            } else {
                correction.errors += 1;
            }
        }
        if !self.syndromes(&fixed, &mut s) {
            return Err(RsError::Uncorrectable);
        }
        block.copy_from_slice(&fixed);
        Ok(correction)
    }
}

#[cfg(test)]
mod tests;
