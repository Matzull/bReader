use std::any::Any;

use crate::{Binarizer, BitMatrix, ImageBuffer, ImageView, Preprocessor, ScanOptions};

/// One attempt over the frame with a given image variant and binarization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Pass {
    /// Index into the pipeline's preprocessors; `None` = original image.
    pub preprocessor: Option<usize>,
    /// Index into the pipeline's binarizers.
    pub binarizer: usize,
    /// Whether the binary image is polarity-flipped (light-on-dark codes).
    pub inverted: bool,
}

impl Pass {
    /// A pass over `preprocessor` (`None` = the original image) binarized by
    /// the binarizer at index `binarizer`, polarity-flipped when `inverted`.
    pub const fn new(preprocessor: Option<usize>, binarizer: usize, inverted: bool) -> Self {
        Self {
            preprocessor,
            binarizer,
            inverted,
        }
    }
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

/// A per-frame value that readers of one frame share, for example the
/// pattern hits that one reader records while it scans and another reuses.
/// `reset` runs at the start of every frame and keeps allocations.
pub trait FrameCache: Default + Send + std::fmt::Debug + 'static {
    /// Clears the value for a new frame.
    fn reset(&mut self);
}

trait Slot: Send + std::fmt::Debug {
    fn reset(&mut self);
    fn as_any(&mut self) -> &mut dyn Any;
}

impl<T: FrameCache> Slot for T {
    fn reset(&mut self) {
        FrameCache::reset(self);
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

/// Buffers reused across frames. Owned by the caller (or the pipeline)
/// so steady-state scanning does not allocate.
#[derive(Debug, Default)]
pub struct Scratch {
    images: Vec<ImageEntry>,
    binaries: Vec<BinaryEntry>,
    inverted: BitMatrix,
    shared: Vec<Box<dyn Slot>>,
    /// Up-scaled copy of the frame for the retry pass.
    pub(crate) upscaled: ImageBuffer,
    /// Buffers of the nested scan over `upscaled`.
    pub(crate) retry: Option<Box<Scratch>>,
}

impl Scratch {
    /// Creates empty scratch space.
    pub fn new() -> Self {
        Self::default()
    }

    fn invalidate(&mut self) {
        self.images.iter_mut().for_each(|e| e.valid = false);
        self.binaries.iter_mut().for_each(|e| e.valid = false);
        self.shared.iter_mut().for_each(|s| s.reset());
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
    /// Starts a frame over `image`; previously cached products in `scratch` are discarded.
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

    /// Options of the current scan.
    pub fn options(&self) -> &'a ScanOptions {
        self.options
    }

    /// The pass currently being run.
    pub fn pass(&self) -> Pass {
        self.pass
    }

    /// Selects the pass whose image and binarization the accessors return.
    pub fn set_pass(&mut self, pass: Pass) {
        self.pass = pass;
    }

    /// The frame-wide value of type `T`, created on first use and reset at the
    /// start of every frame. Plumbing between the reader crates (see
    /// [`FrameCache`]); not covered by SemVer.
    #[doc(hidden)]
    pub fn shared<T: FrameCache>(&mut self) -> &mut T {
        let slots = &mut self.scratch.shared;
        let idx = match slots.iter_mut().position(|s| s.as_any().is::<T>()) {
            Some(i) => i,
            None => {
                slots.push(Box::new(T::default()));
                slots.len() - 1
            }
        };
        slots[idx]
            .as_any()
            .downcast_mut::<T>()
            .expect("slot type matches")
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
            ..
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
