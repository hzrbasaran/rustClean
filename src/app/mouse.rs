//! Optional mouse support (`M`, off by default): a click selects a row or a
//! treemap block, a double click opens it like `Enter`, the wheel moves
//! through lists and scrolls text.
//!
//! The screens record where their clickable rows and blocks were drawn
//! (`Mouse::hits`, filled by `ui` on every frame); a click is looked up there.
//! A popup clears what it covers, so only its own rows stay clickable.
//! Questions record nothing and take no mouse input at all: moving to the
//! trash, uninstalling and running cleanup commands stay keyboard-only.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use crate::treemap::Slot;
use crate::ui::help::{topic, Topic};

use super::{App, Browser, Pane, Screen};

/// Two clicks on the same thing within this time open it.
const DOUBLE_CLICK: Duration = Duration::from_millis(500);

/// Lines one wheel step scrolls text (help, deletion log, failures).
const WHEEL_LINES: u16 = 3;

/// What a clickable spot on the screen stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// A row of the disk list.
    Disk(usize),
    /// A row of the folder list (an index into `Browser::entries`).
    Entry(usize),
    /// A treemap block.
    Block(Slot),
    /// A row of the open result list.
    Result(usize),
    /// A row of one of the summary's lists.
    Pane(Pane, usize),
    /// An item of the report menu.
    Menu(usize),
    /// A saved scan in the comparison picker.
    Snapshot(usize),
    /// A row of the developer tools list.
    Tool(usize),
}

/// Mouse state: whether it is on, and the clickable areas of the last frame.
#[derive(Default)]
pub struct Mouse {
    /// Mouse capture is on (`M`). Not saved: every start begins without it.
    pub on: bool,
    /// Clickable areas of the last frame, in drawing order.
    hits: Vec<(Rect, Hit)>,
    /// The last click, to recognize a double click.
    last_click: Option<(Instant, Hit)>,
}

impl Mouse {
    /// Forgets the areas of the previous frame.
    pub fn clear(&mut self) {
        self.hits.clear();
    }

    pub fn add(&mut self, area: Rect, hit: Hit) {
        self.hits.push((area, hit));
    }

    /// One area per visible row: `rows` is where the first row is drawn and
    /// how far rows go, `offset` the first one shown, `len` the row count.
    pub fn add_rows(&mut self, rows: Rect, offset: usize, len: usize, hit: impl Fn(usize) -> Hit) {
        for k in 0..rows.height {
            let i = offset + usize::from(k);
            if i >= len {
                break;
            }
            self.add(
                Rect {
                    y: rows.y + k,
                    height: 1,
                    ..rows
                },
                hit(i),
            );
        }
    }

    /// The topmost area at a point: what was drawn last wins.
    pub fn hit_at(&self, column: u16, row: u16) -> Option<Hit> {
        let p = Position::new(column, row);
        self.hits
            .iter()
            .rev()
            .find(|(r, _)| r.contains(p))
            .map(|&(_, h)| h)
    }

    /// Records a click and tells whether it completes a double click.
    fn is_double(&mut self, hit: Hit, now: Instant) -> bool {
        let double = self
            .last_click
            .is_some_and(|(t, h)| h == hit && now.saturating_duration_since(t) <= DOUBLE_CLICK);
        self.last_click = if double { None } else { Some((now, hit)) };
        double
    }
}

impl App {
    /// `M`: mouse capture on / off for this session.
    pub(super) fn toggle_mouse(&mut self) {
        self.mouse.on = !self.mouse.on;
        self.mouse.last_click = None;
        let msg = if self.mouse.on {
            t!(
                "Fare açık: tıklayın, çift tıklayın, tekerlekle kaydırın. Metin seçmek için M ile kapatın.",
                "Mouse on: click, double-click, scroll with the wheel. Press M to turn it off and select text.",
            )
        } else {
            t!("Fare kapalı (M: aç).", "Mouse off (M: turn on).")
        };
        match &mut self.browser {
            Some(b) if matches!(self.screen, Screen::Browser) => b.set_status(msg, false),
            _ => self.message = Some(msg.to_string()),
        }
    }

    pub fn on_mouse(&mut self, event: MouseEvent) {
        self.on_mouse_at(event, Instant::now());
    }

    /// `on_mouse` with the time of the event, so tests can tell single from
    /// double clicks.
    pub(crate) fn on_mouse_at(&mut self, event: MouseEvent, now: Instant) {
        if !self.mouse.on {
            return;
        }
        match event.kind {
            MouseEventKind::ScrollUp => self.wheel(false),
            MouseEventKind::ScrollDown => self.wheel(true),
            MouseEventKind::Down(MouseButton::Left) => self.click(event.column, event.row, now),
            _ => {}
        }
    }

    /// Whether the mouse must do nothing now: a question is open (to move to
    /// the trash, uninstall, or what a tool should clean and whether to run
    /// it), text is being typed, or work runs that only `Esc` may stop.
    fn mouse_blocked(&self) -> bool {
        match self.screen {
            Screen::Scanning => true,
            Screen::DiskSelect => false,
            Screen::Browser => self.browser.as_ref().is_some_and(Browser::mouse_blocked),
        }
    }

    fn wheel(&mut self, down: bool) {
        let step = |v: &mut u16, by: u16| {
            *v = if down {
                v.saturating_add(by)
            } else {
                v.saturating_sub(by)
            };
        };
        if let Some(scroll) = &mut self.help {
            step(scroll, WHEEL_LINES);
            return;
        }
        if self.mouse_blocked() {
            return;
        }
        match topic(self) {
            Topic::Dialogs => {
                if let Some(d) = self.browser.as_mut().and_then(|b| b.failures.as_mut()) {
                    step(&mut d.scroll, WHEEL_LINES);
                }
                return;
            }
            Topic::Log => {
                if let Some(log) = self.browser.as_mut().and_then(|b| b.deletion_log.as_mut()) {
                    step(&mut log.scroll, WHEEL_LINES);
                }
                return;
            }
            // On the map the arrows move between blocks, so the wheel stays
            // out; the system data and the scan have nothing to scroll.
            Topic::Map | Topic::System | Topic::Scanning => return,
            // Lists move their selection one row per step, like the arrows.
            _ => {}
        }
        let code = if down { KeyCode::Down } else { KeyCode::Up };
        self.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn click(&mut self, column: u16, row: u16, now: Instant) {
        if self.help.is_some() || self.mouse_blocked() {
            return;
        }
        let Some(hit) = self.mouse.hit_at(column, row) else {
            self.mouse.last_click = None;
            return;
        };
        if !self.select(hit) {
            return;
        }
        if self.mouse.is_double(hit, now) {
            self.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        }
    }

    /// Moves the selection to what was clicked, if it is still on screen.
    fn select(&mut self, hit: Hit) -> bool {
        let current = topic(self);
        if let Hit::Disk(i) = hit {
            let ok = matches!(self.screen, Screen::DiskSelect) && i < self.disks.len();
            if ok {
                self.disk_table.select(Some(i));
            }
            return ok;
        }
        let Some(b) = self
            .browser
            .as_mut()
            .filter(|_| matches!(self.screen, Screen::Browser))
        else {
            return false;
        };
        match hit {
            Hit::Disk(_) => false,
            Hit::Entry(i) if current == Topic::List && i < b.entries.len() => {
                b.table.select(Some(i));
                true
            }
            Hit::Block(Slot::Item(i)) if current == Topic::Map && i < b.entries.len() => {
                b.table.select(Some(i));
                true
            }
            Hit::Block(Slot::Other { .. }) if current == Topic::Map => {
                b.select_first_small();
                true
            }
            Hit::Result(i) if matches!(current, Topic::Results | Topic::Apps | Topic::Basket) => {
                let Some(r) = b.results.as_mut().filter(|r| i < r.rows.len()) else {
                    return false;
                };
                r.table.select(Some(i));
                true
            }
            Hit::Pane(pane, i) if current == Topic::Dashboard => {
                let Some(d) = &mut b.dashboard else {
                    return false;
                };
                let (table, len) = match pane {
                    Pane::Files => (&mut d.files, d.stats.top_files.len()),
                    Pane::Dirs => (&mut d.dirs, d.stats.top_dirs.len()),
                };
                if i >= len {
                    return false;
                }
                table.select(Some(i));
                d.focus = pane;
                true
            }
            Hit::Menu(i) if i < crate::reports::MenuItem::ALL.len() => match &mut b.report_menu {
                Some(sel) => {
                    *sel = i;
                    true
                }
                None => false,
            },
            Hit::Snapshot(i) => match &mut b.snapshot_picker {
                Some((saved, sel)) if i < saved.len() && b.report_menu.is_none() => {
                    *sel = i;
                    true
                }
                _ => false,
            },
            Hit::Tool(i) if current == Topic::Tools => match &mut b.tools {
                Some(view) if i < view.tools.len() => {
                    view.table.select(Some(i));
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }
}

impl Browser {
    /// See `App::mouse_blocked`.
    fn mouse_blocked(&self) -> bool {
        self.deleting.is_some()
            || self.confirm.is_some()
            || self.uninstall.is_some()
            || self.pending_report.is_some()
            || self.rescan.is_some()
            || self.dup_job.is_some()
            || self.input.is_some()
            || self
                .tools
                .as_ref()
                .is_some_and(|t| t.picker.is_some() || t.confirm.is_some() || t.running())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u16, y: u16, width: u16, height: u16) -> Rect {
        Rect::new(x, y, width, height)
    }

    #[test]
    fn rows_cover_only_the_visible_entries() {
        let mut m = Mouse::default();
        // Five rows of room, scrolled to row 10 of 12: two rows are drawn.
        m.add_rows(rect(0, 3, 40, 5), 10, 12, Hit::Entry);
        assert_eq!(m.hit_at(0, 2), None);
        assert_eq!(m.hit_at(0, 3), Some(Hit::Entry(10)));
        assert_eq!(m.hit_at(39, 4), Some(Hit::Entry(11)));
        assert_eq!(m.hit_at(0, 5), None);
        assert_eq!(m.hit_at(40, 3), None);
    }

    #[test]
    fn what_is_drawn_last_wins() {
        let mut m = Mouse::default();
        m.add_rows(rect(0, 0, 80, 20), 0, 20, Hit::Result);
        m.add(rect(10, 5, 20, 1), Hit::Menu(2));
        assert_eq!(m.hit_at(15, 5), Some(Hit::Menu(2)));
        assert_eq!(m.hit_at(5, 5), Some(Hit::Result(5)));
        m.clear();
        assert_eq!(m.hit_at(15, 5), None);
    }

    #[test]
    fn double_click_needs_the_same_target_in_time() {
        let mut m = Mouse::default();
        let t = Instant::now();
        let ms = |n| t + Duration::from_millis(n);
        assert!(!m.is_double(Hit::Entry(1), t));
        assert!(m.is_double(Hit::Entry(1), ms(300)));
        // A third click starts over.
        assert!(!m.is_double(Hit::Entry(1), ms(400)));
        // Another row, or too late, is a new single click.
        assert!(!m.is_double(Hit::Entry(2), ms(500)));
        assert!(!m.is_double(Hit::Entry(2), ms(1200)));
        assert!(m.is_double(Hit::Entry(2), ms(1300)));
    }
}
