//! State and keys of the developer tools cleanup screen.

use std::sync::mpsc::Receiver;

use crossterm::event::KeyCode;
use ratatui::widgets::TableState;

use crate::tools::{self, CleanAction, Measurement, Risk, RunEvent, Status, Tool};

/// Lines of command output kept for the log panel.
const LOG_LINES: usize = 200;

/// Choosing which actions of a tool to run.
pub struct Picker {
    pub tool: usize,
    pub checked: Vec<bool>,
    pub cursor: usize,
}

/// Waiting for the user to confirm the chosen actions.
pub struct Confirm {
    pub tool: usize,
    pub actions: Vec<CleanAction>,
    /// Text typed so far, when a data-loss action requires typing "evet".
    pub typed: Option<String>,
}

/// Actions being run (or just finished).
pub struct Run {
    pub tool: usize,
    pub log: Vec<String>,
    pub finished: bool,
    pub failures: usize,
    events: Receiver<RunEvent>,
}

pub enum ToolsKey {
    None,
    Quit,
    Close,
    Menu,
}

pub struct ToolsView {
    pub tools: Vec<Tool>,
    pub table: TableState,
    pub picker: Option<Picker>,
    pub confirm: Option<Confirm>,
    pub run: Option<Run>,
    measurements: Receiver<Measurement>,
}

pub const CONFIRM_WORD: &str = "evet";

impl ToolsView {
    pub fn open() -> Self {
        let (tools, measurements) = tools::measure_all();
        let mut table = TableState::default();
        table.select(Some(0));
        Self {
            tools,
            table,
            picker: None,
            confirm: None,
            run: None,
            measurements,
        }
    }

    pub fn poll(&mut self) {
        while let Ok((kind, status, actions)) = self.measurements.try_recv() {
            if let Some(t) = self.tools.iter_mut().find(|t| t.kind == kind) {
                t.status = status;
                t.actions = actions;
            }
        }
        let Some(run) = &mut self.run else {
            return;
        };
        while let Ok(ev) = run.events.try_recv() {
            match ev {
                RunEvent::Started(step) => run.log.push(format!("$ {step}")),
                RunEvent::Output(line) => run.log.push(format!("  {line}")),
                RunEvent::Done { ok: false, .. } => {
                    run.failures += 1;
                    run.log.push("  ✗ başarısız".into());
                }
                RunEvent::Done { ok: true, .. } => run.log.push("  ✓ tamam".into()),
                RunEvent::Finished => {
                    run.finished = true;
                    // Measure again to show what is left.
                    let tool = run.tool;
                    self.remeasure(tool);
                    break;
                }
            }
        }
        if let Some(run) = &mut self.run {
            let excess = run.log.len().saturating_sub(LOG_LINES);
            run.log.drain(..excess);
        }
    }

    fn remeasure(&mut self, index: usize) {
        let kind = self.tools[index].kind;
        self.tools[index].status = Status::Measuring;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (status, actions) = tools::measure(kind);
            let _ = tx.send((kind, status, actions));
        });
        self.measurements = rx;
    }

    pub fn selected(&self) -> Option<&Tool> {
        self.table.selected().and_then(|i| self.tools.get(i))
    }

    pub fn running(&self) -> bool {
        self.run.as_ref().is_some_and(|r| !r.finished)
    }

    pub fn reclaimable(&self) -> u64 {
        self.tools
            .iter()
            .map(|t| match t.status {
                Status::Ready { reclaimable, .. } => reclaimable,
                _ => 0,
            })
            .sum()
    }

    pub fn measuring(&self) -> bool {
        self.tools.iter().any(|t| t.status == Status::Measuring)
    }

    pub fn on_key(&mut self, code: KeyCode) -> ToolsKey {
        // Nothing may interrupt running commands.
        if self.running() {
            return ToolsKey::None;
        }
        if let Some(c) = &mut self.confirm {
            match (&mut c.typed, code) {
                (Some(typed), KeyCode::Char(ch)) => typed.push(ch),
                (Some(typed), KeyCode::Backspace) => {
                    typed.pop();
                }
                (Some(typed), KeyCode::Enter) if typed.trim() == CONFIRM_WORD => self.start(),
                (None, KeyCode::Char('e' | 'E' | 'y' | 'Y')) => self.start(),
                (Some(_), KeyCode::Enter) => {}
                _ => self.confirm = None,
            }
            return ToolsKey::None;
        }
        if let Some(p) = &mut self.picker {
            let n = p.checked.len();
            match code {
                KeyCode::Up | KeyCode::Char('k') => p.cursor = (p.cursor + n - 1) % n,
                KeyCode::Down | KeyCode::Char('j') => p.cursor = (p.cursor + 1) % n,
                KeyCode::Char(' ') => p.checked[p.cursor] = !p.checked[p.cursor],
                KeyCode::Enter => self.ask_confirmation(),
                KeyCode::Char('q') => return ToolsKey::Quit,
                KeyCode::Esc => self.picker = None,
                _ => {}
            }
            return ToolsKey::None;
        }
        match code {
            KeyCode::Char('q') => return ToolsKey::Quit,
            KeyCode::Up | KeyCode::Char('k') => self.table.select_previous(),
            KeyCode::Down | KeyCode::Char('j') => self.table.select_next(),
            KeyCode::Enter => self.open_picker(),
            KeyCode::Char('r') => *self = ToolsView::open(),
            KeyCode::Char('m') => return ToolsKey::Menu,
            // First close the finished log, then the screen.
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Left if self.run.is_some() => {
                self.run = None;
            }
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Left => return ToolsKey::Close,
            _ => {}
        }
        ToolsKey::None
    }

    fn open_picker(&mut self) {
        let Some(i) = self.table.selected() else {
            return;
        };
        let n = self.tools[i].actions.len();
        if n > 0 {
            self.run = None;
            self.picker = Some(Picker {
                tool: i,
                checked: vec![false; n],
                cursor: 0,
            });
        }
    }

    fn ask_confirmation(&mut self) {
        let Some(p) = &self.picker else {
            return;
        };
        let actions: Vec<CleanAction> = self.tools[p.tool]
            .actions
            .iter()
            .zip(&p.checked)
            .filter(|(_, &c)| c)
            .map(|(a, _)| a.clone())
            .collect();
        if actions.is_empty() {
            return;
        }
        let dangerous = actions.iter().any(|a| a.risk == Risk::DataLoss);
        self.confirm = Some(Confirm {
            tool: p.tool,
            actions,
            typed: dangerous.then(String::new),
        });
        self.picker = None;
    }

    fn start(&mut self) {
        let Some(c) = self.confirm.take() else {
            return;
        };
        let steps = c.actions.into_iter().flat_map(|a| a.steps).collect();
        self.run = Some(Run {
            tool: c.tool,
            log: Vec::new(),
            finished: false,
            failures: 0,
            events: tools::run(steps),
        });
    }
}
