//! The launcher session: which mode the panel is in, the results it shows and which one is
//! selected. The UI forwards query edits and key intents; the session answers with effects
//! (start a search, run an action, hide) for the UI and platform layers to carry out.

use crate::searcher::Delivery;
use crate::{Action, Item};

/// Which provider the query goes to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Fuzzy search over installed apps.
    #[default]
    Apps,
    /// Spotlight file search, entered with Space on an empty query (as Alfred does).
    Files,
}

/// Keys the session gives meaning to. The UI maps raw keys onto these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Space typed without modifiers.
    Space,
    Backspace,
    Up,
    Down,
    /// Enter.
    Open,
    /// ⌘Enter.
    OpenAlternate,
    /// Esc.
    Dismiss,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    Search {
        mode: Mode,
        generation: u64,
        query: String,
    },
    /// Stop whatever search is running; nothing should arrive for the current results.
    CancelSearch,
    Run(Action),
    /// Hide the panel. The session has already reset itself for the next show.
    Hide,
    /// The selection moved; keep the selected row on screen.
    RevealSelection,
}

/// Where the results on screen stand relative to the current query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Results {
    /// Nothing to search for (an empty query).
    Idle,
    /// A search is running; the rows shown, if any, belong to the previous query until its
    /// first batch arrives, so typing doesn't flash an empty list.
    Searching { replaced: bool },
    /// The search finished; the rows are all there is.
    Done,
}

#[derive(Debug)]
pub struct Session {
    mode: Mode,
    query: String,
    items: Vec<Item>,
    selected: usize,
    generation: u64,
    results: Results,
}

impl Default for Session {
    fn default() -> Self {
        Session {
            mode: Mode::Apps,
            query: String::new(),
            items: Vec::new(),
            selected: 0,
            generation: 0,
            results: Results::Idle,
        }
    }
}

impl Session {
    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn selected_item(&self) -> Option<&Item> {
        self.items.get(self.selected)
    }

    pub fn results(&self) -> Results {
        self.results
    }

    /// Select the result with this id, as a click on its row does. False when it's gone.
    pub fn select_id(&mut self, id: &str) -> bool {
        let Some(i) = self.items.iter().position(|item| item.id == id) else {
            return false;
        };
        self.selected = i;
        true
    }

    /// The text field's contents changed.
    pub fn set_query(&mut self, text: &str) -> Vec<Effect> {
        if text == self.query {
            return Vec::new();
        }
        self.query = text.to_owned();
        self.search()
    }

    /// A key the UI recognised. None leaves the key to the text field (typing a space into a
    /// query, deleting a character); Some consumes it, even when nothing else happens.
    pub fn intent(&mut self, intent: Intent) -> Option<Vec<Effect>> {
        let empty = self.query.is_empty();
        Some(match intent {
            Intent::Space if empty => match self.mode {
                Mode::Apps => self.switch(Mode::Files),
                // A leading space finds nothing in either mode; swallow it.
                Mode::Files => Vec::new(),
            },
            Intent::Backspace if empty => match self.mode {
                Mode::Files => self.switch(Mode::Apps),
                Mode::Apps => return None,
            },
            Intent::Space | Intent::Backspace => return None,
            Intent::Up | Intent::Down => {
                let n = self.items.len();
                if n == 0 {
                    return Some(Vec::new());
                }
                self.selected = if intent == Intent::Down {
                    (self.selected + 1) % n
                } else {
                    (self.selected + n - 1) % n
                };
                vec![Effect::RevealSelection]
            }
            Intent::Open | Intent::OpenAlternate => {
                let Some(item) = self.selected_item() else {
                    return Some(Vec::new());
                };
                let action = if intent == Intent::Open {
                    Some(item.primary.clone())
                } else {
                    item.alternate.clone()
                };
                let Some(action) = action else {
                    return Some(Vec::new());
                };
                let mut effects = vec![Effect::Run(action)];
                effects.extend(self.dismiss());
                effects
            }
            Intent::Dismiss => self.dismiss(),
        })
    }

    /// A provider's answer. Returns whether anything visible changed.
    pub fn deliver(&mut self, delivery: Delivery) -> bool {
        if delivery.generation() != self.generation {
            return false;
        }
        match (delivery, self.results) {
            (Delivery::Batch { items, .. }, Results::Searching { replaced: false }) => {
                self.items = items;
                self.selected = 0;
                self.results = Results::Searching { replaced: true };
            }
            (Delivery::Batch { items, .. }, Results::Searching { replaced: true }) => {
                self.items.extend(items)
            }
            (Delivery::Done { .. }, Results::Searching { replaced }) => {
                if !replaced {
                    self.items.clear();
                    self.selected = 0;
                }
                self.results = Results::Done;
            }
            // Late answers for a search that already finished or was abandoned.
            (_, Results::Idle | Results::Done) => return false,
        }
        true
    }

    /// Back to an empty apps search, as the panel should look the next time it shows.
    pub fn reset(&mut self) -> Vec<Effect> {
        let was_searching = matches!(self.results, Results::Searching { .. });
        self.mode = Mode::Apps;
        self.query.clear();
        self.clear_results();
        if was_searching {
            vec![Effect::CancelSearch]
        } else {
            Vec::new()
        }
    }

    fn dismiss(&mut self) -> Vec<Effect> {
        let mut effects = self.reset();
        effects.push(Effect::Hide);
        effects
    }

    fn switch(&mut self, mode: Mode) -> Vec<Effect> {
        self.mode = mode;
        self.search()
    }

    fn search(&mut self) -> Vec<Effect> {
        let query = self.query.trim();
        if query.is_empty() {
            let was_searching = matches!(self.results, Results::Searching { .. });
            self.clear_results();
            return if was_searching {
                vec![Effect::CancelSearch]
            } else {
                Vec::new()
            };
        }
        let query = query.to_owned();
        self.generation += 1;
        // A mode switch shows nothing of the other mode's rows.
        let keep_rows = self.items.first().is_some_and(|i| self.mode == mode_of(i));
        if !keep_rows {
            self.items.clear();
            self.selected = 0;
        }
        self.results = Results::Searching { replaced: false };
        vec![Effect::Search {
            mode: self.mode,
            generation: self.generation,
            query,
        }]
    }

    fn clear_results(&mut self) {
        // Bump the generation so answers still in flight are recognised as stale.
        self.generation += 1;
        self.items.clear();
        self.selected = 0;
        self.results = Results::Idle;
    }
}

fn mode_of(item: &Item) -> Mode {
    match item.kind {
        crate::ItemKind::Application => Mode::Apps,
        crate::ItemKind::File => Mode::Files,
    }
}

#[cfg(test)]
mod tests;
