use std::cell::RefCell;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::db::{QueryResult, Session};
use crate::models::{format_duration, plural};

#[derive(Debug, Default)]
pub struct QueryState {
    pub result: Option<QueryResult>,
    pub message: Option<String>,
    pub error: Option<String>,
    pub is_running: bool,
    /// Bumped whenever `result` is replaced.
    pub result_version: u64,
}

/// A query tab: runs SQL from the editor and keeps the last result.
pub struct QueryModel {
    pub title: String,
    pub state: RefCell<QueryState>,
    pub session: Arc<Session>,
    on_change: RefCell<Option<Box<dyn Fn()>>>,
}

impl QueryModel {
    pub fn new(title: String, session: Arc<Session>) -> Self {
        Self { title, state: RefCell::default(), session, on_change: RefCell::new(None) }
    }

    pub fn set_on_change(&self, callback: impl Fn() + 'static) {
        *self.on_change.borrow_mut() = Some(Box::new(callback));
    }

    fn notify(&self) {
        if let Some(callback) = self.on_change.borrow().as_ref() {
            callback();
        }
    }

    pub fn is_running(&self) -> bool {
        self.state.borrow().is_running
    }

    /// Runs `sql` (the selection, or the whole editor text).
    pub async fn run(&self, sql: &str) {
        let text = sql.trim().to_string();
        if text.is_empty() || self.is_running() {
            return;
        }
        {
            let mut state = self.state.borrow_mut();
            state.is_running = true;
            state.error = None;
            state.message = None;
        }
        self.notify();

        let started = Instant::now();
        let outcome = self.session.run(move |driver| driver.execute(&text)).await;
        let elapsed = started.elapsed();
        {
            let mut state = self.state.borrow_mut();
            state.is_running = false;
            state.result_version += 1;
            match outcome {
                Ok(mut results) => {
                    state.message = Some(summary(&results, elapsed));
                    let shown = results.iter().rposition(QueryResult::has_result_set);
                    state.result = shown.map(|index| results.swap_remove(index));
                }
                Err(error) => {
                    state.result = None;
                    state.error = Some(error.message);
                }
            }
        }
        self.notify();
    }

    pub fn cancel(&self) {
        if self.is_running() {
            self.session.cancel();
        }
    }
}

pub fn summary(results: &[QueryResult], elapsed: Duration) -> String {
    let mut parts = Vec::new();
    match results.iter().rev().find(|r| r.has_result_set()) {
        Some(shown) => {
            parts.push(plural(shown.rows.len(), "row"));
            if shown.truncated {
                parts.push(format!("showing first {}", shown.rows.len()));
            }
        }
        None => {
            let affected: u64 = results.iter().filter_map(|r| r.affected_rows).sum();
            parts.push(format!("Query OK, {} affected", plural(affected as usize, "row")));
        }
    }
    if results.len() > 1 {
        parts.push(format!("{} statements", results.len()));
    }
    parts.push(format_duration(elapsed));
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarises_results() {
        let rows = QueryResult { columns: vec!["a".into()], rows: vec![vec![None]], ..Default::default() };
        let update = QueryResult { affected_rows: Some(2), ..Default::default() };
        assert_eq!(summary(&[update.clone(), rows], Duration::from_millis(12)), "1 row · 2 statements · 12 ms");
        assert_eq!(summary(&[update], Duration::from_millis(1500)), "Query OK, 2 rows affected · 1.50 s");
    }
}
