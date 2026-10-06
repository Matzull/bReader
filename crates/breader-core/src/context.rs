use crate::{Binarizer, BitMatrix, ImageBuffer, ImageView, Preprocessor, ScanOptions};

/// One attempt over the frame with a given image variant and binarization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pass {
    /// Index into the pipeline's preprocessors; `None` = original image.
    pub preprocessor: Option<usize>,
    pub binarizer: usize,
    pub inverted: bool,
}

#[derive(Debug, Default)]
struct BinaryEntry {
    preprocessor: Option<usize>,
    binarizer: usize,
    valid: bool,
    matrix: BitMatrix,
}

#[derive(Debug, Default)]
struct ImageEntry {
    preprocessor: usize,
    valid: bool,
    buffer: ImageBuffer,
}

/// Buffers reused across frames. Owned by the caller (or the pipeline)
/// so steady-state scanning does not allocate.
#[derive(Debug, Default)]
pub struct Scratch {
    images: Vec<ImageEntry>,
    binaries: Vec<BinaryEntry>,
    inverted: BitMatrix,
}

impl Scratch {
    pub fn new() -> Self {
        Self::default()
    }

    fn invalidate(&mut self) {
        self.images.iter_mut().for_each(|e| e.valid = false);
        self.binaries.iter_mut().for_each(|e| e.valid = false);
    }
}

/// Per-frame state shared by all readers. Derived products (preprocessed
/// images, binarizations) are computed lazily and cached for the frame.
pub struct FrameContext<'a> {
    image: ImageView<'a>,
    options: &'a ScanOptions,
    preprocessors: &'a [Box<dyn Preprocessor>],
    binarizers: &'a [Box<dyn Binarizer>],
    scratch: &'a mut Scratch,
    pass: Pass,
}

impl std::fmt::Debug for FrameContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameContext")
            .field("width", &self.image.width())
            .field("height", &self.image.height())
            .field("pass", &self.pass)
            .finish_non_exhaustive()
    }
}

impl<'a> FrameContext<'a> {
    pub fn new(
        image: ImageView<'a>,
        options: &'a ScanOptions,
        preprocessors: &'a [Box<dyn Preprocessor>],
        binarizers: &'a [Box<dyn Binarizer>],
        scratch: &'a mut Scratch,
    ) -> Self {
        scratch.invalidate();
        Self {
            image,
            options,
            preprocessors,
            binarizers,
            scratch,
            pass: Pass {
                preprocessor: None,
                binarizer: 0,
                inverted: false,
            },
        }
    }

    pub fn options(&self) -> &ScanOptions {
        self.options
    }

    pub fn pass(&self) -> Pass {
        self.pass
    }

    pub fn set_pass(&mut self, pass: Pass) {
        self.pass = pass;
    }

    /// The original frame, independent of the current pass.
    pub fn original(&self) -> ImageView<'a> {
        self.image
    }

    /// Scale of the current pass image relative to the original frame.
    pub fn scale(&mut self) -> (f32, f32) {
        let (ow, oh) = (self.image.width() as f32, self.image.height() as f32);
        let img = self.image();
        (ow / img.width() as f32, oh / img.height() as f32)
    }

    /// Luma image for the current pass. Never inverted; readers working on
    /// grayscale handle polarity themselves via `pass().inverted`.
    pub fn image(&mut self) -> ImageView<'_> {
        pass_image(
            self.image,
            self.preprocessors,
            &mut self.scratch.images,
            self.pass.preprocessor,
        )
    }

    /// Binarized image for the current pass, inverted if the pass requires it.
    pub fn binary(&mut self) -> &BitMatrix {
        let Pass {
            preprocessor,
            binarizer,
            inverted,
        } = self.pass;
        let Scratch {
            images,
            binaries,
            inverted: inv,
        } = &mut *self.scratch;
        let idx = match binaries
            .iter()
            .position(|e| e.preprocessor == preprocessor && e.binarizer == binarizer)
        {
            Some(i) => i,
            None => {
                binaries.push(BinaryEntry {
                    preprocessor,
                    binarizer,
                    ..BinaryEntry::default()
                });
                binaries.len() - 1
            }
        };
        let entry = &mut binaries[idx];
        if !entry.valid {
            let src = pass_image(self.image, self.preprocessors, images, preprocessor);
            entry.matrix.resize(src.width(), src.height());
            self.binarizers[binarizer].binarize(src, &mut entry.matrix);
            entry.valid = true;
        }
        if inverted {
            inv.copy_from(&entry.matrix);
            inv.invert();
            inv
        } else {
            &entry.matrix
        }
    }
}

fn pass_image<'s>(
    original: ImageView<'s>,
    preprocessors: &[Box<dyn Preprocessor>],
    images: &'s mut Vec<ImageEntry>,
    preprocessor: Option<usize>,
) -> ImageView<'s> {
    let Some(p) = preprocessor else {
        return original;
    };
    let idx = match images.iter().position(|e| e.preprocessor == p) {
        Some(i) => i,
        None => {
            images.push(ImageEntry {
                preprocessor: p,
                ..ImageEntry::default()
            });
            images.len() - 1
        }
    };
    let entry = &mut images[idx];
    if !entry.valid {
        preprocessors[p].apply(original, &mut entry.buffer);
        entry.valid = true;
    }
    entry.buffer.view()
}
