//! Checked whole-scene rendering and paint acknowledgement for the ordinary fixture's target.
//!
//! The retained target publishes one client scene: the deterministic background, the
//! optional `watch-marker-v1`, and the ninety visual-token cells, all taken from one
//! command snapshot. The scene is drawn into a GUI-thread-owned, bounded, reusable
//! off-screen buffer and reaches the window through one final copy after construction.
//! Publication failure is reported without assuming that the destination is unchanged.
//! A failed allocation, selection, draw, publication or release fails the synchronous
//! repaint request instead of acknowledging success. One final copy narrows
//! application-created intermediate scenes; it is not a
//! compositor or Windows Graphics Capture atomicity claim.

use std::ffi::c_void;
use std::mem::size_of;

use mado_pilot_platform_windows::fixture_protocol::{
    FixtureVisualCommand, WATCH_MARKER_CELL_SIZE, WATCH_MARKER_HEIGHT, WATCH_MARKER_PRIMARY_RGB,
    WATCH_MARKER_SECONDARY_RGB, WATCH_MARKER_WIDTH, WATCH_MARKER_X, WATCH_MARKER_Y,
    WATCH_TOKEN_CELL_COUNT, WATCH_TOKEN_CELL_SIZE, WATCH_TOKEN_GRID_HEIGHT, WATCH_TOKEN_GRID_WIDTH,
    WATCH_TOKEN_X, WATCH_TOKEN_Y, visual_token_cell,
};
use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleDC, CreateDIBSection,
    CreateSolidBrush, DIB_RGB_COLORS, DeleteDC, DeleteObject, FillRect, HBITMAP, HBRUSH, HDC,
    HGDIOBJ, SRCCOPY, SelectObject,
};

/// Largest client width or height, in pixels, that the renderer backs off-screen.
pub(crate) const MAX_SCENE_DIMENSION: i32 = 16_384;
/// Largest off-screen backing store, in bytes, that the renderer allocates.
pub(crate) const MAX_SCENE_BYTES: usize = 128 * 1024 * 1024;
const BYTES_PER_PIXEL: usize = 4;

const MARKER: RECT = RECT {
    left: WATCH_MARKER_X,
    top: WATCH_MARKER_Y,
    right: WATCH_MARKER_X + WATCH_MARKER_WIDTH,
    bottom: WATCH_MARKER_Y + WATCH_MARKER_HEIGHT,
};
const MARKER_TOP_MIDDLE: RECT = RECT {
    left: WATCH_MARKER_X + WATCH_MARKER_CELL_SIZE,
    top: WATCH_MARKER_Y,
    right: WATCH_MARKER_X + WATCH_MARKER_CELL_SIZE * 2,
    bottom: WATCH_MARKER_Y + WATCH_MARKER_CELL_SIZE,
};
const MARKER_BOTTOM_LEFT: RECT = RECT {
    left: WATCH_MARKER_X,
    top: WATCH_MARKER_Y + WATCH_MARKER_CELL_SIZE,
    right: WATCH_MARKER_X + WATCH_MARKER_CELL_SIZE,
    bottom: WATCH_MARKER_Y + WATCH_MARKER_HEIGHT,
};

/// Why one requested target paint did not publish its scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaintFailure {
    /// No retained target exists, so there is nothing to repaint.
    NoTarget,
    /// Invalidation or the synchronous update request was refused.
    RepaintRequest,
    /// No paint of the requested generation reached the requested window.
    Unpainted,
    /// The paint of the requested generation rendered a different command snapshot.
    StaleSnapshot,
    /// The paint session or the client rectangle was unavailable.
    PaintSession,
    /// The client area is empty; there is no surface to publish.
    EmptyClient,
    /// The visual command's marker or token would be clipped by the client area.
    IncompleteScene,
    /// The client area exceeds the dimension or byte bound.
    ExtentExceedsBound,
    /// The memory device context could not be created.
    MemoryDevice,
    /// The backing bitmap could not be created.
    BackingBitmap,
    /// The backing bitmap could not be selected into its device context.
    BitmapSelection,
    /// A solid brush could not be created.
    Brush,
    /// One rectangle fill was refused.
    Fill,
    /// The final copy into the window did not complete.
    Publish,
    /// A brush, bitmap or device-context release was refused.
    Release,
}

impl PaintFailure {
    /// Returns the closed ASCII label reported in a failed control outcome.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::NoTarget => "no-target",
            Self::RepaintRequest => "repaint-request",
            Self::Unpainted => "unpainted",
            Self::StaleSnapshot => "stale-snapshot",
            Self::PaintSession => "paint-session",
            Self::EmptyClient => "empty-client",
            Self::IncompleteScene => "incomplete-scene",
            Self::ExtentExceedsBound => "extent-bound",
            Self::MemoryDevice => "memory-device",
            Self::BackingBitmap => "backing-bitmap",
            Self::BitmapSelection => "bitmap-selection",
            Self::Brush => "brush",
            Self::Fill => "fill",
            Self::Publish => "publish",
            Self::Release => "release",
        }
    }
}

/// A validated, non-empty, bounded client extent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SceneExtent {
    width: i32,
    height: i32,
}

impl SceneExtent {
    /// Validates one client rectangle before any native allocation.
    pub(crate) fn from_client(client: &RECT) -> Result<Self, PaintFailure> {
        let width = client
            .right
            .checked_sub(client.left)
            .filter(|width| *width > 0);
        let height = client
            .bottom
            .checked_sub(client.top)
            .filter(|height| *height > 0);
        let (Some(width), Some(height)) = (width, height) else {
            return Err(PaintFailure::EmptyClient);
        };
        if width > MAX_SCENE_DIMENSION || height > MAX_SCENE_DIMENSION {
            return Err(PaintFailure::ExtentExceedsBound);
        }
        let bytes = usize::try_from(width)
            .ok()
            .zip(usize::try_from(height).ok())
            .and_then(|(width, height)| width.checked_mul(height))
            .and_then(|pixels| pixels.checked_mul(BYTES_PER_PIXEL));
        match bytes {
            Some(bytes) if bytes <= MAX_SCENE_BYTES => Ok(Self { width, height }),
            _ => Err(PaintFailure::ExtentExceedsBound),
        }
    }

    /// Returns the validated width in pixels.
    pub(crate) const fn width(self) -> i32 {
        self.width
    }

    /// Returns the validated height in pixels.
    pub(crate) const fn height(self) -> i32 {
        self.height
    }
}

/// One synchronous repaint request awaiting the paint it causes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "a request is resolved only through `TargetScene::outcome`"]
pub(crate) struct PaintRequest {
    generation: u64,
}

/// The exact GDI calls the scene renderer issues.
///
/// `Gdi` forwards each call to the process GDI; tests script outcomes to drive the
/// renderer's failure and release paths. This is a call seam, not a drawing abstraction:
/// it exists so that sequencing, ownership balance and acknowledgement logic can be
/// exercised without a desktop.
pub(crate) trait SceneGdi {
    /// Creates a screen-compatible memory device context.
    fn create_memory_dc(&mut self) -> Option<HDC>;
    /// Creates a 32-bit backing bitmap of exactly `extent`.
    fn create_backing_bitmap(&mut self, extent: SceneExtent) -> Option<HBITMAP>;
    /// Selects `object` into `dc`, returning the object it replaced.
    fn select_object(&mut self, dc: HDC, object: HGDIOBJ) -> Option<HGDIOBJ>;
    /// Deletes an unselected brush or bitmap.
    fn delete_object(&mut self, object: HGDIOBJ) -> bool;
    /// Deletes a memory device context.
    fn delete_dc(&mut self, dc: HDC) -> bool;
    /// Creates a solid brush of the `0xRRGGBB` colour.
    fn create_solid_brush(&mut self, rgb: u32) -> Option<HBRUSH>;
    /// Fills `rect` in `dc` with `brush`.
    fn fill_rect(&mut self, dc: HDC, rect: &RECT, brush: HBRUSH) -> bool;
    /// Copies `extent` pixels from the origin of `source` to the origin of `destination`.
    fn copy_to(&mut self, destination: HDC, source: HDC, extent: SceneExtent) -> bool;
}

/// The process GDI, owned by whichever GUI thread issues the calls.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Gdi;

impl SceneGdi for Gdi {
    fn create_memory_dc(&mut self) -> Option<HDC> {
        // SAFETY: a null reference device requests a memory DC compatible with the screen.
        // The handle is owned by this thread until `delete_dc` releases it.
        let dc = unsafe { CreateCompatibleDC(None) };
        (!dc.is_invalid()).then_some(dc)
    }

    fn create_backing_bitmap(&mut self, extent: SceneExtent) -> Option<HBITMAP> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: u32::try_from(size_of::<BITMAPINFOHEADER>())
                    .expect("BITMAPINFOHEADER fits u32"),
                biWidth: extent.width(),
                // A negative height requests top-down rows so buffer row `y` is client row `y`.
                biHeight: -extent.height(),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..BITMAPINFOHEADER::default()
            },
            ..BITMAPINFO::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        // SAFETY: `info` is a complete 32-bit BI_RGB header whose extent was bounded before
        // this call, `bits` is a writable out-pointer, and no file mapping is supplied. The
        // returned bitmap is owned by this thread until `delete_object` releases it.
        unsafe {
            CreateDIBSection(
                None,
                &raw const info,
                DIB_RGB_COLORS,
                &raw mut bits,
                None,
                0,
            )
        }
        .ok()
    }

    fn select_object(&mut self, dc: HDC, object: HGDIOBJ) -> Option<HGDIOBJ> {
        // SAFETY: both handles are live GDI objects owned by this thread.
        let previous = unsafe { SelectObject(dc, object) };
        (!previous.is_invalid()).then_some(previous)
    }

    fn delete_object(&mut self, object: HGDIOBJ) -> bool {
        // SAFETY: the object is owned by this thread and is no longer selected into any DC.
        unsafe { DeleteObject(object) }.as_bool()
    }

    fn delete_dc(&mut self, dc: HDC) -> bool {
        // SAFETY: the memory DC is owned by this thread and holds only its stock bitmap again.
        unsafe { DeleteDC(dc) }.as_bool()
    }

    fn create_solid_brush(&mut self, rgb: u32) -> Option<HBRUSH> {
        // SAFETY: creating a process-owned brush has no preconditions; ownership passes to
        // the caller, who deletes it.
        let brush = unsafe { CreateSolidBrush(colorref(rgb)) };
        (!brush.is_invalid()).then_some(brush)
    }

    fn fill_rect(&mut self, dc: HDC, rect: &RECT, brush: HBRUSH) -> bool {
        // SAFETY: dc, rect and brush are live for the duration of this call.
        let filled = unsafe { FillRect(dc, rect, brush) };
        filled != 0
    }

    fn copy_to(&mut self, destination: HDC, source: HDC, extent: SceneExtent) -> bool {
        // SAFETY: both device contexts are live and the source bitmap covers `extent` exactly.
        unsafe {
            BitBlt(
                destination,
                0,
                0,
                extent.width(),
                extent.height(),
                Some(source),
                0,
                0,
                SRCCOPY,
            )
        }
        .is_ok()
    }
}

/// One off-screen buffer: a memory device context with the backing bitmap selected.
#[derive(Debug)]
struct SceneBuffer {
    extent: SceneExtent,
    dc: HDC,
    bitmap: HBITMAP,
    /// The stock bitmap the device held before ours; restored before ours is deleted.
    previous: HGDIOBJ,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PaintRecord {
    generation: u64,
    window: usize,
    snapshot: u64,
    outcome: Result<SceneExtent, PaintFailure>,
}

/// GUI-thread-owned renderer state for the retained target.
///
/// Holds at most one off-screen buffer, reused while the client extent is unchanged, plus
/// the record that ties the most recent target paint to the repaint request which caused
/// it. Dropping the owner releases the buffer. No method here dispatches window messages,
/// so a caller that holds no borrow across `UpdateWindow` cannot re-enter it.
#[derive(Debug)]
pub(crate) struct TargetScene<G: SceneGdi> {
    gdi: G,
    buffer: Option<SceneBuffer>,
    requested: u64,
    completed: Option<PaintRecord>,
    release_failed: bool,
}

impl<G: SceneGdi> TargetScene<G> {
    /// Creates an owner with no buffer and no open request.
    pub(crate) const fn new(gdi: G) -> Self {
        Self {
            gdi,
            buffer: None,
            requested: 0,
            completed: None,
            release_failed: false,
        }
    }

    /// Opens the next repaint generation.
    ///
    /// Only the paint recorded after this call, and before the next one, can resolve the
    /// returned request; an earlier success never does.
    pub(crate) fn request_paint(&mut self) -> PaintRequest {
        self.requested = self.requested.wrapping_add(1);
        self.completed = None;
        PaintRequest {
            generation: self.requested,
        }
    }

    /// Paints the complete client scene for `snapshot` into `destination` and records the
    /// result for `window` against the open generation.
    ///
    /// `snapshot` is the packed visual command; zero paints the background alone. The buffer
    /// is reused for an unchanged extent and replaced otherwise. Any failed step releases
    /// the buffer and returns before publication.
    pub(crate) fn paint(
        &mut self,
        window: usize,
        destination: HDC,
        client: &RECT,
        background_rgb: u32,
        snapshot: u64,
    ) -> Result<SceneExtent, PaintFailure> {
        let outcome = self.publish(
            destination,
            client,
            background_rgb,
            FixtureVisualCommand::from_packed(snapshot),
        );
        self.record(window, snapshot, outcome);
        outcome
    }

    /// Records a paint that failed before any scene work could start.
    pub(crate) fn record_failure(
        &mut self,
        window: usize,
        snapshot: u64,
        failure: PaintFailure,
    ) -> Result<SceneExtent, PaintFailure> {
        let failure = if self.release().is_err() {
            PaintFailure::Release
        } else {
            failure
        };
        self.record(window, snapshot, Err(failure));
        Err(failure)
    }

    /// Resolves `request`: success only when the paint recorded for its generation belongs
    /// to `window`, rendered `snapshot`, and published.
    pub(crate) fn outcome(
        &self,
        request: PaintRequest,
        window: usize,
        snapshot: u64,
    ) -> Result<SceneExtent, PaintFailure> {
        let Some(record) = self
            .completed
            .filter(|record| record.generation == request.generation && record.window == window)
        else {
            return Err(PaintFailure::Unpainted);
        };
        if record.snapshot != snapshot {
            return Err(PaintFailure::StaleSnapshot);
        }
        record.outcome
    }

    /// Releases the buffer, if one is held. An error means a GDI object may have leaked.
    pub(crate) fn release(&mut self) -> Result<(), PaintFailure> {
        let released = match self.buffer.take() {
            Some(buffer) => release_buffer(&mut self.gdi, buffer),
            None => Ok(()),
        };
        self.release_failed |= released.is_err();
        if self.release_failed {
            Err(PaintFailure::Release)
        } else {
            Ok(())
        }
    }

    fn record(&mut self, window: usize, snapshot: u64, outcome: Result<SceneExtent, PaintFailure>) {
        self.completed = Some(PaintRecord {
            generation: self.requested,
            window,
            snapshot,
            outcome,
        });
    }

    fn publish(
        &mut self,
        destination: HDC,
        client: &RECT,
        background_rgb: u32,
        command: Option<FixtureVisualCommand>,
    ) -> Result<SceneExtent, PaintFailure> {
        if self.release_failed {
            return Err(PaintFailure::Release);
        }
        let extent = SceneExtent::from_client(client).and_then(|extent| {
            let token_right = WATCH_TOKEN_X
                + WATCH_TOKEN_CELL_SIZE
                    * i32::try_from(WATCH_TOKEN_GRID_WIDTH).expect("token columns fit i32");
            let token_bottom = WATCH_TOKEN_Y
                + WATCH_TOKEN_CELL_SIZE
                    * i32::try_from(WATCH_TOKEN_GRID_HEIGHT).expect("token rows fit i32");
            if command.is_some()
                && (extent.width() < MARKER.right.max(token_right)
                    || extent.height() < MARKER.bottom.max(token_bottom))
            {
                Err(PaintFailure::IncompleteScene)
            } else {
                Ok(extent)
            }
        });
        // A changed or unusable extent never keeps obsolete storage alive.
        if self
            .buffer
            .as_ref()
            .is_some_and(|buffer| Ok(buffer.extent) != extent)
        {
            self.release()?;
        }
        let extent = extent?;
        let buffer = match self.buffer.take() {
            Some(buffer) => buffer,
            None => match allocate_buffer(&mut self.gdi, extent) {
                Ok(buffer) => buffer,
                Err(failure) => {
                    self.release_failed |= failure == PaintFailure::Release;
                    return Err(failure);
                }
            },
        };
        let published = draw_scene(&mut self.gdi, &buffer, background_rgb, command)
            .and_then(|()| copy_scene(&mut self.gdi, &buffer, destination));
        match published {
            Ok(()) => {
                self.buffer = Some(buffer);
                Ok(extent)
            }
            Err(failure) => {
                // A release refusal is terminal for this owner: further paints must not
                // repeatedly allocate resources whose predecessors could not be released.
                let released = release_buffer(&mut self.gdi, buffer);
                self.release_failed = failure == PaintFailure::Release || released.is_err();
                Err(if self.release_failed {
                    PaintFailure::Release
                } else {
                    failure
                })
            }
        }
    }
}

impl<G: SceneGdi> Drop for TargetScene<G> {
    fn drop(&mut self) {
        if let Some(buffer) = self.buffer.take() {
            let _released = release_buffer(&mut self.gdi, buffer);
        }
    }
}

fn allocate_buffer<G: SceneGdi>(
    gdi: &mut G,
    extent: SceneExtent,
) -> Result<SceneBuffer, PaintFailure> {
    let dc = gdi.create_memory_dc().ok_or(PaintFailure::MemoryDevice)?;
    let Some(bitmap) = gdi.create_backing_bitmap(extent) else {
        return Err(if gdi.delete_dc(dc) {
            PaintFailure::BackingBitmap
        } else {
            PaintFailure::Release
        });
    };
    let Some(previous) = gdi.select_object(dc, bitmap.into()) else {
        let bitmap_deleted = gdi.delete_object(bitmap.into());
        let dc_deleted = gdi.delete_dc(dc);
        return Err(if bitmap_deleted && dc_deleted {
            PaintFailure::BitmapSelection
        } else {
            PaintFailure::Release
        });
    };
    Ok(SceneBuffer {
        extent,
        dc,
        bitmap,
        previous,
    })
}

fn release_buffer<G: SceneGdi>(gdi: &mut G, buffer: SceneBuffer) -> Result<(), PaintFailure> {
    // Restore before deleting. If restoration fails, successfully deleting the DC
    // first removes its selection; never attempt to delete a still-selected bitmap.
    let restored = gdi.select_object(buffer.dc, buffer.previous).is_some();
    let (bitmap_deleted, dc_deleted) = if restored {
        (
            gdi.delete_object(buffer.bitmap.into()),
            gdi.delete_dc(buffer.dc),
        )
    } else {
        let dc_deleted = gdi.delete_dc(buffer.dc);
        let bitmap_deleted = dc_deleted && gdi.delete_object(buffer.bitmap.into());
        (bitmap_deleted, dc_deleted)
    };
    (restored && bitmap_deleted && dc_deleted)
        .then_some(())
        .ok_or(PaintFailure::Release)
}

fn draw_scene<G: SceneGdi>(
    gdi: &mut G,
    buffer: &SceneBuffer,
    background_rgb: u32,
    command: Option<FixtureVisualCommand>,
) -> Result<(), PaintFailure> {
    let brushes = SceneBrushes::create(gdi, background_rgb)?;
    let drawn = brushes.draw(gdi, buffer, command);
    let released = brushes.release(gdi);
    released.and(drawn)
}

fn copy_scene<G: SceneGdi>(
    gdi: &mut G,
    buffer: &SceneBuffer,
    destination: HDC,
) -> Result<(), PaintFailure> {
    gdi.copy_to(destination, buffer.dc, buffer.extent)
        .then_some(())
        .ok_or(PaintFailure::Publish)
}

struct SceneBrushes {
    background: HBRUSH,
    primary: HBRUSH,
    secondary: HBRUSH,
}

impl SceneBrushes {
    fn create<G: SceneGdi>(gdi: &mut G, background_rgb: u32) -> Result<Self, PaintFailure> {
        let background = gdi
            .create_solid_brush(background_rgb)
            .ok_or(PaintFailure::Brush)?;
        let Some(primary) = gdi.create_solid_brush(WATCH_MARKER_PRIMARY_RGB) else {
            return Err(if gdi.delete_object(background.into()) {
                PaintFailure::Brush
            } else {
                PaintFailure::Release
            });
        };
        let Some(secondary) = gdi.create_solid_brush(WATCH_MARKER_SECONDARY_RGB) else {
            let background_deleted = gdi.delete_object(background.into());
            let primary_deleted = gdi.delete_object(primary.into());
            return Err(if background_deleted && primary_deleted {
                PaintFailure::Brush
            } else {
                PaintFailure::Release
            });
        };
        Ok(Self {
            background,
            primary,
            secondary,
        })
    }

    /// Draws the background, then the marker when visible, then every token cell, in the
    /// same order and at the same coordinates the fixture has always used.
    fn draw<G: SceneGdi>(
        &self,
        gdi: &mut G,
        buffer: &SceneBuffer,
        command: Option<FixtureVisualCommand>,
    ) -> Result<(), PaintFailure> {
        let client = RECT {
            left: 0,
            top: 0,
            right: buffer.extent.width(),
            bottom: buffer.extent.height(),
        };
        fill(gdi, buffer.dc, &client, self.background)?;
        let Some(command) = command else {
            return Ok(());
        };
        if command.state().marker_is_visible() {
            fill(gdi, buffer.dc, &MARKER, self.primary)?;
            fill(gdi, buffer.dc, &MARKER_TOP_MIDDLE, self.secondary)?;
            fill(gdi, buffer.dc, &MARKER_BOTTOM_LEFT, self.secondary)?;
        }
        for index in 0..WATCH_TOKEN_CELL_COUNT {
            let lit = visual_token_cell(command, index)
                .expect("index is bounded by WATCH_TOKEN_CELL_COUNT");
            let brush = if lit { self.primary } else { self.secondary };
            fill(gdi, buffer.dc, &token_cell(index), brush)?;
        }
        Ok(())
    }

    fn release<G: SceneGdi>(self, gdi: &mut G) -> Result<(), PaintFailure> {
        // FillRect never leaves a brush selected, so all three delete directly. Every
        // deletion runs even after an earlier refusal.
        let background = gdi.delete_object(self.background.into());
        let primary = gdi.delete_object(self.primary.into());
        let secondary = gdi.delete_object(self.secondary.into());
        (background && primary && secondary)
            .then_some(())
            .ok_or(PaintFailure::Release)
    }
}

fn fill<G: SceneGdi>(gdi: &mut G, dc: HDC, rect: &RECT, brush: HBRUSH) -> Result<(), PaintFailure> {
    gdi.fill_rect(dc, rect, brush)
        .then_some(())
        .ok_or(PaintFailure::Fill)
}

/// Returns the client rectangle of visual-token cell `index` in row-major order.
fn token_cell(index: usize) -> RECT {
    let column =
        i32::try_from(index % WATCH_TOKEN_GRID_WIDTH).expect("visual-token column fits i32");
    let row = i32::try_from(index / WATCH_TOKEN_GRID_WIDTH).expect("visual-token row fits i32");
    let left = WATCH_TOKEN_X + column * WATCH_TOKEN_CELL_SIZE;
    let top = WATCH_TOKEN_Y + row * WATCH_TOKEN_CELL_SIZE;
    RECT {
        left,
        top,
        right: left + WATCH_TOKEN_CELL_SIZE,
        bottom: top + WATCH_TOKEN_CELL_SIZE,
    }
}

fn colorref(rgb: u32) -> COLORREF {
    let red = rgb & 0xff_0000;
    let green = rgb & 0x00_ff00;
    let blue = rgb & 0x00_00ff;
    COLORREF((red >> 16) | green | (blue << 16))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::num::NonZeroU32;
    use std::rc::Rc;
    use std::sync::Mutex;

    use mado_pilot_platform_windows::fixture_protocol::{
        BENCHMARK_FILL_RGB, FILL_RGB, FixtureVisualState,
    };
    use windows::Win32::Graphics::Gdi::GetPixel;
    use windows::Win32::System::Threading::{GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources};

    use super::*;

    const WINDOW: usize = 0x0001_0000;
    const OTHER_WINDOW: usize = 0x0002_0000;
    const DESTINATION: HDC = HDC(std::ptr::without_provenance_mut(0x0d00));
    /// The stock bitmap a fresh scripted device holds; never allocated, never deleted.
    const STOCK_BITMAP: usize = 0x0010;
    /// Cell index of the payload bit that mirrors the marker state.
    const MARKER_STATE_CELL: usize = WATCH_TOKEN_GRID_WIDTH + 64;
    const SCENE_FILLS: usize = 1 + 3 + WATCH_TOKEN_CELL_COUNT;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    fn client(width: i32, height: i32) -> RECT {
        rect(0, 0, width, height)
    }

    fn command(state: FixtureVisualState, token: u32) -> FixtureVisualCommand {
        FixtureVisualCommand::new(state, NonZeroU32::new(token).expect("nonzero visual token"))
    }

    fn visible(token: u32) -> FixtureVisualCommand {
        command(FixtureVisualState::Visible, token)
    }

    fn absent(token: u32) -> FixtureVisualCommand {
        command(FixtureVisualState::Absent, token)
    }

    fn handle(value: usize) -> *mut c_void {
        std::ptr::without_provenance_mut(value)
    }

    /// One scripted call refusal. Each fault is consumed by the first call it names.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Fault {
        MemoryDevice,
        BackingBitmap,
        Selection,
        Restoration,
        /// The n-th brush created through this ledger, counted from zero across paints.
        Brush(usize),
        /// The n-th fill attempted through this ledger, counted from zero across paints.
        Fill(usize),
        Copy,
        DeleteBrush,
        DeleteBitmap,
        DeleteDevice,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Kind {
        Device,
        Bitmap,
        Brush(u32),
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Event {
        CreateDevice(usize),
        CreateBitmap(usize, SceneExtent),
        Select(usize, usize),
        DeleteBitmap(usize),
        DeleteDevice(usize),
        Copy(usize),
    }

    /// Records every GDI-shaped call the renderer makes and enforces GDI's ownership rules.
    ///
    /// A refused deletion retains the object, matching the native ownership boundary.
    #[derive(Debug, Default)]
    struct Ledger {
        next: usize,
        live: BTreeMap<usize, Kind>,
        /// Device -> selected bitmap. Absent means the stock bitmap is selected.
        selected: BTreeMap<usize, usize>,
        fills: Vec<(usize, RECT, u32)>,
        copies: Vec<(HDC, usize, SceneExtent)>,
        events: Vec<Event>,
        faults: Vec<Fault>,
        brushes_created: usize,
        fills_attempted: usize,
        violations: Vec<String>,
    }

    impl Ledger {
        fn fault(&mut self, fault: Fault) -> bool {
            let Some(index) = self.faults.iter().position(|pending| *pending == fault) else {
                return false;
            };
            self.faults.remove(index);
            true
        }

        fn allocate(&mut self, kind: Kind) -> usize {
            self.next += 0x10;
            let value = 0x1000 + self.next;
            self.live.insert(value, kind);
            value
        }

        fn live_kind(&mut self, value: usize, call: &str) -> Option<Kind> {
            let kind = self.live.get(&value).copied();
            if kind.is_none() {
                self.violations
                    .push(format!("{call} used a handle that is not live: {value:#x}"));
            }
            kind
        }

        fn scene_fills(&self) -> Vec<(RECT, u32)> {
            self.fills
                .iter()
                .map(|(_, rect, rgb)| (*rect, *rgb))
                .collect()
        }

        fn live_brushes(&self) -> usize {
            self.live
                .values()
                .filter(|kind| matches!(kind, Kind::Brush(_)))
                .count()
        }

        fn bitmaps_created(&self) -> usize {
            self.events
                .iter()
                .filter(|event| matches!(event, Event::CreateBitmap(..)))
                .count()
        }
    }

    #[derive(Debug, Default, Clone)]
    struct ScriptedGdi(Rc<RefCell<Ledger>>);

    impl SceneGdi for ScriptedGdi {
        fn create_memory_dc(&mut self) -> Option<HDC> {
            let mut ledger = self.0.borrow_mut();
            if ledger.fault(Fault::MemoryDevice) {
                return None;
            }
            let value = ledger.allocate(Kind::Device);
            ledger.events.push(Event::CreateDevice(value));
            Some(HDC(handle(value)))
        }

        fn create_backing_bitmap(&mut self, extent: SceneExtent) -> Option<HBITMAP> {
            let mut ledger = self.0.borrow_mut();
            if ledger.fault(Fault::BackingBitmap) {
                return None;
            }
            let value = ledger.allocate(Kind::Bitmap);
            ledger.events.push(Event::CreateBitmap(value, extent));
            Some(HBITMAP(handle(value)))
        }

        fn select_object(&mut self, dc: HDC, object: HGDIOBJ) -> Option<HGDIOBJ> {
            let mut ledger = self.0.borrow_mut();
            let device = dc.0.addr();
            let target = object.0.addr();
            if ledger.live_kind(device, "SelectObject") != Some(Kind::Device) {
                return None;
            }
            let previous = ledger
                .selected
                .get(&device)
                .copied()
                .unwrap_or(STOCK_BITMAP);
            if target == STOCK_BITMAP {
                if ledger.fault(Fault::Restoration) {
                    return None;
                }
                ledger.selected.remove(&device);
            } else {
                if ledger.live_kind(target, "SelectObject") != Some(Kind::Bitmap) {
                    return None;
                }
                if ledger.fault(Fault::Selection) {
                    return None;
                }
                ledger.selected.insert(device, target);
            }
            ledger.events.push(Event::Select(device, target));
            Some(HGDIOBJ(handle(previous)))
        }

        fn delete_object(&mut self, object: HGDIOBJ) -> bool {
            let mut ledger = self.0.borrow_mut();
            let value = object.0.addr();
            if value == STOCK_BITMAP {
                ledger
                    .violations
                    .push("DeleteObject deleted the stock bitmap".to_owned());
                return true;
            }
            let Some(kind) = ledger.live_kind(value, "DeleteObject") else {
                return false;
            };
            if ledger.selected.values().any(|selected| *selected == value) {
                ledger
                    .violations
                    .push(format!("DeleteObject deleted selected bitmap {value:#x}"));
                return false;
            }
            let fault = match kind {
                Kind::Brush(_) => Fault::DeleteBrush,
                Kind::Bitmap => {
                    ledger.events.push(Event::DeleteBitmap(value));
                    Fault::DeleteBitmap
                }
                Kind::Device => {
                    ledger
                        .violations
                        .push(format!("DeleteObject deleted device {value:#x}"));
                    return false;
                }
            };
            if ledger.fault(fault) {
                return false;
            }
            ledger.live.remove(&value);
            true
        }

        fn delete_dc(&mut self, dc: HDC) -> bool {
            let mut ledger = self.0.borrow_mut();
            let value = dc.0.addr();
            if ledger.live_kind(value, "DeleteDC") != Some(Kind::Device) {
                return false;
            }
            if ledger.fault(Fault::DeleteDevice) {
                return false;
            }
            ledger.selected.remove(&value);
            ledger.live.remove(&value);
            ledger.events.push(Event::DeleteDevice(value));
            true
        }

        fn create_solid_brush(&mut self, rgb: u32) -> Option<HBRUSH> {
            let mut ledger = self.0.borrow_mut();
            let index = ledger.brushes_created;
            ledger.brushes_created += 1;
            if ledger.fault(Fault::Brush(index)) {
                return None;
            }
            let value = ledger.allocate(Kind::Brush(rgb));
            Some(HBRUSH(handle(value)))
        }

        fn fill_rect(&mut self, dc: HDC, rect: &RECT, brush: HBRUSH) -> bool {
            let mut ledger = self.0.borrow_mut();
            let index = ledger.fills_attempted;
            ledger.fills_attempted += 1;
            let device = dc.0.addr();
            if ledger.live_kind(device, "FillRect") != Some(Kind::Device) {
                return false;
            }
            let Some(Kind::Brush(rgb)) = ledger.live_kind(brush.0.addr(), "FillRect") else {
                ledger
                    .violations
                    .push("FillRect used a handle that is not a live brush".to_owned());
                return false;
            };
            if !ledger.selected.contains_key(&device) {
                ledger
                    .violations
                    .push("FillRect drew into a device holding only its stock bitmap".to_owned());
                return false;
            }
            if ledger.fault(Fault::Fill(index)) {
                return false;
            }
            ledger.fills.push((device, *rect, rgb));
            true
        }

        fn copy_to(&mut self, destination: HDC, source: HDC, extent: SceneExtent) -> bool {
            let mut ledger = self.0.borrow_mut();
            let device = source.0.addr();
            if ledger.live_kind(device, "BitBlt") != Some(Kind::Device) {
                return false;
            }
            if !ledger.selected.contains_key(&device) {
                ledger
                    .violations
                    .push("BitBlt copied from a device holding only its stock bitmap".to_owned());
                return false;
            }
            if ledger.fault(Fault::Copy) {
                return false;
            }
            ledger.copies.push((destination, device, extent));
            ledger.events.push(Event::Copy(device));
            true
        }
    }

    fn scripted(faults: &[Fault]) -> (TargetScene<ScriptedGdi>, Rc<RefCell<Ledger>>) {
        let gdi = ScriptedGdi::default();
        gdi.0.borrow_mut().faults = faults.to_vec();
        let ledger = Rc::clone(&gdi.0);
        (TargetScene::new(gdi), ledger)
    }

    #[test]
    fn visible_command_draws_background_marker_and_every_cell_then_publishes_once() {
        let (mut scene, ledger) = scripted(&[]);
        let command = visible(0x2d5a_9c3f);
        let request = scene.request_paint();
        let extent = scene
            .paint(
                WINDOW,
                DESTINATION,
                &client(360, 240),
                FILL_RGB,
                command.packed(),
            )
            .expect("complete scene published");
        assert_eq!(scene.outcome(request, WINDOW, command.packed()), Ok(extent));

        let ledger = ledger.borrow();
        assert!(ledger.violations.is_empty(), "{:?}", ledger.violations);
        let device = ledger.fills[0].0;
        assert!(ledger.fills.iter().all(|(dc, _, _)| *dc == device));
        let fills = ledger.scene_fills();
        assert_eq!(fills.len(), SCENE_FILLS);
        assert_eq!(fills[0], (client(360, 240), FILL_RGB));
        assert_eq!(fills[1], (rect(64, 48, 136, 96), WATCH_MARKER_PRIMARY_RGB));
        assert_eq!(
            fills[2],
            (rect(88, 48, 112, 72), WATCH_MARKER_SECONDARY_RGB)
        );
        assert_eq!(fills[3], (rect(64, 72, 88, 96), WATCH_MARKER_SECONDARY_RGB));
        for (index, (cell, rgb)) in fills[4..].iter().enumerate() {
            let lit = visual_token_cell(command, index).expect("bounded cell index");
            assert_eq!(*cell, token_cell(index), "cell {index} rectangle");
            let expected = if lit {
                WATCH_MARKER_PRIMARY_RGB
            } else {
                WATCH_MARKER_SECONDARY_RGB
            };
            assert_eq!(*rgb, expected, "cell {index} colour");
        }
        assert_eq!(fills[4 + MARKER_STATE_CELL].1, WATCH_MARKER_PRIMARY_RGB);
        assert_eq!(token_cell(0), rect(176, 48, 184, 56));
        assert_eq!(token_cell(9), rect(248, 48, 256, 56));
        assert_eq!(token_cell(10), rect(176, 56, 184, 64));
        assert_eq!(token_cell(89), rect(248, 112, 256, 120));

        // One copy of the whole extent follows every fill and every brush deletion.
        assert_eq!(ledger.copies.as_slice(), &[(DESTINATION, device, extent)]);
        assert_eq!(ledger.events.last(), Some(&Event::Copy(device)));
        assert_eq!(ledger.live_brushes(), 0);
        // The buffer stays live and selected for the next equal-extent paint.
        assert_eq!(ledger.live.len(), 2);
        assert_eq!(ledger.selected.len(), 1);
    }

    #[test]
    fn absent_and_missing_commands_never_draw_the_marker() {
        let (mut scene, ledger) = scripted(&[]);
        let command = absent(7);
        scene
            .paint(
                WINDOW,
                DESTINATION,
                &client(360, 240),
                BENCHMARK_FILL_RGB,
                command.packed(),
            )
            .expect("absent scene published");
        {
            let ledger = ledger.borrow();
            let fills = ledger.scene_fills();
            assert_eq!(fills.len(), 1 + WATCH_TOKEN_CELL_COUNT);
            assert_eq!(fills[0], (client(360, 240), BENCHMARK_FILL_RGB));
            assert!(
                fills[1..]
                    .iter()
                    .all(|(cell, _)| cell.left >= WATCH_TOKEN_X),
                "no fill reaches the marker area"
            );
            assert_eq!(fills[1 + MARKER_STATE_CELL].1, WATCH_MARKER_SECONDARY_RGB);
        }

        ledger.borrow_mut().fills.clear();
        scene
            .paint(WINDOW, DESTINATION, &client(360, 240), FILL_RGB, 0)
            .expect("background-only scene published");
        let ledger = ledger.borrow();
        assert_eq!(ledger.scene_fills(), [(client(360, 240), FILL_RGB)]);
        assert_eq!(ledger.copies.len(), 2);
        assert!(ledger.violations.is_empty(), "{:?}", ledger.violations);
    }

    #[test]
    fn equal_extents_reuse_one_buffer_and_a_new_extent_replaces_it_in_order() {
        let (mut scene, ledger) = scripted(&[]);
        let snapshot = visible(3).packed();
        scene
            .paint(WINDOW, DESTINATION, &client(360, 240), FILL_RGB, snapshot)
            .expect("first paint");
        scene
            .paint(
                WINDOW,
                DESTINATION,
                &client(360, 240),
                BENCHMARK_FILL_RGB,
                snapshot,
            )
            .expect("pulse with another background");
        {
            let ledger = ledger.borrow();
            assert_eq!(
                ledger.bitmaps_created(),
                1,
                "an equal extent reuses the buffer"
            );
            assert_eq!(ledger.copies.len(), 2);
        }

        let resized = scene
            .paint(WINDOW, DESTINATION, &client(480, 320), FILL_RGB, snapshot)
            .expect("resized paint");
        assert_eq!((resized.width(), resized.height()), (480, 320));
        let ledger = ledger.borrow();
        assert!(ledger.violations.is_empty(), "{:?}", ledger.violations);
        let events: Vec<Event> = ledger
            .events
            .iter()
            .copied()
            .filter(|event| !matches!(event, Event::Copy(_)))
            .collect();
        let [
            Event::CreateDevice(first_device),
            Event::CreateBitmap(first_bitmap, _),
            Event::Select(selected_device, selected_bitmap),
            Event::Select(restored_device, restored_bitmap),
            Event::DeleteBitmap(deleted_bitmap),
            Event::DeleteDevice(deleted_device),
            Event::CreateDevice(second_device),
            Event::CreateBitmap(second_bitmap, second_extent),
            Event::Select(reselected_device, reselected_bitmap),
        ] = events.as_slice()
        else {
            panic!("unexpected buffer lifecycle {events:?}");
        };
        assert_eq!(
            (selected_device, selected_bitmap),
            (first_device, first_bitmap)
        );
        assert_eq!(restored_device, first_device);
        assert_eq!(
            *restored_bitmap, STOCK_BITMAP,
            "the stock bitmap is restored before the obsolete bitmap is deleted"
        );
        assert_eq!(deleted_bitmap, first_bitmap);
        assert_eq!(deleted_device, first_device);
        assert_ne!(second_device, first_device);
        assert_eq!(*second_extent, resized);
        assert_eq!(
            (reselected_device, reselected_bitmap),
            (second_device, second_bitmap)
        );
        assert_eq!(ledger.live.len(), 2);
        assert_eq!(ledger.copies.len(), 3);
    }

    #[test]
    fn every_failed_step_fails_the_request_and_releases_every_object() {
        let snapshot = visible(11).packed();
        let cases = [
            (Fault::MemoryDevice, PaintFailure::MemoryDevice),
            (Fault::BackingBitmap, PaintFailure::BackingBitmap),
            (Fault::Selection, PaintFailure::BitmapSelection),
            (Fault::Brush(0), PaintFailure::Brush),
            (Fault::Brush(2), PaintFailure::Brush),
            (Fault::Fill(0), PaintFailure::Fill),
            (Fault::Fill(3), PaintFailure::Fill),
            (Fault::Fill(SCENE_FILLS - 1), PaintFailure::Fill),
            (Fault::Copy, PaintFailure::Publish),
        ];
        for (fault, expected) in cases {
            let (mut scene, ledger) = scripted(&[fault]);
            let request = scene.request_paint();
            let outcome = scene.paint(WINDOW, DESTINATION, &client(360, 240), FILL_RGB, snapshot);
            assert_eq!(outcome, Err(expected), "{fault:?}");
            assert_eq!(
                scene.outcome(request, WINDOW, snapshot),
                Err(expected),
                "{fault:?} must not acknowledge"
            );
            {
                let ledger = ledger.borrow();
                assert!(ledger.faults.is_empty(), "{fault:?} was never reached");
                assert!(
                    ledger.violations.is_empty(),
                    "{fault:?}: {:?}",
                    ledger.violations
                );
                assert!(ledger.live.is_empty(), "{fault:?} leaked {:?}", ledger.live);
                assert!(ledger.copies.is_empty(), "{fault:?} still published");
            }
            assert!(scene.buffer.is_none(), "{fault:?} kept a buffer");

            // The failure does not latch: the next paint allocates afresh and publishes.
            let request = scene.request_paint();
            let extent = scene
                .paint(WINDOW, DESTINATION, &client(360, 240), FILL_RGB, snapshot)
                .unwrap_or_else(|failure| panic!("{fault:?} latched as {failure:?}"));
            assert_eq!(scene.outcome(request, WINDOW, snapshot), Ok(extent));
            let ledger = ledger.borrow();
            assert_eq!(ledger.live.len(), 2, "{fault:?}");
            assert_eq!(ledger.copies.len(), 1, "{fault:?}");
        }
    }

    #[test]
    fn refused_release_of_an_obsolete_buffer_fails_the_resized_paint() {
        let snapshot = visible(13).packed();
        for fault in [Fault::DeleteBitmap, Fault::DeleteDevice] {
            let (mut scene, ledger) = scripted(&[fault]);
            scene
                .paint(WINDOW, DESTINATION, &client(360, 240), FILL_RGB, snapshot)
                .expect("first paint");
            let request = scene.request_paint();
            assert_eq!(
                scene.paint(WINDOW, DESTINATION, &client(480, 320), FILL_RGB, snapshot),
                Err(PaintFailure::Release),
                "{fault:?}"
            );
            assert_eq!(
                scene.outcome(request, WINDOW, snapshot),
                Err(PaintFailure::Release),
                "{fault:?}"
            );
            assert!(scene.buffer.is_none(), "{fault:?} kept the obsolete buffer");
            let ledger = ledger.borrow();
            assert_eq!(
                ledger.copies.len(),
                1,
                "{fault:?} published the resized scene"
            );
            assert_eq!(
                ledger.bitmaps_created(),
                1,
                "{fault:?} allocated after a refused release"
            );
            assert!(
                ledger.violations.is_empty(),
                "{fault:?}: {:?}",
                ledger.violations
            );
        }
    }

    #[test]
    fn refused_cleanup_blocks_further_allocations() {
        for fault in [Fault::DeleteBrush, Fault::DeleteBitmap, Fault::DeleteDevice] {
            let (mut scene, ledger) = scripted(&[]);
            let snapshot = visible(17).packed();
            scene
                .paint(WINDOW, DESTINATION, &client(360, 240), FILL_RGB, snapshot)
                .expect("initial scene");
            ledger.borrow_mut().faults.push(fault);
            let request = scene.request_paint();
            let size = if fault == Fault::DeleteBrush {
                client(360, 240)
            } else {
                client(480, 320)
            };
            assert_eq!(
                scene.paint(WINDOW, DESTINATION, &size, FILL_RGB, snapshot),
                Err(PaintFailure::Release)
            );
            assert_eq!(
                scene.outcome(request, WINDOW, snapshot),
                Err(PaintFailure::Release)
            );
            let objects = ledger.borrow().live.len();
            let events = ledger.borrow().events.len();
            assert!(objects > 0, "a refused deletion must not fabricate release");
            for _ in 0..3 {
                let request = scene.request_paint();
                assert_eq!(
                    scene.paint(WINDOW, DESTINATION, &size, FILL_RGB, snapshot),
                    Err(PaintFailure::Release)
                );
                assert_eq!(
                    scene.outcome(request, WINDOW, snapshot),
                    Err(PaintFailure::Release)
                );
            }
            assert_eq!(ledger.borrow().live.len(), objects);
            assert_eq!(ledger.borrow().events.len(), events);
            assert_eq!(scene.release(), Err(PaintFailure::Release));
        }
    }

    #[test]
    fn restoration_refusal_releases_the_device_before_the_bitmap() {
        for refuse_device in [false, true] {
            let (mut scene, ledger) = scripted(&[]);
            scene
                .paint(WINDOW, DESTINATION, &client(360, 240), FILL_RGB, 0)
                .expect("initial scene");
            ledger.borrow_mut().faults.push(Fault::Restoration);
            if refuse_device {
                ledger.borrow_mut().faults.push(Fault::DeleteDevice);
            }
            assert_eq!(scene.release(), Err(PaintFailure::Release));
            assert!(ledger.borrow().violations.is_empty());
            assert_eq!(
                ledger.borrow().live.len(),
                if refuse_device { 2 } else { 0 }
            );
            let events = ledger.borrow().events.len();
            assert_eq!(
                scene.paint(WINDOW, DESTINATION, &client(360, 240), FILL_RGB, 0),
                Err(PaintFailure::Release)
            );
            assert_eq!(ledger.borrow().events.len(), events);
        }
    }

    #[test]
    fn acknowledgement_requires_the_requested_generation_window_and_snapshot() {
        let (mut scene, _ledger) = scripted(&[]);
        let first = visible(1).packed();
        let request = scene.request_paint();
        assert_eq!(
            scene.outcome(request, WINDOW, first),
            Err(PaintFailure::Unpainted),
            "a request is unresolved before its paint"
        );
        let extent = scene
            .paint(WINDOW, DESTINATION, &client(360, 240), FILL_RGB, first)
            .expect("first paint");
        assert_eq!(scene.outcome(request, WINDOW, first), Ok(extent));
        assert_eq!(
            scene.outcome(request, OTHER_WINDOW, first),
            Err(PaintFailure::Unpainted)
        );
        assert_eq!(
            scene.outcome(request, WINDOW, absent(1).packed()),
            Err(PaintFailure::StaleSnapshot)
        );

        // A new generation is never satisfied by the previous generation's success.
        let second = scene.request_paint();
        assert_ne!(second, request);
        assert_eq!(
            scene.outcome(second, WINDOW, first),
            Err(PaintFailure::Unpainted)
        );

        // A paint that could not start records its failure against the open generation.
        assert_eq!(
            scene.record_failure(WINDOW, first, PaintFailure::PaintSession),
            Err(PaintFailure::PaintSession)
        );
        assert_eq!(
            scene.outcome(second, WINDOW, first),
            Err(PaintFailure::PaintSession)
        );

        // Only the snapshot the paint actually rendered resolves the request.
        let third = scene.request_paint();
        let updated = absent(2).packed();
        scene
            .paint(WINDOW, DESTINATION, &client(360, 240), FILL_RGB, updated)
            .expect("updated paint");
        assert_eq!(
            scene.outcome(third, WINDOW, first),
            Err(PaintFailure::StaleSnapshot)
        );
        assert_eq!(scene.outcome(third, WINDOW, updated), Ok(extent));
    }

    #[test]
    fn client_extents_are_bounded_before_any_allocation() {
        let (mut scene, ledger) = scripted(&[]);
        let snapshot = visible(5).packed();
        let rejected = [
            (rect(0, 0, 0, 0), PaintFailure::EmptyClient),
            (rect(0, 0, 360, 0), PaintFailure::EmptyClient),
            (rect(10, 10, 10, 200), PaintFailure::EmptyClient),
            (rect(100, 0, 0, 100), PaintFailure::EmptyClient),
            (rect(-1, 0, i32::MAX, 100), PaintFailure::EmptyClient),
            (client(255, 240), PaintFailure::IncompleteScene),
            (client(360, 119), PaintFailure::IncompleteScene),
            (
                rect(0, 0, MAX_SCENE_DIMENSION + 1, 1),
                PaintFailure::ExtentExceedsBound,
            ),
            (
                rect(0, 0, 1, MAX_SCENE_DIMENSION + 1),
                PaintFailure::ExtentExceedsBound,
            ),
            (rect(0, 0, 8_192, 8_192), PaintFailure::ExtentExceedsBound),
        ];
        for (area, expected) in rejected {
            assert_eq!(
                scene.paint(WINDOW, DESTINATION, &area, FILL_RGB, snapshot),
                Err(expected),
                "{area:?}"
            );
        }
        assert!(
            ledger.borrow().events.is_empty(),
            "no native allocation precedes a rejected extent"
        );

        // The largest permitted store allocates; one more row exceeds the byte bound.
        let widest = usize::try_from(MAX_SCENE_DIMENSION).expect("positive dimension");
        let tallest =
            i32::try_from(MAX_SCENE_BYTES / BYTES_PER_PIXEL / widest).expect("row count fits i32");
        let largest = scene
            .paint(
                WINDOW,
                DESTINATION,
                &client(MAX_SCENE_DIMENSION, tallest),
                FILL_RGB,
                snapshot,
            )
            .expect("largest permitted buffer");
        assert_eq!(
            (largest.width(), largest.height()),
            (MAX_SCENE_DIMENSION, tallest)
        );
        assert_eq!(ledger.borrow().live.len(), 2);
        assert_eq!(
            scene.paint(
                WINDOW,
                DESTINATION,
                &client(MAX_SCENE_DIMENSION, tallest + 1),
                FILL_RGB,
                snapshot,
            ),
            Err(PaintFailure::ExtentExceedsBound)
        );
        // Storage stays bounded once the extent becomes unusable.
        assert!(scene.buffer.is_none());
        let ledger = ledger.borrow();
        assert!(ledger.live.is_empty(), "{:?}", ledger.live);
        assert!(ledger.violations.is_empty(), "{:?}", ledger.violations);
    }

    #[test]
    fn release_and_drop_return_the_buffer_and_tolerate_absence() {
        let (mut scene, ledger) = scripted(&[]);
        assert_eq!(scene.release(), Ok(()));
        scene
            .paint(WINDOW, DESTINATION, &client(360, 240), FILL_RGB, 0)
            .expect("paint");
        assert_eq!(ledger.borrow().live.len(), 2);
        assert_eq!(scene.release(), Ok(()));
        assert!(ledger.borrow().live.is_empty());
        assert_eq!(scene.release(), Ok(()), "a second release is a no-op");
        scene
            .paint(WINDOW, DESTINATION, &client(360, 240), FILL_RGB, 0)
            .expect("paint after release reallocates");
        assert_eq!(ledger.borrow().live.len(), 2);
        drop(scene);
        let ledger = ledger.borrow();
        assert!(ledger.live.is_empty(), "drop released {:?}", ledger.live);
        assert!(ledger.violations.is_empty(), "{:?}", ledger.violations);
    }

    static PROCESS_GDI: Mutex<()> = Mutex::new(());

    fn gdi_object_count() -> u32 {
        // SAFETY: the current-process pseudo-handle needs no acquisition or release.
        unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) }
    }

    /// Reads one published pixel as `0x00BBGGRR`, ignoring the unused fourth byte.
    fn pixel(dc: HDC, x: i32, y: i32) -> u32 {
        // SAFETY: dc is a live memory device owned by this test and (x, y) lies inside its
        // bitmap.
        let colour = unsafe { GetPixel(dc, x, y) };
        colour.0 & 0x00ff_ffff
    }

    fn center(rect: RECT) -> (i32, i32) {
        ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)
    }

    fn assert_token_pixels(dc: HDC, command: FixtureVisualCommand) {
        for index in 0..WATCH_TOKEN_CELL_COUNT {
            let (x, y) = center(token_cell(index));
            let expected = if visual_token_cell(command, index).expect("bounded cell index") {
                WATCH_MARKER_PRIMARY_RGB
            } else {
                WATCH_MARKER_SECONDARY_RGB
            };
            assert_eq!(pixel(dc, x, y), colorref(expected).0, "token cell {index}");
        }
    }

    /// Capture-free check of the process GDI path: exact pixels, the visible-to-absent
    /// transition, a resize, a refused publication and object balance, all in owned
    /// off-screen devices. It does not qualify Windows Graphics Capture.
    #[test]
    fn process_gdi_publishes_exact_scene_pixels_and_releases_every_object() {
        let _serial = PROCESS_GDI.lock().expect("process GDI checks serialized");
        let mut gdi = Gdi;
        let extent = SceneExtent::from_client(&client(360, 240)).expect("bounded client");
        // Exercise GDI once so lazily created per-process objects do not skew the baseline.
        let warm = allocate_buffer(&mut gdi, extent).expect("warm-up buffer");
        release_buffer(&mut gdi, warm).expect("warm-up release");
        let baseline = gdi_object_count();

        let destination = allocate_buffer(&mut gdi, extent).expect("owned destination buffer");
        assert_eq!(gdi_object_count(), baseline + 2);

        let mut scene = TargetScene::new(Gdi);
        let shown = visible(0x5a3c_96e1);
        let request = scene.request_paint();
        assert_eq!(
            scene.paint(
                WINDOW,
                destination.dc,
                &client(360, 240),
                FILL_RGB,
                shown.packed(),
            ),
            Ok(extent)
        );
        assert_eq!(scene.outcome(request, WINDOW, shown.packed()), Ok(extent));
        assert_eq!(
            gdi_object_count(),
            baseline + 4,
            "one device and one bitmap back the scene; no brush survives the paint"
        );
        assert_eq!(pixel(destination.dc, 0, 0), colorref(FILL_RGB).0);
        assert_eq!(pixel(destination.dc, 359, 239), colorref(FILL_RGB).0);
        let marker_cells = [
            (76, 60, WATCH_MARKER_PRIMARY_RGB),
            (100, 60, WATCH_MARKER_SECONDARY_RGB),
            (124, 60, WATCH_MARKER_PRIMARY_RGB),
            (76, 84, WATCH_MARKER_SECONDARY_RGB),
            (100, 84, WATCH_MARKER_PRIMARY_RGB),
            (124, 84, WATCH_MARKER_PRIMARY_RGB),
        ];
        for (x, y, rgb) in marker_cells {
            assert_eq!(
                pixel(destination.dc, x, y),
                colorref(rgb).0,
                "marker cell at ({x}, {y})"
            );
        }
        assert_token_pixels(destination.dc, shown);

        // Visible to absent: the marker vanishes because the whole background is redrawn,
        // and the buffer is reused rather than reallocated.
        let cleared = absent(7);
        assert_eq!(
            scene.paint(
                WINDOW,
                destination.dc,
                &client(360, 240),
                BENCHMARK_FILL_RGB,
                cleared.packed(),
            ),
            Ok(extent)
        );
        assert_eq!(gdi_object_count(), baseline + 4);
        for (x, y, _) in marker_cells {
            assert_eq!(
                pixel(destination.dc, x, y),
                colorref(BENCHMARK_FILL_RGB).0,
                "cleared marker cell at ({x}, {y})"
            );
        }
        assert_token_pixels(destination.dc, cleared);

        // A resize replaces the buffer without leaking the obsolete one.
        let resized = scene
            .paint(
                WINDOW,
                destination.dc,
                &client(480, 320),
                FILL_RGB,
                shown.packed(),
            )
            .expect("resized scene published into the clipped destination");
        assert_eq!((resized.width(), resized.height()), (480, 320));
        assert_eq!(gdi_object_count(), baseline + 4);

        // A refused publication fails the request and releases the buffer.
        let request = scene.request_paint();
        assert_eq!(
            scene.paint(
                WINDOW,
                HDC::default(),
                &client(480, 320),
                FILL_RGB,
                shown.packed(),
            ),
            Err(PaintFailure::Publish)
        );
        assert_eq!(
            scene.outcome(request, WINDOW, shown.packed()),
            Err(PaintFailure::Publish)
        );
        assert!(scene.buffer.is_none());
        assert_eq!(gdi_object_count(), baseline + 2);

        assert_eq!(scene.release(), Ok(()));
        drop(scene);
        release_buffer(&mut gdi, destination).expect("destination release");
        assert_eq!(gdi_object_count(), baseline);
    }
}
