//! Frame-to-frame tracking of symbols in a video stream.

use breader_core::{Effort, ImageView, Point, ScanOptions, Symbol};

use crate::Reader;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackerConfig {
    /// A full-effort global scan runs every this many frames.
    pub full_scan_interval: u32,
    /// Same, while nothing is tracked; 1 scans at full effort every empty frame.
    pub idle_scan_interval: u32,
    /// A track is lost when it has not been seen for this many frames.
    pub lost_after: u32,
    /// Each side of a track's bounding box grows by this fraction of its size.
    pub roi_margin: f32,
    pub roi_effort: Effort,
    pub global_effort: Effort,
    pub full_effort: Effort,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        Self {
            full_scan_interval: 4,
            idle_scan_interval: 1,
            lost_after: 5,
            roi_margin: 0.5,
            roi_effort: Effort::Balanced,
            global_effort: Effort::Fast,
            full_effort: Effort::Balanced,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub id: u64,
    /// The most recent detection, in frame coordinates.
    pub symbol: Symbol,
    pub first_frame: u64,
    pub last_frame: u64,
    pub hits: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackEvent {
    New(u64),
    Updated(u64),
    Lost(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameUpdate {
    pub events: Vec<TrackEvent>,
    pub frame_index: u64,
}

/// Stateful video reader; feed it frames in order.
#[derive(Debug)]
pub struct Tracker {
    config: TrackerConfig,
    readers: Vec<(Effort, Reader)>,
    tracks: Vec<Track>,
    frame_index: u64,
    next_id: u64,
    since_full: u32,
    #[cfg(test)]
    stats: TestStats,
}

#[cfg(test)]
#[derive(Debug, Default)]
struct TestStats {
    roi_scans: u32,
    global_scans: u32,
    full_scans: u32,
}

impl Tracker {
    pub fn new(options: ScanOptions, config: TrackerConfig) -> Self {
        let mut readers: Vec<(Effort, Reader)> = Vec::new();
        for effort in [config.roi_effort, config.global_effort, config.full_effort] {
            if readers.iter().all(|(e, _)| *e != effort) {
                let mut o = options.clone();
                o.effort = effort;
                readers.push((effort, Reader::with_options(o)));
            }
        }
        Self {
            config,
            readers,
            tracks: Vec::new(),
            frame_index: 0,
            next_id: 0,
            since_full: 0,
            #[cfg(test)]
            stats: TestStats::default(),
        }
    }

    pub fn config(&self) -> &TrackerConfig {
        &self.config
    }

    /// Tracks currently alive, in creation order.
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    fn scan(&mut self, effort: Effort, view: ImageView<'_>) -> Vec<Symbol> {
        match self.readers.iter_mut().find(|(e, _)| *e == effort) {
            Some((_, reader)) => reader.scan(view),
            None => Vec::new(),
        }
    }

    pub fn process(&mut self, frame: ImageView<'_>) -> FrameUpdate {
        let frame_index = self.frame_index;
        self.frame_index += 1;
        let mut detections: Vec<Symbol> = Vec::new();

        let rois: Vec<(usize, usize, usize, usize)> = self
            .tracks
            .iter()
            .filter_map(|t| roi_rect(&t.symbol, self.config.roi_margin, &frame))
            .collect();
        for (x, y, w, h) in rois {
            let Some(view) = frame.crop(x, y, w, h) else {
                continue;
            };
            #[cfg(test)]
            {
                self.stats.roi_scans += 1;
            }
            let found = self.scan(self.config.roi_effort, view);
            for mut s in found {
                offset(&mut s, x as f32, y as f32);
                push_unique(&mut detections, s);
            }
        }

        let interval = if self.tracks.is_empty() {
            self.config
                .idle_scan_interval
                .min(self.config.full_scan_interval)
        } else {
            self.config.full_scan_interval
        };
        let full = self.since_full + 1 >= interval;
        let effort = if full {
            self.since_full = 0;
            #[cfg(test)]
            {
                self.stats.full_scans += 1;
            }
            self.config.full_effort
        } else {
            self.since_full += 1;
            #[cfg(test)]
            {
                self.stats.global_scans += 1;
            }
            self.config.global_effort
        };
        for s in self.scan(effort, frame) {
            push_unique(&mut detections, s);
        }

        let mut events = Vec::new();
        let assigned = self.assign(&detections);
        for (det, track) in detections.into_iter().zip(assigned) {
            match track {
                Some(i) => {
                    let t = &mut self.tracks[i];
                    t.symbol = det;
                    t.last_frame = frame_index;
                    t.hits += 1;
                    events.push(TrackEvent::Updated(t.id));
                }
                None => {
                    let id = self.next_id;
                    self.next_id += 1;
                    self.tracks.push(Track {
                        id,
                        symbol: det,
                        first_frame: frame_index,
                        last_frame: frame_index,
                        hits: 1,
                    });
                    events.push(TrackEvent::New(id));
                }
            }
        }

        let lost_after = u64::from(self.config.lost_after);
        self.tracks.retain(|t| {
            let lost = frame_index - t.last_frame >= lost_after;
            if lost {
                events.push(TrackEvent::Lost(t.id));
            }
            !lost
        });
        FrameUpdate {
            events,
            frame_index,
        }
    }

    /// Matches detections to existing tracks with the same symbology and data,
    /// closest pairs first, one track per detection.
    fn assign(&self, detections: &[Symbol]) -> Vec<Option<usize>> {
        let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
        for (d, det) in detections.iter().enumerate() {
            for (t, track) in self.tracks.iter().enumerate() {
                let s = &track.symbol;
                if s.symbology != det.symbology || s.data != det.data {
                    continue;
                }
                let dist = det.quad.center().distance(s.quad.center());
                if dist <= diagonal(det).max(diagonal(s)) {
                    pairs.push((dist, d, t));
                }
            }
        }
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        let mut out = vec![None; detections.len()];
        let mut taken = vec![false; self.tracks.len()];
        for (_, d, t) in pairs {
            if out[d].is_none() && !taken[t] {
                out[d] = Some(t);
                taken[t] = true;
            }
        }
        out
    }
}

fn diagonal(s: &Symbol) -> f32 {
    let c = &s.quad.corners;
    c[0].distance(c[2]).max(c[1].distance(c[3]))
}

/// Same symbol seen twice in one frame (overlapping regions, ROI plus global scan).
fn push_unique(list: &mut Vec<Symbol>, s: Symbol) {
    let dup = list.iter().any(|o| {
        o.symbology == s.symbology
            && o.data == s.data
            && o.quad.center().distance(s.quad.center()) < 0.5 * diagonal(o).min(diagonal(&s))
    });
    if !dup {
        list.push(s);
    }
}

fn offset(s: &mut Symbol, dx: f32, dy: f32) {
    for c in &mut s.quad.corners {
        *c = Point::new(c.x + dx, c.y + dy);
    }
}

fn roi_rect(
    s: &Symbol,
    margin: f32,
    frame: &ImageView<'_>,
) -> Option<(usize, usize, usize, usize)> {
    let cs = &s.quad.corners;
    let min_x = cs.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
    let max_x = cs.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
    let min_y = cs.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
    let max_y = cs.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
    if ![min_x, max_x, min_y, max_y].iter().all(|v| v.is_finite()) {
        return None;
    }
    let m = margin.max(0.0);
    let (mx, my) = ((max_x - min_x) * m, (max_y - min_y) * m);
    let x0 = (min_x - mx).floor().max(0.0);
    let y0 = (min_y - my).floor().max(0.0);
    let x1 = (max_x + mx).ceil().min(frame.width() as f32);
    let y1 = (max_y + my).ceil().min(frame.height() as f32);
    (x1 > x0 && y1 > y0).then_some({
        (
            x0 as usize,
            y0 as usize,
            (x1 - x0) as usize,
            (y1 - y0) as usize,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use breader_core::{ImageBuffer, Symbology};
    use qrcode::{Color, EcLevel, QrCode};

    const W: usize = 480;
    const H: usize = 320;
    const PX: usize = 4;

    fn qr_frame(placements: &[(&str, usize, usize)]) -> ImageBuffer {
        let mut img = ImageBuffer::new(W, H);
        img.as_mut_slice().fill(225);
        for &(text, ox, oy) in placements {
            let code = QrCode::with_error_correction_level(text, EcLevel::M).unwrap();
            let dim = code.width();
            let colors = code.to_colors();
            let quiet = 4 * PX;
            let (x0, x1) = (ox.saturating_sub(quiet), (ox + dim * PX + quiet).min(W));
            for y in oy.saturating_sub(quiet)..(oy + dim * PX + quiet).min(H) {
                img.row_mut(y)[x0..x1].fill(225);
            }
            for my in 0..dim {
                for mx in 0..dim {
                    if colors[my * dim + mx] != Color::Dark {
                        continue;
                    }
                    let xs = ox + mx * PX..(ox + (mx + 1) * PX).min(W);
                    for y in (oy + my * PX..oy + (my + 1) * PX).filter(|&y| y < H) {
                        img.row_mut(y)[xs.clone()].fill(20);
                    }
                }
            }
        }
        img
    }

    fn tracker() -> Tracker {
        Tracker::new(ScanOptions::default(), TrackerConfig::default())
    }

    #[test]
    fn moving_code_keeps_its_id() {
        let mut t = tracker();
        let mut ids = Vec::new();
        for i in 0..12 {
            let img = qr_frame(&[("hello tracker", 40 + i * 8, 60 + i * 3)]);
            let up = t.process(img.view());
            assert_eq!(up.frame_index, i as u64);
            for e in &up.events {
                match e {
                    TrackEvent::New(id) | TrackEvent::Updated(id) => ids.push(*id),
                    TrackEvent::Lost(_) => panic!("lost while visible"),
                }
            }
        }
        assert_eq!(ids.len(), 12);
        assert!(ids.iter().all(|&id| id == ids[0]));
        assert_eq!(t.tracks().len(), 1);
        assert_eq!(t.tracks()[0].hits, 12);
        assert_eq!(t.tracks()[0].symbol.symbology, Symbology::Qr);
        assert!(t.stats.roi_scans >= 10, "{}", t.stats.roi_scans);
        assert!(t.stats.global_scans > 0 && t.stats.full_scans > 0);
    }

    #[test]
    fn vanished_code_is_lost_after_n_frames() {
        let mut t = tracker();
        let img = qr_frame(&[("gone soon", 100, 100)]);
        let up = t.process(img.view());
        assert_eq!(up.events, vec![TrackEvent::New(0)]);
        let blank = qr_frame(&[]);
        for i in 1..5 {
            let up = t.process(blank.view());
            assert!(up.events.is_empty(), "frame {i}: {:?}", up.events);
            assert_eq!(t.tracks().len(), 1);
        }
        let up = t.process(blank.view());
        assert_eq!(up.events, vec![TrackEvent::Lost(0)]);
        assert!(t.tracks().is_empty());
    }

    #[test]
    fn identical_codes_get_separate_tracks() {
        let mut t = tracker();
        let img = qr_frame(&[("twin", 30, 40), ("twin", 280, 150)]);
        let up = t.process(img.view());
        assert_eq!(up.events.len(), 2);
        assert_eq!(t.tracks().len(), 2);
        for _ in 0..4 {
            let up = t.process(img.view());
            assert_eq!(up.events.len(), 2);
            assert!(
                up.events
                    .iter()
                    .all(|e| matches!(e, TrackEvent::Updated(_)))
            );
        }
        assert_eq!(t.tracks().len(), 2);
        assert!(t.tracks().iter().all(|t| t.hits == 5));
    }

    #[test]
    fn roi_only_frame_keeps_track_without_global_help() {
        let config = TrackerConfig {
            full_scan_interval: 1000,
            ..TrackerConfig::default()
        };
        let mut t = Tracker::new(ScanOptions::default(), config);
        let img = qr_frame(&[("roi path", 150, 90)]);
        t.process(img.view());
        let before = t.stats.roi_scans;
        let up = t.process(img.view());
        assert_eq!(up.events, vec![TrackEvent::Updated(0)]);
        assert_eq!(t.stats.roi_scans, before + 1);
    }

    #[test]
    fn roi_detections_are_in_frame_coordinates() {
        let mut t = tracker();
        let img = qr_frame(&[("coords", 200, 120)]);
        t.process(img.view());
        t.process(img.view());
        let c = t.tracks()[0].symbol.quad.center();
        assert!(
            (c.x - 220.0).abs() < 40.0 && (c.y - 140.0).abs() < 40.0,
            "{c:?}"
        );
    }
}
