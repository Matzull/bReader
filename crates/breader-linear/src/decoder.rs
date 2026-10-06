use breader_core::{EanAddonPolicy, ScanOptions, Symbology, SymbologySet};

use crate::runs::Runs;

/// A symbol read along a single scanline.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LineHit {
    pub symbology: Symbology,
    /// Payload bytes; ASCII, or ISO-8859-1 for Code 128 with FNC4.
    pub text: Vec<u8>,
    /// Supplement digits that follow the symbol, if any.
    pub addon: Option<Vec<u8>>,
    /// Profile positions of the first and last bar edge in reading order.
    pub start: f32,
    pub end: f32,
    /// Estimated module width in profile samples.
    pub module: f32,
    /// Mean match margin in [0, 1]; informational only.
    pub quality: f32,
    /// ISO/IEC 15424 symbology identifier.
    pub aim: [u8; 3],
    /// An optional check character was present and verified.
    pub checksum_verified: bool,
    /// Bars were lighter than spaces. Set by the reader, not by decoders.
    pub inverted: bool,
}

/// The subset of [`ScanOptions`] the decoders consume, copied once per scan.
/// `ScanOptions` itself cannot be borrowed across the pass-image accessor of
/// the frame context, and cloning it would allocate.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LineOptions {
    pub symbologies: SymbologySet,
    pub ean_addon: EanAddonPolicy,
    pub code39_min_length: usize,
    pub code39_checksum: bool,
    pub code39_full_ascii: bool,
    pub itf_min_length: usize,
    /// Bit `n` set: `n` digits accepted. Only meaningful when `itf_any` is false.
    pub(crate) itf_lengths: u128,
    pub(crate) itf_any: bool,
    /// Read dark-on-light symbols.
    pub normal: bool,
    /// Read light-on-dark symbols.
    pub invert: bool,
}

impl LineOptions {
    /// Longest ITF payload an explicit length list can name.
    pub const ITF_MAX_LISTED: usize = 127;

    pub fn itf_length_ok(&self, digits: usize) -> bool {
        self.itf_any || (digits <= Self::ITF_MAX_LISTED && self.itf_lengths >> digits & 1 == 1)
    }
}

impl From<&ScanOptions> for LineOptions {
    fn from(o: &ScanOptions) -> Self {
        let mut lengths = 0u128;
        for &n in &o.itf.allowed_lengths {
            if n <= Self::ITF_MAX_LISTED {
                lengths |= 1 << n;
            }
        }
        Self {
            symbologies: o.symbologies,
            ean_addon: o.ean_addon,
            code39_min_length: o.code39.min_length,
            code39_checksum: o.code39.require_checksum,
            code39_full_ascii: o.code39.full_ascii,
            itf_min_length: o.itf.min_length,
            itf_lengths: lengths,
            itf_any: o.itf.allowed_lengths.is_empty(),
            normal: true,
            invert: o.try_invert.unwrap_or(true),
        }
    }
}

/// Family-internal decoder working on one scanline's runs. Composed
/// statically, so every implementation is inlined into the reader.
pub(crate) trait LinearDecoder {
    /// Pushes every validated symbol found in `runs`, read in the forward
    /// direction of the runs. Callers also pass the reversed runs.
    fn decode(&self, runs: &Runs, opts: &LineOptions, out: &mut Vec<LineHit>);
}

macro_rules! decoder_tuples {
    ($($name:ident),+) => {
        impl<$($name: LinearDecoder),+> LinearDecoder for ($($name,)+) {
            #[allow(non_snake_case)]
            fn decode(&self, runs: &Runs, opts: &LineOptions, out: &mut Vec<LineHit>) {
                let ($($name,)+) = self;
                $($name.decode(runs, opts, out);)+
            }
        }
    };
}

decoder_tuples!(A);
decoder_tuples!(A, B);
decoder_tuples!(A, B, C);
decoder_tuples!(A, B, C, D);
decoder_tuples!(A, B, C, D, E);
decoder_tuples!(A, B, C, D, E, F);

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(&'static [u8]);

    impl LinearDecoder for Fixed {
        fn decode(&self, _runs: &Runs, _opts: &LineOptions, out: &mut Vec<LineHit>) {
            out.push(LineHit {
                symbology: Symbology::Code39,
                text: self.0.to_vec(),
                addon: None,
                start: 0.0,
                end: 1.0,
                module: 1.0,
                quality: 1.0,
                aim: *b"]A0",
                checksum_verified: false,
                inverted: false,
            });
        }
    }

    #[test]
    fn itf_length_list_becomes_a_bitset() {
        let mut o = ScanOptions::default();
        let opts = LineOptions::from(&o);
        assert!(opts.itf_length_ok(14) && !opts.itf_length_ok(12));
        o.itf.allowed_lengths = vec![6, 10, 500];
        let opts = LineOptions::from(&o);
        assert!(opts.itf_length_ok(6) && opts.itf_length_ok(10) && !opts.itf_length_ok(14));
        assert!(!opts.itf_length_ok(500) && !opts.itf_length_ok(usize::MAX));
        o.itf.allowed_lengths.clear();
        assert!(LineOptions::from(&o).itf_length_ok(usize::MAX));
    }

    #[test]
    fn tuples_run_every_decoder_in_order() {
        let runs = Runs::from_widths(true, &[1.0; 40], 10.0, 10.0);
        let opts = LineOptions::from(&ScanOptions::default());
        let mut out = Vec::new();
        (Fixed(b"a"), Fixed(b"b"), Fixed(b"c")).decode(&runs, &opts, &mut out);
        let texts: Vec<_> = out.iter().map(|h| h.text.as_slice()).collect();
        assert_eq!(texts, [b"a", b"b", b"c"]);
    }
}
