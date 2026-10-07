//! Tracks symbols across video frames and prints track events.
//!
//! Frames come from a directory of images (sorted by file name) or from raw
//! 8-bit grayscale frames on stdin:
//!
//!   cargo run --example tracker --features image -- frames_dir/
//!   ffmpeg -i in.mp4 -f rawvideo -pix_fmt gray - | \
//!     cargo run --example tracker --features image -- --size 1280x720

use std::io::Read;

use breader::{ImageView, ScanOptions, TrackEvent, Tracker, TrackerConfig};

fn report(tracker: &Tracker, update: &breader::FrameUpdate) {
    for event in &update.events {
        match *event {
            TrackEvent::New(id) => {
                if let Some(t) = tracker.tracks().iter().find(|t| t.id == id) {
                    println!(
                        "frame {}: new     #{id} {} {:?}",
                        update.frame_index,
                        t.symbol.symbology.name(),
                        t.symbol.text
                    );
                }
            }
            TrackEvent::Updated(_) => {}
            TrackEvent::Lost(id) => println!("frame {}: lost    #{id}", update.frame_index),
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut tracker = Tracker::new(ScanOptions::default(), TrackerConfig::default());

    if let Some(i) = args.iter().position(|a| a == "--size") {
        let size = args.get(i + 1).and_then(|s| s.split_once('x'));
        let parsed =
            size.and_then(|(w, h)| Some((w.parse::<usize>().ok()?, h.parse::<usize>().ok()?)));
        let Some((w, h)) = parsed else {
            eprintln!("--size expects WxH, e.g. 1280x720");
            std::process::exit(2);
        };
        let mut stdin = std::io::stdin().lock();
        let mut frame = vec![0u8; w * h];
        while stdin.read_exact(&mut frame).is_ok() {
            let view = ImageView::from_packed(&frame, w, h).expect("non-zero size");
            let update = tracker.process(view);
            report(&tracker, &update);
        }
        return;
    }

    let Some(dir) = args.first() else {
        eprintln!("usage: tracker <frames_dir> | tracker --size WxH < frames.gray");
        std::process::exit(2);
    };
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .expect("readable directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .is_some_and(|e| matches!(e.to_str(), Some("png" | "jpg" | "jpeg")))
        })
        .collect();
    files.sort();
    for path in files {
        let gray = image::open(&path).expect("decodable image").into_luma8();
        let (w, h) = (gray.width() as usize, gray.height() as usize);
        let view = ImageView::from_packed(gray.as_raw(), w, h).expect("non-zero size");
        let update = tracker.process(view);
        report(&tracker, &update);
    }
}
