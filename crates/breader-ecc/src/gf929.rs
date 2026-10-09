//! Reed-Solomon error correction over the prime field GF(929), as used by
//! PDF417 (ISO/IEC 15438).
//!
//! Block layout: `block[0]` is the highest-degree coefficient; the last
//! `ec_len` codewords are check codewords. The generator polynomial is
//! `g(x) = (x - 3^1)(x - 3^2)...(x - 3^ec_len)` and the check codewords are the
//! negated remainder of `data * x^ec_len` modulo `g`, so every valid block
//! evaluates to zero at `3^1 ..= 3^ec_len`.

use crate::RsError;

const P: u32 = 929;
const ORDER: usize = 928;
/// Longest block: the multiplicative order of 3, so that positions have
/// distinct locators.
pub const MAX_BLOCK_LEN: usize = ORDER;
/// Largest number of check codewords (PDF417 error correction level 8).
pub const MAX_EC_LEN: usize = 512;

const L: usize = MAX_EC_LEN + 1;

/// Result of a successful [`Gf929::decode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Correction {
    /// Codewords outside the erasure list that were changed.
    pub errors: usize,
    /// Codewords in the erasure list that were changed.
    pub erasures: usize,
}

/// The prime field of 929 elements with generator 3.
#[derive(Debug)]
pub struct Gf929 {
    exp: [u16; ORDER],
    log: [u16; 929],
}

/// Field used by PDF417.
pub static PDF417_FIELD: Gf929 = Gf929::new();

#[inline]
fn mul(a: u16, b: u16) -> u16 {
    (u32::from(a) * u32::from(b) % P) as u16
}

#[inline]
fn add(a: u16, b: u16) -> u16 {
    ((u32::from(a) + u32::from(b)) % P) as u16
}

#[inline]
fn sub(a: u16, b: u16) -> u16 {
    ((u32::from(a) + P - u32::from(b)) % P) as u16
}

impl Gf929 {
    const fn new() -> Self {
        let mut exp = [0u16; ORDER];
        let mut log = [0u16; 929];
        let mut x = 1u32;
        let mut i = 0;
        while i < ORDER {
            exp[i] = x as u16;
            log[x as usize] = i as u16;
            x = x * 3 % P;
            i += 1;
        }
        Self { exp, log }
    }

    /// `3^e` for any exponent.
    #[inline]
    fn alpha(&self, e: usize) -> u16 {
        self.exp[e % ORDER]
    }

    /// Inverse of a nonzero element.
    #[inline]
    fn inv(&self, a: u16) -> u16 {
        self.exp[(ORDER - usize::from(self.log[usize::from(a)])) % ORDER]
    }

    /// Syndromes `S_j = block(3^(j+1))`; returns true if all are zero.
    fn syndromes(&self, block: &[u16], ec_len: usize, s: &mut [u16; L]) -> bool {
        let mut zero = true;
        for (j, out) in s.iter_mut().enumerate().take(ec_len) {
            let root = self.alpha(j + 1);
            let mut acc = 0u16;
            for &c in block {
                acc = add(mul(acc, root), c);
            }
            *out = acc;
            zero &= acc == 0;
        }
        zero
    }

    /// Computes `ec_len` check codewords for `data` into `out`
    /// (`out.len() == ec_len`). Data codewords must be below 929.
    pub fn encode(&self, data: &[u16], ec_len: usize, out: &mut [u16]) -> Result<(), RsError> {
        if out.len() != ec_len
            || ec_len > MAX_EC_LEN
            || data.len() + ec_len > MAX_BLOCK_LEN
            || data.iter().any(|&d| u32::from(d) >= P)
        {
            return Err(RsError::InvalidInput);
        }
        if ec_len == 0 {
            return Ok(());
        }
        // g[i] = coefficient of x^i, monic of degree ec_len.
        let mut g = [0u16; L + 1];
        g[0] = 1;
        for deg in 0..ec_len {
            let root = self.alpha(deg + 1);
            for i in (0..=deg + 1).rev() {
                let lower = if i > 0 { g[i - 1] } else { 0 };
                g[i] = sub(lower, mul(g[i], root));
            }
        }
        out.fill(0);
        for &d in data {
            let fb = add(d, out[0]);
            out.copy_within(1.., 0);
            out[ec_len - 1] = 0;
            if fb != 0 {
                for (k, o) in out.iter_mut().enumerate() {
                    *o = sub(*o, mul(fb, g[ec_len - 1 - k]));
                }
            }
        }
        for o in out.iter_mut() {
            *o = sub(0, *o);
        }
        Ok(())
    }

    /// Corrects `block` in place; the last `ec_len` codewords are check
    /// codewords and `erasures` lists known-bad positions. `block` is left
    /// untouched on failure.
    ///
    /// The result reports errors and erasures separately so that callers can
    /// apply a stricter bound than the code's own (`2 * errors + erasures <=
    /// ec_len`). Never allocates.
    pub fn decode(
        &self,
        block: &mut [u16],
        ec_len: usize,
        erasures: &[usize],
    ) -> Result<Correction, RsError> {
        let n = block.len();
        if n > MAX_BLOCK_LEN
            || ec_len > n
            || ec_len > MAX_EC_LEN
            || erasures.len() > ec_len
            || block.iter().any(|&c| u32::from(c) >= P)
        {
            return Err(RsError::InvalidInput);
        }
        let mut is_erasure = [false; ORDER];
        for &e in erasures {
            if e >= n || is_erasure[e] {
                return Err(RsError::InvalidInput);
            }
            is_erasure[e] = true;
        }
        if ec_len == 0 {
            return Ok(Correction::default());
        }
        let mut s = [0u16; L];
        if self.syndromes(block, ec_len, &mut s) {
            return Ok(Correction::default());
        }
        let f = erasures.len();

        // Erasure locator: prod (1 - X_k x), X_k = 3^(n-1-pos).
        let mut lambda = [0u16; L];
        lambda[0] = 1;
        for (k, &pos) in erasures.iter().enumerate() {
            let x = self.alpha(n - 1 - pos);
            for i in (1..=k + 1).rev() {
                lambda[i] = sub(lambda[i], mul(x, lambda[i - 1]));
            }
        }

        // Berlekamp-Massey seeded with the erasure locator.
        let mut b = lambda;
        let mut l = f;
        let mut last_d = 1u16;
        let mut m = 1usize;
        for r in f..ec_len {
            let mut d = 0u16;
            for i in 0..=l.min(r) {
                d = add(d, mul(lambda[i], s[r - i]));
            }
            if d == 0 {
                m += 1;
                continue;
            }
            let coef = mul(d, self.inv(last_d));
            let prev = lambda;
            for (i, &bi) in b.iter().enumerate() {
                if bi == 0 {
                    continue;
                }
                if i + m >= L {
                    return Err(RsError::Uncorrectable);
                }
                lambda[i + m] = sub(lambda[i + m], mul(coef, bi));
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

        // Chien search: position i has locator X = 3^(n-1-i); root of lambda at X^-1.
        let mut locs = [0usize; L];
        let mut found = 0usize;
        for i in 0..n {
            let xinv = self.alpha(ORDER - (n - 1 - i) % ORDER);
            let mut acc = 0u16;
            for &c in lambda[..=deg].iter().rev() {
                acc = add(mul(acc, xinv), c);
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
        let mut omega = [0u16; L];
        for (k, o) in omega.iter_mut().enumerate().take(ec_len) {
            let mut acc = 0u16;
            for i in 0..=k.min(deg) {
                acc = add(acc, mul(lambda[i], s[k - i]));
            }
            *o = acc;
        }

        // Forney with first root 3^1: the received value is corrected by
        // adding omega(X^-1) / lambda'(X^-1).
        let mut fixed = [0u16; ORDER];
        fixed[..n].copy_from_slice(block);
        let mut correction = Correction::default();
        for &i in &locs[..found] {
            let xinv = self.alpha(ORDER - (n - 1 - i) % ORDER);
            let mut num = 0u16;
            for &c in omega[..ec_len].iter().rev() {
                num = add(mul(num, xinv), c);
            }
            let mut den = 0u16;
            let mut pow = 1u16;
            for (k, &c) in lambda[..=deg].iter().enumerate().skip(1) {
                den = add(den, mul(mul(c, k as u16), pow));
                pow = mul(pow, xinv);
            }
            if den == 0 {
                return Err(RsError::Uncorrectable);
            }
            let delta = mul(num, self.inv(den));
            if delta == 0 {
                // A located position without a change means the locator is
                // spurious, unless it is an erasure that was received intact.
                if is_erasure[i] {
                    continue;
                }
                return Err(RsError::Uncorrectable);
            }
            fixed[i] = add(fixed[i], delta);
            if is_erasure[i] {
                correction.erasures += 1;
            } else {
                correction.errors += 1;
            }
        }
        let mut check = [0u16; L];
        if !self.syndromes(&fixed[..n], ec_len, &mut check) {
            return Err(RsError::Uncorrectable);
        }
        block.copy_from_slice(&fixed[..n]);
        Ok(correction)
    }
}

#[cfg(test)]
mod tests;
