//! Reed-Solomon error correction over GF(256), the binary extension fields of
//! Aztec Code (GF(16) to GF(4096)) and, for PDF417, GF(929).
//!
//! Block layout: `block[0]` is the highest-degree coefficient; the last
//! `ec_len` symbols are check symbols. The code's generator polynomial has
//! roots `alpha^base, ..., alpha^(base + ec_len - 1)`.
//!
//! Implementation crate of breader: not covered by SemVer; depend on `breader`, which re-exports what is public (docs/API-STABILITY.md).

#![forbid(unsafe_code)]

mod gf2m;
mod gf929;
pub use gf2m::{AZTEC_DATA_6, AZTEC_DATA_8, AZTEC_DATA_10, AZTEC_DATA_12, AZTEC_PARAM, Gf2m};
pub use gf929::{Correction, Gf929, MAX_BLOCK_LEN, MAX_EC_LEN, PDF417_FIELD};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RsError {
    /// More errors than the code can correct, or an inconsistent locator.
    Uncorrectable,
    /// `ec_len` larger than the block, block longer than 255, or erasure out of range.
    InvalidInput,
}

/// GF(2^8) defined by a primitive polynomial, with the generator polynomial
/// roots starting at `alpha^generator_base`.
#[derive(Debug)]
pub struct Gf256 {
    exp: [u8; 512],
    log: [u8; 256],
    generator_base: u8,
}

/// Field used by QR Code: x^8 + x^4 + x^3 + x^2 + 1, roots from alpha^0.
pub static QR_FIELD: Gf256 = Gf256::new(0x11d, 0);

const N: usize = 256;

impl Gf256 {
    pub const fn new(primitive: u16, generator_base: u8) -> Self {
        let mut exp = [0u8; 512];
        let mut log = [0u8; 256];
        let mut x: u16 = 1;
        let mut i = 0;
        while i < 255 {
            exp[i] = x as u8;
            log[x as usize] = i as u8;
            x <<= 1;
            if x & 0x100 != 0 {
                x ^= primitive;
            }
            x &= 0xff;
            i += 1;
        }
        while i < 512 {
            exp[i] = exp[i - 255];
            i += 1;
        }
        Self {
            exp,
            log,
            generator_base,
        }
    }

    #[inline]
    fn mul(&self, a: u8, b: u8) -> u8 {
        if a == 0 || b == 0 {
            0
        } else {
            self.exp[self.log[a as usize] as usize + self.log[b as usize] as usize]
        }
    }

    /// `a / b`; `b` must be nonzero (returns 0 otherwise).
    #[inline]
    fn div(&self, a: u8, b: u8) -> u8 {
        if a == 0 || b == 0 {
            0
        } else {
            self.exp[self.log[a as usize] as usize + 255 - self.log[b as usize] as usize]
        }
    }

    /// alpha^e for any exponent.
    #[inline]
    fn alpha(&self, e: usize) -> u8 {
        self.exp[e % 255]
    }

    /// Syndromes `S_j = block(alpha^(base + j))`; returns true if all are zero.
    fn syndromes(&self, block: &[u8], ec_len: usize, s: &mut [u8; N]) -> bool {
        let mut zero = true;
        for (j, out) in s.iter_mut().enumerate().take(ec_len) {
            let root = self.alpha(self.generator_base as usize + j);
            let mut acc = 0u8;
            for &c in block {
                acc = self.mul(acc, root) ^ c;
            }
            *out = acc;
            zero &= acc == 0;
        }
        zero
    }

    /// Computes `ec_len` check symbols for `data` into `out` (`out.len() == ec_len`).
    pub fn encode(&self, data: &[u8], ec_len: usize, out: &mut [u8]) -> Result<(), RsError> {
        if out.len() != ec_len || ec_len >= N || data.len() + ec_len > 255 {
            return Err(RsError::InvalidInput);
        }
        if ec_len == 0 {
            return Ok(());
        }
        // g(x) = prod (x - alpha^(base+j)), monic, g[i] = coefficient of x^i.
        let mut g = [0u8; N];
        g[0] = 1;
        for (deg, j) in (0..ec_len).enumerate() {
            let root = self.alpha(self.generator_base as usize + j);
            for i in (0..=deg + 1).rev() {
                let lower = if i > 0 { g[i - 1] } else { 0 };
                g[i] = self.mul(g[i], root) ^ lower;
            }
        }
        // Remainder of data * x^ec_len divided by g, via an LFSR.
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

    /// Corrects `block` in place. The last `ec_len` symbols are check symbols;
    /// `erasures` lists known-bad positions. Returns the number of symbols changed.
    /// Never allocates.
    pub fn decode(
        &self,
        block: &mut [u8],
        ec_len: usize,
        erasures: &[usize],
    ) -> Result<usize, RsError> {
        let n = block.len();
        if n > 255 || ec_len > n || erasures.len() > ec_len {
            return Err(RsError::InvalidInput);
        }
        let mut seen = [false; N];
        for &e in erasures {
            if e >= n || seen[e] {
                return Err(RsError::InvalidInput);
            }
            seen[e] = true;
        }
        if ec_len == 0 {
            return Ok(0);
        }
        let mut s = [0u8; N];
        if self.syndromes(block, ec_len, &mut s) {
            return Ok(0);
        }
        let f = erasures.len();

        // Erasure locator: prod (1 + X_k x), X_k = alpha^(n-1-pos).
        let mut lambda = [0u8; N];
        lambda[0] = 1;
        for (k, &pos) in erasures.iter().enumerate() {
            let x = self.alpha(n - 1 - pos);
            for i in (1..=k + 1).rev() {
                lambda[i] ^= self.mul(x, lambda[i - 1]);
            }
        }

        // Berlekamp-Massey seeded with the erasure locator.
        let mut b = lambda;
        let mut l = f;
        let mut last_d = 1u8;
        let mut m = 1usize;
        for r in f..ec_len {
            let mut d = 0u8;
            for i in 0..=l.min(r) {
                d ^= self.mul(lambda[i], s[r - i]);
            }
            if d == 0 {
                m += 1;
                continue;
            }
            let coef = self.div(d, last_d);
            let prev = lambda;
            for i in 0..N {
                let bi = b[i];
                if bi == 0 {
                    continue;
                }
                if i + m >= N {
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
        if deg == 0 || deg > n || 2 * (deg - f.min(deg)) + f > ec_len {
            return Err(RsError::Uncorrectable);
        }

        // Chien search: position i has locator X = alpha^(n-1-i); root of lambda at X^-1.
        let mut locs = [0usize; N];
        let mut found = 0usize;
        for i in 0..n {
            let xinv = self.alpha(255 - (n - 1 - i) % 255);
            let mut acc = 0u8;
            for &c in lambda[..=deg].iter().rev() {
                acc = self.mul(acc, xinv) ^ c;
            }
            if acc == 0 {
                if found == deg {
                    return Err(RsError::Uncorrectable);
                }
                locs[found] = i;
                found += 1;
            }
        }
        if found != deg {
            return Err(RsError::Uncorrectable);
        }

        // Omega = S * lambda mod x^ec_len.
        let mut omega = [0u8; N];
        for (k, o) in omega.iter_mut().enumerate().take(ec_len) {
            let mut acc = 0u8;
            for i in 0..=k.min(deg) {
                acc ^= self.mul(lambda[i], s[k - i]);
            }
            *o = acc;
        }

        // Forney: e = X^(1-base) * omega(X^-1) / lambda'(X^-1).
        let mut corrections = [0u8; N];
        let base = self.generator_base as usize % 255;
        for &i in &locs[..found] {
            let p = n - 1 - i;
            let xinv = self.alpha(255 - p % 255);
            let mut num = 0u8;
            for &c in omega[..ec_len].iter().rev() {
                num = self.mul(num, xinv) ^ c;
            }
            // Formal derivative: only odd-degree terms survive.
            let mut den = 0u8;
            let mut k = 1;
            while k <= deg {
                den ^= self.mul(lambda[k], self.alpha((k - 1) * (255 - p % 255)));
                k += 2;
            }
            if den == 0 {
                return Err(RsError::Uncorrectable);
            }
            let scale = self.alpha(p * (256 - base));
            corrections[i] = self.mul(scale, self.div(num, den));
        }

        let mut fixed = [0u8; N];
        fixed[..n].copy_from_slice(block);
        let mut changed = 0;
        for &i in &locs[..found] {
            if corrections[i] != 0 {
                fixed[i] ^= corrections[i];
                changed += 1;
            }
        }
        let mut check = [0u8; N];
        if !self.syndromes(&fixed[..n], ec_len, &mut check) {
            return Err(RsError::Uncorrectable);
        }
        block.copy_from_slice(&fixed[..n]);
        Ok(changed)
    }
}

#[cfg(test)]
mod tests;
