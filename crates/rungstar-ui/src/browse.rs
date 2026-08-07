//! The three ways of showing a song list, as geometry and state with nothing drawn.
//!
//! All three answer the same question — *given N songs and a cursor, which ones are visible
//! and where* — so they are one state machine with three placement functions rather than three
//! screens. Switching layout keeps the cursor, the filter and the scroll position, because
//! none of those belong to the layout.
//!
//! Scrolling is animated by keeping the cursor and the *view* as separate values: moving the
//! cursor leaves the view behind and the view eases after it. Holding a direction therefore
//! glides instead of stepping, and — the part that matters on a 30,000 song library — the
//! cursor is never waiting for an animation, so input is never dropped during a fast scroll.

use serde::{Deserialize, Serialize};

use crate::geom::Rect;

/// How the songs are arranged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Layout {
    /// A vertical strip, cursor centred. Fastest to read, best for long lists.
    #[default]
    List,
    /// A grid of covers. Best when you recognise artwork faster than titles.
    Chessboard,
    /// Covers on an arc, the cursor largest at the front. UltraStar's signature look.
    Roulette,
}

impl Layout {
    pub const ALL: [Layout; 3] = [Layout::List, Layout::Chessboard, Layout::Roulette];

    pub fn name(self) -> &'static str {
        match self {
            Self::List => "List",
            Self::Chessboard => "Chessboard",
            Self::Roulette => "Roulette",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::List => Self::Chessboard,
            Self::Chessboard => Self::Roulette,
            Self::Roulette => Self::List,
        }
    }

    pub fn previous(self) -> Self {
        self.next().next()
    }

    /// Whether left and right move the cursor by one, rather than by a page.
    ///
    /// In a grid they are the horizontal axis; in a strip or an arc they are the fast scroll.
    pub fn horizontal_steps(self) -> bool {
        matches!(self, Self::Chessboard)
    }
}

impl crate::settings::Choice for Layout {
    const VALUES: &'static [Self] = &Self::ALL;

    fn label(self) -> &'static str {
        self.name()
    }
}

/// One visible item and where to draw it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Index into the song list.
    pub index: usize,
    pub rect: Rect,
    /// `1.0` at the cursor, falling off with distance. Drives size, opacity and which items
    /// get a label — a fully general "how prominent is this" so the three layouts share code.
    pub emphasis: f32,
    /// Whether this is the cursor. Exactly one placement has it, when the list is not empty.
    pub selected: bool,
    /// How far from the cursor, in items, signed. Used for depth ordering in Roulette.
    pub distance: f32,
}

/// Cursor and scroll state over a list of songs.
#[derive(Debug, Clone)]
pub struct Browser {
    pub layout: Layout,
    count: usize,
    cursor: usize,
    /// How far the view lags the cursor, in items. Eases to zero.
    lag: f32,
    /// Rows visible in the chessboard, remembered so paging matches what is on screen.
    grid_columns: usize,
}

/// Seconds for the lag to fall to `1/e` of what it was.
///
/// Applied as `lag *= exp(-dt / TAU)`, which is framerate independent by construction: a 60 Hz
/// display and a 144 Hz one settle at the same speed rather than the faster one feeling
/// snappier. At this value a single step lands in about a fifth of a second — long enough to
/// read as movement, short enough that it is over before the next press.
const EASE_TAU: f32 = 0.07;

/// The furthest the view is allowed to fall behind, in items.
///
/// Without this, holding a direction over ten thousand songs builds a lag the animation then
/// has to unwind for several seconds after you let go. Clamping means the list always settles
/// within one glide of the last input.
const MAX_LAG: f32 = 3.0;

/// Below this the animation is over; keeping it running costs a redraw per frame forever.
const SETTLED: f32 = 0.002;

impl Default for Browser {
    fn default() -> Self {
        Self::new()
    }
}

impl Browser {
    pub fn new() -> Self {
        Self {
            layout: Layout::default(),
            count: 0,
            cursor: 0,
            lag: 0.0,
            grid_columns: 1,
        }
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Whether the view is still catching up, and so whether a redraw is needed.
    pub fn animating(&self) -> bool {
        self.lag.abs() > SETTLED
    }

    /// Point at a new list. The cursor is kept if it still exists, because re-running a search
    /// that returns the same song at the same place should not move the selection.
    pub fn set_count(&mut self, count: usize) {
        self.count = count;
        if count == 0 {
            self.cursor = 0;
            self.lag = 0.0;
        } else if self.cursor >= count {
            self.cursor = count - 1;
            self.lag = 0.0;
        }
    }

    /// Jump straight to an index with no animation. Used by search and by "random".
    pub fn jump_to(&mut self, index: usize) {
        if self.count == 0 {
            return;
        }
        self.cursor = index.min(self.count - 1);
        self.lag = 0.0;
    }

    /// Move the cursor, wrapping at both ends, and leave the view behind to catch up.
    pub fn move_by(&mut self, delta: isize) {
        if self.count == 0 || delta == 0 {
            return;
        }
        let count = self.count as isize;
        let next = (self.cursor as isize + delta).rem_euclid(count);
        // Wrapping the long way round would send the view scrolling through the whole library.
        // Animate the short way and let the wrap happen instantly.
        let visual_delta = if delta.unsigned_abs() <= self.count / 2 {
            delta as f32
        } else {
            0.0
        };
        self.cursor = next as usize;
        self.lag = (self.lag - visual_delta).clamp(-MAX_LAG, MAX_LAG);
    }

    /// Advance the animation.
    pub fn tick(&mut self, dt: f32) {
        if !self.animating() {
            self.lag = 0.0;
            return;
        }
        self.lag *= (-dt.max(0.0) / EASE_TAU).exp();
    }

    /// How many items a page-up or page-down moves, for the area last laid out.
    pub fn page_size(&self, area: Rect) -> usize {
        match self.layout {
            Layout::List => rows_in(area).max(1),
            Layout::Chessboard => {
                let (cols, rows) = grid_shape(area);
                (cols * rows).max(1)
            }
            Layout::Roulette => ROULETTE_VISIBLE.max(1),
        }
    }

    /// The visible items, nearest the cursor last so a painter's-algorithm backend draws the
    /// cursor on top without needing a depth buffer.
    pub fn placements(&mut self, area: Rect) -> Vec<Placement> {
        if self.count == 0 || area.w <= 0.0 || area.h <= 0.0 {
            return Vec::new();
        }
        let mut placements = match self.layout {
            Layout::List => self.list_placements(area),
            Layout::Chessboard => self.grid_placements(area),
            Layout::Roulette => self.roulette_placements(area),
        };
        placements.sort_by(|a, b| {
            a.emphasis
                .partial_cmp(&b.emphasis)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        placements
    }

    /// The item at a point, for mouse and touch.
    pub fn hit(&mut self, area: Rect, point: crate::geom::Point) -> Option<usize> {
        self.placements(area)
            .into_iter()
            .rev()
            .find(|p| p.rect.contains(point))
            .map(|p| p.index)
    }

    /// Resolve a relative offset from the cursor into a real index, wrapping.
    ///
    /// Wrapping matters for more than tidiness: with fewer items than slots the same song
    /// would otherwise appear several times in one view, and clicking the second copy would
    /// select the first.
    fn index_at(&self, offset: isize) -> Option<usize> {
        let count = self.count as isize;
        // The window has to be `count` *consecutive* offsets, not `-count..count`: with two
        // songs, offsets -1 and +1 both wrap to the same one, and it would be drawn above and
        // below the cursor at once.
        let lowest = -((count - 1) / 2);
        let highest = count / 2;
        if offset < lowest || offset > highest {
            return None;
        }
        Some((self.cursor as isize + offset).rem_euclid(count) as usize)
    }

    fn list_placements(&self, area: Rect) -> Vec<Placement> {
        let row_h = row_height(area);
        let visible = rows_in(area);
        // One extra either side, so a row slides in already drawn rather than appearing.
        let half = (visible / 2) as isize + 1;
        let centre = area.center();

        (-half..=half)
            .filter_map(|k| {
                let index = self.index_at(k)?;
                let slot = k as f32 - self.lag;
                let y = centre.y + slot * row_h - row_h / 2.0;
                let rect = Rect::new(area.x, y, area.w, row_h);
                // Nothing to draw if it has slid off the end.
                area.intersect(&rect)?;
                Some(Placement {
                    index,
                    rect,
                    emphasis: falloff(slot, 1.0),
                    selected: k == 0,
                    distance: slot,
                })
            })
            .collect()
    }

    fn grid_placements(&mut self, area: Rect) -> Vec<Placement> {
        let (cols, rows) = grid_shape(area);
        self.grid_columns = cols;
        let per_page = cols * rows;
        if per_page == 0 {
            return Vec::new();
        }

        // The cursor's row is held at the middle of the grid and the content scrolls past it,
        // rather than the cursor walking to the bottom and the page jumping. A jump loses your
        // place; a scroll never does.
        let cursor_row = (self.cursor / cols) as isize;
        let middle_row = (rows / 2) as isize;
        let cell_w = area.w / cols as f32;
        let cell_h = area.h / rows as f32;
        let gap = (cell_w.min(cell_h) * 0.06).max(2.0);
        let lag_rows = self.lag / cols as f32;

        let mut out = Vec::with_capacity(per_page + cols * 2);
        for row_offset in -1..=(rows as isize) {
            let row = cursor_row - middle_row + row_offset;
            for col in 0..cols {
                let index = (row * cols as isize) + col as isize;
                if index < 0 || index >= self.count as isize {
                    continue;
                }
                let index = index as usize;
                let x = area.x + col as f32 * cell_w;
                let y = area.y + (row_offset as f32 - lag_rows) * cell_h;
                let rect = Rect::new(x, y, cell_w, cell_h).inset(gap);
                if area.intersect(&rect).is_none() {
                    continue;
                }
                let selected = index == self.cursor;
                out.push(Placement {
                    index,
                    rect,
                    emphasis: if selected { 1.0 } else { 0.45 },
                    selected,
                    distance: index as f32 - self.cursor as f32,
                });
            }
        }
        out
    }

    fn roulette_placements(&self, area: Rect) -> Vec<Placement> {
        let half = (ROULETTE_VISIBLE / 2) as isize;
        // The arc is as tall as the area and the covers are square, sized so the front one
        // dominates without crowding its neighbours off the screen.
        let front = area.h.min(area.w * 0.42) * 0.62;
        let centre = area.center();
        // Horizontal spread, in units per slot. Tight enough that the neighbours overlap
        // slightly, which is what makes it read as a carousel rather than a row.
        let spread = front * 0.62;

        (-half..=half)
            .filter_map(|k| {
                let index = self.index_at(k)?;
                let slot = k as f32 - self.lag;
                let emphasis = falloff(slot, 1.6);
                // Scale and vertical drop both follow emphasis, so the front cover is big and
                // level and the far ones are small and low — depth without a projection.
                let size = front * (0.55 + 0.45 * emphasis);
                let x = centre.x + slot * spread;
                let y = centre.y + (1.0 - emphasis) * area.h * 0.10;
                let rect = Rect::new(0.0, 0.0, size, size).offset(x - size / 2.0, y - size / 2.0);
                Some(Placement {
                    index,
                    rect,
                    emphasis,
                    selected: k == 0,
                    distance: slot,
                })
            })
            .collect()
    }
}

/// Covers shown on the arc, including the front one. Odd, so there is a true centre.
const ROULETTE_VISIBLE: usize = 9;

/// Design units per list row. Scales with the area so a tall screen shows more, but stays
/// within a band that keeps text readable and touch targets hittable.
fn row_height(area: Rect) -> f32 {
    (area.h / 11.0).clamp(52.0, 96.0)
}

fn rows_in(area: Rect) -> usize {
    (area.h / row_height(area)).floor().max(1.0) as usize
}

/// Columns and rows for a chessboard in this area, aiming at roughly square cells.
fn grid_shape(area: Rect) -> (usize, usize) {
    if area.w <= 0.0 || area.h <= 0.0 {
        return (1, 1);
    }
    // Target cell size in design units: big enough to recognise a cover, small enough that a
    // screenful is worth scanning.
    const TARGET: f32 = 210.0;
    let cols = (area.w / TARGET).round().clamp(2.0, 8.0) as usize;
    let cell = area.w / cols as f32;
    let rows = (area.h / cell).round().clamp(1.0, 8.0) as usize;
    (cols, rows)
}

/// Emphasis from distance: `1.0` at zero, falling smoothly away.
///
/// `sharpness` sets how fast: the list wants a gentle fade over several rows, the roulette
/// wants the front cover clearly ahead of its neighbours.
fn falloff(distance: f32, sharpness: f32) -> f32 {
    1.0 / (1.0 + (distance * sharpness).abs().powi(2))
}
