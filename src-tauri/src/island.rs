// Island window: placement on the chosen display, the two window sizes
// (full panel / invisible wake strip), dragging, click-through and the cursor
// poll.
//
// There is no notch on a PC, so the island is a black shape drawn at the top
// centre of the main display inside a borderless, transparent, always-on-top
// window that never takes focus. The user can drag it anywhere on the display.
// Let go near the top, left or right edge, it docks there (at the sides as a
// slim tab) and hides into that edge; elsewhere it stays where it was left.
// Where the window goes is worked out in src/placement.rs.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Monitor, PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::placement::{self, Placement, Rect, Work};
use crate::platform::{self, cursor_physical, left_button_down};
use crate::settings::{Dock, IslandPos};

/// Logical size of the full window — the largest island view, like the macOS panel.
pub const PANEL_W: f64 = 720.0;
pub const PANEL_H: f64 = 320.0;
/// Logical size of the invisible strip that wakes the island when it is hidden.
pub const STRIP_W: f64 = 240.0;
pub const STRIP_H: f64 = 6.0;
/// Height of the compact island docked to a side edge (SIDE_H in
/// src/core/layout.ts). Hidden there, the wake strip stands upright: STRIP_H
/// wide and this tall.
pub const SIDE_H: f64 = 112.0;

pub const WINDOW_LABEL: &str = "island";

/// Margin around the island that still counts as "on the island", in logical px.
/// Wider than the macOS 6 pt because a click must never be swallowed.
const HIT_MARGIN: f64 = 14.0;

#[derive(Serialize, Clone)]
pub struct CursorPayload {
    pub x: f64,
    pub y: f64,
}

#[derive(Serialize, Clone)]
pub struct ScreenInfo {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

/// The island shape in window-logical coordinates, pushed by the front end.
/// The poll thread owns the click-through decision so it lands in the same 16 ms
/// tick as the cursor read — an IPC round trip here loses clicks.
#[derive(Clone, Copy, Default)]
pub struct IslandRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Where the island hangs inside the full window (window-logical) and the edge
/// it is docked to: the front end draws the island from this.
#[derive(Serialize, Clone, Copy)]
pub struct Anchor {
    pub x: f64,
    pub y: f64,
    pub dock: Dock,
}

/// Wakes / parks the cursor poll thread so a hidden island costs literally nothing.
pub struct PollGate {
    active: Mutex<bool>,
    cv: Condvar,
    pub collapsed: AtomicBool,
    pub rect: Mutex<IslandRect>,
    /// Mirrors the window flag so we only call into the OS when it changes.
    ignoring: AtomicBool,
    /// The island is being dragged: the window keeps the mouse throughout.
    pub dragging: AtomicBool,
    /// The last anchor sent, for a page that loads after it was.
    pub anchor: Mutex<Anchor>,
}

impl PollGate {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(false),
            cv: Condvar::new(),
            collapsed: AtomicBool::new(true),
            rect: Mutex::new(IslandRect::default()),
            ignoring: AtomicBool::new(false),
            dragging: AtomicBool::new(false),
            anchor: Mutex::new(Anchor { x: PANEL_W / 2.0, y: 0.0, dock: Dock::Top }),
        }
    }

    pub fn set_rect(&self, rect: IslandRect) {
        *self.rect.lock().unwrap() = rect;
    }

    /// Forces the next poll tick to re-apply the flag (after a window resize).
    pub fn forget_ignore_state(&self) {
        self.ignoring.store(false, Ordering::Relaxed);
    }

    pub fn set_active(&self, on: bool) {
        let mut guard = self.active.lock().unwrap();
        *guard = on;
        self.cv.notify_all();
    }

    fn wait_until_active(&self) {
        let mut guard = self.active.lock().unwrap();
        while !*guard {
            guard = self.cv.wait(guard).unwrap();
        }
    }

    fn is_active(&self) -> bool {
        *self.active.lock().unwrap()
    }
}

pub fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(WINDOW_LABEL)
}

fn monitor_contains(m: &Monitor, x: f64, y: f64) -> bool {
    let p = m.position();
    let s = m.size();
    x >= p.x as f64
        && x < (p.x + s.width as i32) as f64
        && y >= p.y as f64
        && y < (p.y + s.height as i32) as f64
}

/// The display the island lives on: the primary one, or the one under the cursor.
fn target_monitor(app: &AppHandle, pref: &str) -> Option<Monitor> {
    let monitors = app.available_monitors().ok()?;
    if pref == "cursor" {
        if let Some((cx, cy)) = cursor_physical() {
            if let Some(m) = monitors.iter().find(|m| monitor_contains(m, cx, cy)) {
                return Some(m.clone());
            }
        }
    }
    app.primary_monitor()
        .ok()
        .flatten()
        .or_else(|| monitors.into_iter().next())
}

pub fn screen_info(app: &AppHandle, pref: &str) -> ScreenInfo {
    match target_monitor(app, pref) {
        Some(m) => {
            let scale = m.scale_factor();
            let p = m.position();
            let s = m.size();
            ScreenInfo {
                x: p.x as f64 / scale,
                y: p.y as f64 / scale,
                width: s.width as f64 / scale,
                height: s.height as f64 / scale,
                scale,
            }
        }
        None => ScreenInfo { x: 0.0, y: 0.0, width: 1920.0, height: 1080.0, scale: 1.0 },
    }
}

/// The display's work area — the screen minus the taskbar — and its scale:
/// src/placement.rs works in logical pixels from its top-left corner.
struct Area {
    origin: PhysicalPosition<i32>,
    scale: f64,
    work: Work,
}

impl Area {
    fn of(m: &Monitor) -> Self {
        let scale = m.scale_factor();
        let wa = m.work_area();
        Self {
            origin: wa.position,
            scale,
            work: Work { w: wa.size.width as f64 / scale, h: wa.size.height as f64 / scale },
        }
    }

    fn to_physical(&self, r: Rect) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
        let s = self.scale;
        (
            PhysicalPosition::new(
                self.origin.x + (r.x * s).round() as i32,
                self.origin.y + (r.y * s).round() as i32,
            ),
            PhysicalSize::new((r.w * s).round().max(1.0) as u32, (r.h * s).round().max(1.0) as u32),
        )
    }

    fn to_logical(&self, x: f64, y: f64) -> (f64, f64) {
        ((x - self.origin.x as f64) / self.scale, (y - self.origin.y as f64) / self.scale)
    }
}

/// Places and sizes the window. `collapsed` picks the wake strip instead of the
/// panel; `pos` is where the user dragged the island, None for the top centre.
/// The front end is told where the island hangs inside the window.
pub fn apply_geometry(app: &AppHandle, pref: &str, collapsed: bool, pos: Option<IslandPos>) {
    let Some(win) = window(app) else { return };
    let Some(m) = target_monitor(app, pref) else { return };
    let area = Area::of(&m);
    let placed = placement::place(pos, area.work, collapsed);
    let (origin, size) = area.to_physical(placed.window);
    let (x, y) = (origin.x, origin.y);
    let (pw, ph) = (size.width, size.height);

    // GTK never sizes a non-resizable window below its natural size (200 px
    // here), so on Linux the 6 px wake strip would stay a 200 px block. tao
    // re-applies the config's `resizable: false` after the first configure, so
    // this is asked every time, just before the resize. Undecorated, the window
    // still offers the user nothing to resize it by. (Found by @YossiYad, #44.)
    #[cfg(target_os = "linux")]
    let _ = win.set_resizable(true);
    let _ = win.set_size(PhysicalSize::new(pw, ph));
    let _ = win.set_position(PhysicalPosition::new(x, y));
    // Moving across displays can rescale the window: re-assert the physical size.
    let _ = win.set_size(PhysicalSize::new(pw, ph));
    let _ = win.set_always_on_top(true);
    publish_anchor(app, &placed);
}

fn publish_anchor(app: &AppHandle, placed: &Placement) {
    let anchor = Anchor { x: placed.anchor.0, y: placed.anchor.1, dock: placed.dock };
    if let Some(shared) = app.try_state::<crate::Shared>() {
        *shared.gate.anchor.lock().unwrap() = anchor;
    }
    let _ = app.emit_to(WINDOW_LABEL, "island-anchor", anchor);
}

/// How often a drag follows the mouse.
const DRAG_TICK: Duration = Duration::from_millis(8);
/// A drag nobody lets go of ends by itself.
const DRAG_LIMIT: Duration = Duration::from_secs(120);

#[derive(Serialize, Clone, Copy)]
struct DragDock {
    dock: Option<Dock>,
}

/// Drags the island with the mouse until the button is let go. `grab` is where
/// it was taken hold of, in logical pixels from the island's top-left corner.
///
/// The OS move loop behind `start_dragging` starts late (it is a posted
/// message), never says when it ends, and lets the window wander off the
/// screen, so the drag is run here instead: every few milliseconds the window
/// follows the cursor, the island stays inside the work area, and the front
/// end is told which edge it would dock to (for the preview). Escape puts it
/// back where it was.
pub fn spawn_drag(app: AppHandle, grab: (f64, f64)) {
    let gate = app.state::<crate::Shared>().gate.clone();
    if gate.dragging.swap(true, Ordering::SeqCst) {
        return;
    }
    // Without a cursor to follow (Linux), the system moves the window; where
    // it was left is not remembered then.
    if !platform::CURSOR_POLL {
        if let Some(win) = window(&app) {
            let _ = win.start_dragging();
        }
        gate.dragging.store(false, Ordering::SeqCst);
        return;
    }
    std::thread::spawn(move || {
        run_drag(&app, &gate, grab);
        gate.dragging.store(false, Ordering::SeqCst);
        refresh_click_through(&app, &gate);
        let _ = app.emit_to(WINDOW_LABEL, "island-drag-end", ());
    });
}

fn run_drag(app: &AppHandle, gate: &PollGate, grab: (f64, f64)) {
    let shared = app.state::<crate::Shared>();
    let pref = shared.settings.lock().unwrap().screen.clone();
    let Some(win) = window(app) else { return };
    let Some(m) = target_monitor(app, &pref) else { return };
    let area = Area::of(&m);
    set_ignore_cursor(app, false);
    gate.forget_ignore_state();

    let started = Instant::now();
    let mut offered: Option<Dock> = None;
    let mut island: Option<Rect> = None;
    let mut last_origin: Option<PhysicalPosition<i32>> = None;
    let mut cancelled = false;
    loop {
        if !left_button_down() || started.elapsed() > DRAG_LIMIT {
            break;
        }
        if platform::escape_down() {
            cancelled = true;
            break;
        }
        if let Some((cx, cy)) = cursor_physical() {
            // The island's shape inside the window, as last drawn: it can
            // change size mid-drag (an open island closing on its timer).
            let r = *gate.rect.lock().unwrap();
            let (lx, ly) = area.to_logical(cx, cy);
            let (ix, iy) = placement::clamp_island(lx - grab.0, ly - grab.1, r.w, r.h, area.work);
            let here = Rect { x: ix, y: iy, w: r.w, h: r.h };
            island = Some(here);

            let dock = placement::dock_at(here, area.work, offered);
            if dock != offered {
                offered = dock;
                let _ = app.emit_to(WINDOW_LABEL, "island-drag-dock", DragDock { dock });
            }

            // The window carries the island at the same place inside it for
            // the whole drag, so the island follows the cursor exactly; it is
            // put back inside the screen once the island is let go.
            let (origin, _) = area.to_physical(Rect { x: ix - r.x, y: iy - r.y, w: PANEL_W, h: PANEL_H });
            if last_origin != Some(origin) {
                last_origin = Some(origin);
                let _ = win.set_position(origin);
            }
        }
        std::thread::sleep(DRAG_TICK);
    }
    if offered.is_some() {
        let _ = app.emit_to(WINDOW_LABEL, "island-drag-dock", DragDock { dock: None });
    }

    // Let go before the first step, it never moved: nothing to remember.
    let dropped = island.filter(|_| !cancelled);
    let updated = {
        let mut current = shared.settings.lock().unwrap();
        if let Some(here) = dropped {
            current.island_pos = placement::drop_position(here, offered, area.work);
        }
        current.clone()
    };
    if dropped.is_some() {
        if let Err(err) = crate::settings::save(&updated) {
            crate::log::line(format!("could not save the island position: {err}"));
        }
    }
    // Docked, or back inside the screen, or (cancelled) where it was.
    let collapsed = gate.collapsed.load(Ordering::Relaxed);
    apply_geometry(app, &updated.screen, collapsed, updated.island_pos);
    let _ = app.emit("settings-changed", updated);
}

/// Position, size and scale of the monitor the island lives on. Any change here
/// means the island has to be placed again.
fn current_screen_key(app: &AppHandle) -> Option<(i32, i32, u32, u32, u64)> {
    let pref = app
        .try_state::<crate::Shared>()
        .map(|s| s.settings.lock().unwrap().screen.clone())
        .unwrap_or_else(|| "primary".into());
    let m = target_monitor(app, &pref)?;
    let p = m.position();
    let size = m.size();
    Some((p.x, p.y, size.width, size.height, m.scale_factor().to_bits()))
}

/// Emits `cursor` (window-logical coordinates) at ~60 Hz while the island is
/// visible. Parked on a condvar the rest of the time.
pub fn spawn_cursor_poll(app: AppHandle, gate: Arc<PollGate>) {
    std::thread::spawn(move || {
        // Remembered across wakes so a display change while hidden is noticed the
        // moment the island comes back.
        let mut last_screen: Option<(i32, i32, u32, u32, u64)> = None;
        // Without a cursor to read (Linux) the loop only watches the display
        // layout, and twice a second is plenty for that: waking at 60 Hz just to
        // find no cursor costs CPU for nothing.
        let (period, screen_every) = if platform::CURSOR_POLL { (16, 30) } else { (500, 1) };
        loop {
            gate.wait_until_active();
            let mut last = (f64::MIN, f64::MIN);
            let mut ticks: u32 = 0;
            while gate.is_active() {
                std::thread::sleep(Duration::from_millis(period));

                // Monitors get plugged in, unplugged, rearranged and rescaled, and
                // an island pinned to coordinates that no longer exist is an island
                // nobody can reach. Checked about twice a second — the cursor poll
                // is already running, so this costs one monitor query.
                ticks = ticks.wrapping_add(1);
                if ticks % screen_every == 0 {
                    let now = current_screen_key(&app);
                    if now.is_some() && now != last_screen {
                        let first = last_screen.is_none();
                        last_screen = now;
                        if !first {
                            crate::log::line("display layout changed — repositioning".to_string());
                            let _ = app.emit_to(WINDOW_LABEL, "screen-changed", ());
                        }
                    }
                }

                let Some(win) = window(&app) else { continue };
                let Ok(origin) = win.outer_position() else { continue };
                let scale = win.scale_factor().unwrap_or(1.0);
                let Some((cx, cy)) = cursor_physical() else { continue };
                let x = (cx - origin.x as f64) / scale;
                let y = (cy - origin.y as f64) / scale;
                let size = match win.inner_size() {
                    Ok(s) => (s.width as f64 / scale, s.height as f64 / scale),
                    Err(_) => (PANEL_W, PANEL_H),
                };
                if (x - last.0).abs() < 1.0 && (y - last.1).abs() < 1.0 {
                    continue;
                }
                last = (x, y);

                // Click-through: the window only takes the mouse over the island
                // shape. A small entry margin means the flag is already off by the
                // time a moving cursor reaches a button.
                let r = *gate.rect.lock().unwrap();
                let on_island = r.w > 0.0
                    && x >= r.x - HIT_MARGIN
                    && x <= r.x + r.w + HIT_MARGIN
                    && y >= r.y - HIT_MARGIN
                    && y <= r.y + r.h + HIT_MARGIN;

                // A file being dragged has to be able to find us. WS_EX_TRANSPARENT
                // — what click-through is on Windows — hides the window from
                // WindowFromPoint, so OLE finds no drop target and shows the "no
                // drop" cursor. macOS has no such problem: AppKit delivers drags to
                // registered destinations whatever ignoresMouseEvents says. So while
                // a button is held anywhere over the panel, the whole panel takes
                // the mouse, which also makes the drop zone as forgiving as the Mac's.
                let dragging = left_button_down()
                    && x >= 0.0
                    && x <= size.0
                    && y >= 0.0
                    && y <= size.1;

                let accept = on_island || dragging || gate.dragging.load(Ordering::Relaxed);
                if gate.ignoring.load(Ordering::Relaxed) == accept {
                    gate.ignoring.store(!accept, Ordering::Relaxed);
                    let _ = win.set_ignore_cursor_events(!accept);
                }

                let _ = win.emit("cursor", CursorPayload { x, y });
            }
        }
    });
}

/// Re-applies click-through after the window or the island changed shape.
///
/// With the cursor poll (Windows) the window takes the mouse again and the next
/// tick decides from the cursor. Without it (Linux) the input region is set to
/// the island itself, or to the whole wake strip while collapsed.
pub fn refresh_click_through(app: &AppHandle, gate: &PollGate) {
    if platform::CURSOR_POLL {
        set_ignore_cursor(app, false);
        gate.forget_ignore_state();
        return;
    }
    let Some(win) = window(app) else { return };
    let region = if gate.collapsed.load(Ordering::Relaxed) {
        // The wake strip itself, never "the whole window": if the window ever
        // fails to shrink to the strip, the rest of it must not swallow clicks
        // meant for whatever sits under the top of the screen.
        let side = app
            .try_state::<crate::Shared>()
            .and_then(|s| s.settings.lock().unwrap().island_pos)
            .is_some_and(|p| matches!(p.dock, Dock::Left | Dock::Right));
        let (w, h) = if side { (STRIP_H, SIDE_H) } else { (STRIP_W, STRIP_H) };
        Some((0.0, 0.0, w, h))
    } else {
        let r = *gate.rect.lock().unwrap();
        if r.w <= 0.0 {
            // Nothing drawn yet: nothing takes the mouse.
            Some((0.0, 0.0, 0.0, 0.0))
        } else {
            let x0 = (r.x - HIT_MARGIN).max(0.0);
            let y0 = (r.y - HIT_MARGIN).max(0.0);
            let x1 = r.x + r.w + HIT_MARGIN;
            let y1 = r.y + r.h + HIT_MARGIN;
            Some((x0, y0, x1 - x0, y1 - y0))
        }
    };
    platform::set_input_region(&win, region);
}

pub fn set_ignore_cursor(app: &AppHandle, ignore: bool) {
    if let Some(win) = window(app) {
        let _ = win.set_ignore_cursor_events(ignore);
    }
}
