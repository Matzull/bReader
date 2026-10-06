use breader_core::{ScanOptions, Symbology};

use crate::runs::Runs;

/// A symbol read along a single scanline.
#[derive(Debug, Clone, PartialEq)]
pub struct LineHit {
    pub symbology: Symbology,
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
}

/// Family-internal decoder working on one scanline's runs. Composed
/// statically, so every implementation is inlined into the reader.
pub trait LinearDecoder {
    /// Pushes every validated symbol found in `runs`, read in the forward
    /// direction of the runs. Callers also pass the reversed runs.
    fn decode(&self, runs: &Runs, opts: &ScanOptions, out: &mut Vec<LineHit>);
}

macro_rules! decoder_tuples {
    ($($name:ident),+) => {
        impl<$($name: LinearDecoder),+> LinearDecoder for ($($name,)+) {
            #[allow(non_snake_case)]
            fn decode(&self, runs: &Runs, opts: &ScanOptions, out: &mut Vec<LineHit>) {
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

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(&'static [u8]);

    impl LinearDecoder for Fixed {
        fn decode(&self, _runs: &Runs, _opts: &ScanOptions, out: &mut Vec<LineHit>) {
            out.push(LineHit {
                symbology: Symbology::Code39,
                text: self.0.to_vec(),
                addon: None,
                start: 0.0,
                end: 1.0,
                module: 1.0,
                quality: 1.0,
            });
        }
    }

    #[test]
    fn tuples_run_every_decoder_in_order() {
        let runs = Runs::from_widths(true, &[1.0; 40], 10.0, 10.0);
        let opts = ScanOptions::default();
        let mut out = Vec::new();
        (Fixed(b"a"), Fixed(b"b"), Fixed(b"c")).decode(&runs, &opts, &mut out);
        let texts: Vec<_> = out.iter().map(|h| h.text.as_slice()).collect();
        assert_eq!(texts, [b"a", b"b", b"c"]);
    }
}
