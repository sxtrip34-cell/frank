// Where the island window goes, worked out in logical pixels relative to the
// top-left of the display's work area (the screen minus the taskbar), so every
// case can be tested without a display.
//
// The island is drawn inside a fixed 720×320 window. What has to stay on the
// screen is the island, not that window: the window is clamped into the work
// area, and the island's anchor — its top centre, or the point on the edge it
// is docked to — is handed to the front end as a position inside the window.
// The front end then keeps the island inside the window at whatever size it
// grows to, so an island near an edge opens away from that edge.

use crate::island::{PANEL_H, PANEL_W, SIDE_H, STRIP_H, STRIP_W};
use crate::settings::{Dock, IslandPos};

/// A dragged island this close to an edge docks to it on release.
pub const SNAP_IN: f64 = 20.0;
/// Once docking is offered, the island has to move this far off to lose it,
/// so the offer doesn't flicker on and off at the threshold.
pub const SNAP_OUT: f64 = 36.0;
/// Docked to the top this close to the middle, the island takes its own place
/// in the middle again.
pub const CENTRE_MAGNET: f64 = 40.0;
/// The compact island's height: a free island keeps at least this on screen.
const COMPACT_H: f64 = 32.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn right(&self) -> f64 {
        self.x + self.w
    }
}

/// Width and height of the work area, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Work {
    pub w: f64,
    pub h: f64,
}

/// The window's place in the work area, and the island's anchor inside a full
/// window (also while the window is the wake strip, for when it grows back).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub window: Rect,
    pub anchor: (f64, f64),
    pub dock: Dock,
}

/// `None` is the island's own place: docked at the top, in the middle.
fn resolve(pos: Option<IslandPos>, work: Work) -> IslandPos {
    pos.unwrap_or(IslandPos { x: work.w / 2.0, y: 0.0, dock: Dock::Top })
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi.max(lo))
}

pub fn place(pos: Option<IslandPos>, work: Work, collapsed: bool) -> Placement {
    let p = resolve(pos, work);
    // The anchor, in the work area: the island's top centre, or for a side
    // dock the point on that edge level with the island's top.
    let ax = match p.dock {
        Dock::Left => 0.0,
        Dock::Right => work.w,
        Dock::Top | Dock::Free => clamp(p.x, 0.0, work.w),
    };
    let ay = match p.dock {
        Dock::Top => 0.0,
        Dock::Left | Dock::Right => clamp(p.y, 0.0, work.h - SIDE_H),
        Dock::Free => clamp(p.y, 0.0, work.h - COMPACT_H),
    };

    let wx = clamp(ax - PANEL_W / 2.0, 0.0, work.w - PANEL_W);
    let wy = match p.dock {
        Dock::Top => 0.0,
        _ => clamp(ay, 0.0, work.h - PANEL_H),
    };
    let anchor = (ax - wx, ay - wy);

    let window = if collapsed {
        match p.dock {
            Dock::Left => Rect { x: 0.0, y: ay, w: STRIP_H, h: SIDE_H },
            Dock::Right => Rect { x: work.w - STRIP_H, y: ay, w: STRIP_H, h: SIDE_H },
            // A free island never hides by itself; paused, its strip waits at
            // the top above where it was.
            Dock::Top | Dock::Free => Rect {
                x: clamp(ax - STRIP_W / 2.0, 0.0, work.w - STRIP_W),
                y: 0.0,
                w: STRIP_W,
                h: STRIP_H,
            },
        }
    } else {
        Rect { x: wx, y: wy, w: PANEL_W, h: PANEL_H }
    };
    Placement { window, anchor, dock: p.dock }
}

/// Keeps a dragged island (its top-left, at its current size) inside the work
/// area: it can touch every edge but never leave the screen.
pub fn clamp_island(x: f64, y: f64, w: f64, h: f64, work: Work) -> (f64, f64) {
    (clamp(x, 0.0, work.w - w), clamp(y, 0.0, work.h - h))
}

/// The edge a dragged island would dock to if let go now. `offered` is the edge
/// offered on the previous step: it is kept until the island moves SNAP_OUT
/// away. The bottom edge is the taskbar's: nothing docks there.
pub fn dock_at(island: Rect, work: Work, offered: Option<Dock>) -> Option<Dock> {
    let edges = [
        (Dock::Top, island.y),
        (Dock::Left, island.x),
        (Dock::Right, work.w - island.right()),
    ];
    let mut best: Option<(Dock, f64)> = None;
    for (dock, distance) in edges {
        let limit = if offered == Some(dock) { SNAP_OUT } else { SNAP_IN };
        if distance > limit {
            continue;
        }
        // Closest edge wins; on a tie (a corner) the one already offered, then
        // the first in the list.
        let better = match best {
            None => true,
            Some((held, d)) => distance < d || (distance == d && offered == Some(dock) && offered != Some(held)),
        };
        if better {
            best = Some((dock, distance));
        }
    }
    best.map(|(dock, _)| dock)
}

/// What to remember once a dragged island is let go at `island` (work-area
/// logical), docked to `dock` or free. None: its own place, top centre.
pub fn drop_position(island: Rect, dock: Option<Dock>, work: Work) -> Option<IslandPos> {
    let cx = island.x + island.w / 2.0;
    match dock {
        Some(Dock::Top) if (cx - work.w / 2.0).abs() <= CENTRE_MAGNET => None,
        Some(Dock::Top) => Some(IslandPos { x: cx, y: 0.0, dock: Dock::Top }),
        Some(Dock::Left) => Some(IslandPos { x: 0.0, y: island.y, dock: Dock::Left }),
        Some(Dock::Right) => Some(IslandPos { x: work.w, y: island.y, dock: Dock::Right }),
        Some(Dock::Free) | None => Some(IslandPos { x: cx, y: island.y, dock: Dock::Free }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1920×1080 with a 48 px taskbar at the bottom.
    const WORK: Work = Work { w: 1920.0, h: 1032.0 };

    fn pos(x: f64, y: f64, dock: Dock) -> Option<IslandPos> {
        Some(IslandPos { x, y, dock })
    }

    /// Where the front end draws an island of `w`×`h` for this placement —
    /// the same rule as islandRect() in src/island/island.ts.
    fn island_in_window(p: &Placement, w: f64, h: f64) -> Rect {
        let x = match p.dock {
            Dock::Left => 0.0,
            Dock::Right => PANEL_W - w,
            _ => clamp(p.anchor.0 - w / 2.0, 0.0, PANEL_W - w),
        };
        let y = match p.dock {
            Dock::Top => 0.0,
            _ => clamp(p.anchor.1, 0.0, PANEL_H - h),
        };
        Rect { x: p.window.x + x, y: p.window.y + y, w, h }
    }

    fn on_screen(r: Rect) -> bool {
        r.x >= 0.0 && r.y >= 0.0 && r.right() <= WORK.w && r.y + r.h <= WORK.h
    }

    #[test]
    fn default_is_the_top_centre() {
        let p = place(None, WORK, false);
        assert_eq!(p.dock, Dock::Top);
        assert_eq!(p.window, Rect { x: 600.0, y: 0.0, w: PANEL_W, h: PANEL_H });
        assert_eq!(p.anchor, (360.0, 0.0));
    }

    #[test]
    fn docked_at_the_top_left_the_island_reaches_the_corner() {
        let p = place(pos(60.0, 0.0, Dock::Top), WORK, false);
        assert_eq!(p.window.x, 0.0);
        let compact = island_in_window(&p, 288.0, 32.0);
        assert_eq!((compact.x, compact.y), (0.0, 0.0));
        // Opened, it grows to the right rather than off the screen.
        let open = island_in_window(&p, 640.0, 300.0);
        assert!(on_screen(open), "{open:?}");
    }

    #[test]
    fn a_free_island_can_sit_against_every_edge_and_still_open() {
        for (x, y) in [(10.0, 500.0), (1910.0, 500.0), (960.0, 1031.0), (5.0, 1031.0), (1915.0, 2.0)] {
            let p = place(pos(x, y, Dock::Free), WORK, false);
            assert!(on_screen(p.window), "window off screen for {x},{y}: {:?}", p.window);
            for (w, h) in [(288.0, 32.0), (640.0, 160.0), (640.0, 300.0)] {
                let r = island_in_window(&p, w, h);
                assert!(on_screen(r), "{w}x{h} island off screen for {x},{y}: {r:?}");
            }
        }
    }

    #[test]
    fn side_docks_hug_their_edge() {
        let left = place(pos(0.0, 400.0, Dock::Left), WORK, false);
        let tab = island_in_window(&left, 44.0, SIDE_H);
        assert_eq!((tab.x, tab.y), (0.0, 400.0));
        let right = place(pos(WORK.w, 400.0, Dock::Right), WORK, false);
        let tab = island_in_window(&right, 44.0, SIDE_H);
        assert_eq!((tab.right(), tab.y), (WORK.w, 400.0));
        // Opened low on the screen, the drawer moves up to fit.
        let low = place(pos(WORK.w, 1000.0, Dock::Right), WORK, false);
        let drawer = island_in_window(&low, 640.0, 300.0);
        assert!(on_screen(drawer), "{drawer:?}");
        assert_eq!(drawer.right(), WORK.w);
    }

    #[test]
    fn hidden_islands_leave_a_strip_on_their_edge() {
        let top = place(pos(60.0, 0.0, Dock::Top), WORK, true);
        assert_eq!(top.window, Rect { x: 0.0, y: 0.0, w: STRIP_W, h: STRIP_H });
        let left = place(pos(0.0, 400.0, Dock::Left), WORK, true);
        assert_eq!(left.window, Rect { x: 0.0, y: 400.0, w: STRIP_H, h: SIDE_H });
        let right = place(pos(WORK.w, 400.0, Dock::Right), WORK, true);
        assert_eq!(right.window, Rect { x: WORK.w - STRIP_H, y: 400.0, w: STRIP_H, h: SIDE_H });
        // The anchor is still the full window's, for when it opens again.
        assert_eq!(right.anchor, place(pos(WORK.w, 400.0, Dock::Right), WORK, false).anchor);
    }

    #[test]
    fn dragging_never_takes_the_island_off_screen() {
        assert_eq!(clamp_island(-300.0, -50.0, 288.0, 32.0, WORK), (0.0, 0.0));
        assert_eq!(clamp_island(5000.0, 5000.0, 288.0, 32.0, WORK), (1920.0 - 288.0, 1032.0 - 32.0));
    }

    #[test]
    fn docking_is_offered_near_an_edge_and_held_a_little_longer() {
        let at = |x: f64, y: f64| Rect { x, y, w: 288.0, h: 32.0 };
        assert_eq!(dock_at(at(800.0, 400.0), WORK, None), None);
        assert_eq!(dock_at(at(800.0, 10.0), WORK, None), Some(Dock::Top));
        assert_eq!(dock_at(at(12.0, 400.0), WORK, None), Some(Dock::Left));
        assert_eq!(dock_at(at(1920.0 - 288.0 - 5.0, 400.0), WORK, None), Some(Dock::Right));
        // 30 px off the left edge: not offered from scratch, kept once offered.
        assert_eq!(dock_at(at(30.0, 400.0), WORK, None), None);
        assert_eq!(dock_at(at(30.0, 400.0), WORK, Some(Dock::Left)), Some(Dock::Left));
        assert_eq!(dock_at(at(40.0, 400.0), WORK, Some(Dock::Left)), None);
        // In a corner the closer edge wins.
        assert_eq!(dock_at(at(3.0, 15.0), WORK, None), Some(Dock::Left));
        assert_eq!(dock_at(at(15.0, 3.0), WORK, None), Some(Dock::Top));
    }

    #[test]
    fn dropping_remembers_where() {
        let r = Rect { x: 40.0, y: 0.0, w: 288.0, h: 32.0 };
        assert_eq!(drop_position(r, Some(Dock::Top), WORK), pos(184.0, 0.0, Dock::Top));
        // Near the middle of the top edge: its own place again.
        let mid = Rect { x: 960.0 - 144.0 + 25.0, y: 0.0, w: 288.0, h: 32.0 };
        assert_eq!(drop_position(mid, Some(Dock::Top), WORK), None);
        let r = Rect { x: 0.0, y: 300.0, w: 288.0, h: 32.0 };
        assert_eq!(drop_position(r, Some(Dock::Left), WORK), pos(0.0, 300.0, Dock::Left));
        let r = Rect { x: 700.0, y: 500.0, w: 288.0, h: 32.0 };
        assert_eq!(drop_position(r, None, WORK), pos(844.0, 500.0, Dock::Free));
    }

    #[test]
    fn positions_saved_before_docking_read_as_free() {
        let old: IslandPos = serde_json::from_str(r#"{"x":300,"y":200}"#).unwrap();
        assert_eq!(old.dock, Dock::Free);
        let left: IslandPos = serde_json::from_str(r#"{"x":0,"y":200,"dock":"left"}"#).unwrap();
        assert_eq!(left.dock, Dock::Left);
    }
}
