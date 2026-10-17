use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::db::{ColumnInfo, DbResult, Driver, IndexInfo, QueryResult, Session, TableKind, TableRef};
use crate::models::connection::DatabaseKind;
use crate::models::plural;

pub const PAGE_SIZE: usize = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FilterOperator {
    #[default]
    Equals,
    NotEquals,
    Greater,
    GreaterOrEqual,
    Less,
    LessOrEqual,
    Contains,
    NotContains,
    Like,
    In,
    IsNull,
    IsNotNull,
}

impl FilterOperator {
    pub const ALL: [FilterOperator; 12] = [
        FilterOperator::Equals,
        FilterOperator::NotEquals,
        FilterOperator::Greater,
        FilterOperator::GreaterOrEqual,
        FilterOperator::Less,
        FilterOperator::LessOrEqual,
        FilterOperator::Contains,
        FilterOperator::NotContains,
        FilterOperator::Like,
        FilterOperator::In,
        FilterOperator::IsNull,
        FilterOperator::IsNotNull,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FilterOperator::Equals => "=",
            FilterOperator::NotEquals => "!=",
            FilterOperator::Greater => ">",
            FilterOperator::GreaterOrEqual => ">=",
            FilterOperator::Less => "<",
            FilterOperator::LessOrEqual => "<=",
            FilterOperator::Contains => "contains",
            FilterOperator::NotContains => "not contains",
            FilterOperator::Like => "LIKE",
            FilterOperator::In => "IN",
            FilterOperator::IsNull => "IS NULL",
            FilterOperator::IsNotNull => "IS NOT NULL",
        }
    }

    pub fn needs_value(self) -> bool {
        !matches!(self, FilterOperator::IsNull | FilterOperator::IsNotNull)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TableFilter {
    /// Empty column means `value` is a raw SQL `WHERE` clause.
    pub column: String,
    pub op: FilterOperator,
    pub value: String,
}

impl TableFilter {
    pub fn is_raw(&self) -> bool {
        self.column.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        self.is_raw() && self.value.trim().is_empty()
    }

    pub fn sql(&self, kind: DatabaseKind, quote: &dyn Fn(&str) -> String) -> Option<String> {
        if self.is_raw() {
            let clause = self.value.trim();
            return (!clause.is_empty()).then(|| clause.to_string());
        }
        let name = kind.quote_identifier(&self.column);
        let text_name = if kind == DatabaseKind::Postgres { format!("{name}::text") } else { name.clone() };
        let like = if kind == DatabaseKind::Postgres { "ILIKE" } else { "LIKE" };
        let value = &self.value;
        Some(match self.op {
            FilterOperator::IsNull | FilterOperator::IsNotNull => format!("{name} {}", self.op.label()),
            FilterOperator::Contains => format!("{text_name} {like} {}", quote(&format!("%{value}%"))),
            FilterOperator::NotContains => format!("{text_name} NOT {like} {}", quote(&format!("%{value}%"))),
            FilterOperator::Like => format!("{text_name} {like} {}", quote(value)),
            FilterOperator::In => {
                let items: Vec<String> = value.split(',').map(|item| quote(item.trim())).collect();
                format!("{name} IN ({})", items.join(", "))
            }
            op => format!("{name} {} {}", op.label(), quote(value)),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableMode {
    #[default]
    Data,
    Structure,
}

/// (row, column) of a grid cell.
pub type CellKey = (usize, usize);

/// A table opened in a tab: one page of its rows plus the edits staged on
/// them, and its structure.
#[derive(Debug)]
pub struct TableState {
    pub table: TableRef,
    pub kind: DatabaseKind,
    pub mode: TableMode,

    pub columns: Vec<ColumnInfo>,
    pub indexes: Vec<IndexInfo>,
    pub result_columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    pub loaded_row_count: usize,
    pub page: usize,
    pub sort_column: Option<String>,
    pub sort_ascending: bool,
    pub applied_filter: TableFilter,
    pub filter: TableFilter,

    pub is_loading: bool,
    pub duration: Duration,
    pub error: Option<String>,

    /// Staged values; `None` inside means NULL.
    pub edits: HashMap<CellKey, Option<String>>,
    pub deleted_rows: BTreeSet<usize>,
    /// Bumped when a new page of rows (or a new structure) arrives.
    pub page_version: u64,
    pub structure_version: u64,
}

impl TableState {
    pub fn new(table: TableRef, kind: DatabaseKind) -> Self {
        Self {
            table,
            kind,
            mode: TableMode::Data,
            columns: Vec::new(),
            indexes: Vec::new(),
            result_columns: Vec::new(),
            rows: Vec::new(),
            loaded_row_count: 0,
            page: 0,
            sort_column: None,
            sort_ascending: true,
            applied_filter: TableFilter::default(),
            filter: TableFilter::default(),
            is_loading: false,
            duration: Duration::ZERO,
            error: None,
            edits: HashMap::new(),
            deleted_rows: BTreeSet::new(),
            page_version: 0,
            structure_version: 0,
        }
    }

    pub fn primary_key(&self) -> Vec<String> {
        self.columns.iter().filter(|c| c.is_primary_key).map(|c| c.name.clone()).collect()
    }

    pub fn can_edit(&self) -> bool {
        self.table.kind == TableKind::Table && self.columns.iter().any(|c| c.is_primary_key)
    }

    pub fn inserted_row_count(&self) -> usize {
        self.rows.len() - self.loaded_row_count
    }

    pub fn has_changes(&self) -> bool {
        !self.edits.is_empty() || !self.deleted_rows.is_empty() || self.inserted_row_count() > 0
    }

    pub fn change_count(&self) -> usize {
        let mut rows: BTreeSet<usize> = self.edits.keys().map(|&(row, _)| row).collect();
        rows.extend(&self.deleted_rows);
        rows.extend(self.loaded_row_count..self.rows.len());
        rows.len()
    }

    pub fn has_next_page(&self) -> bool {
        self.loaded_row_count == PAGE_SIZE
    }

    pub fn first_row_number(&self) -> usize {
        self.page * PAGE_SIZE + 1
    }

    pub fn edit_disabled_reason(&self) -> Option<&'static str> {
        if self.table.kind == TableKind::View {
            return Some("Views are read-only");
        }
        if !self.columns.is_empty() && !self.columns.iter().any(|c| c.is_primary_key) {
            return Some("Editing needs a primary key");
        }
        None
    }

    pub fn status_text(&self) -> String {
        let duration = super::format_duration(self.duration);
        if self.loaded_row_count == 0 {
            return format!("No rows · {duration}");
        }
        let first = self.first_row_number();
        let last = first + self.loaded_row_count - 1;
        format!("Rows {first}–{last} · {duration}")
    }

    pub fn changes_text(&self) -> String {
        format!("{} pending", plural(self.change_count(), "change"))
    }

    // Loading

    fn select_query(&self) -> SelectQuery {
        SelectQuery {
            name: self.kind.qualified_name(&self.table),
            kind: self.kind,
            filter: self.applied_filter.clone(),
            order: self.sort_column.as_ref().map(|column| {
                format!("{} {}", self.kind.quote_identifier(column), if self.sort_ascending { "ASC" } else { "DESC" })
            }),
            offset: self.page * PAGE_SIZE,
        }
    }

    fn apply_page(&mut self, result: DbResult<QueryResult>, duration: Duration) {
        self.page_version += 1;
        match result {
            Ok(result) => {
                self.duration = duration;
                self.result_columns = result.columns;
                self.rows = result.rows;
                self.loaded_row_count = self.rows.len();
                self.error = None;
            }
            Err(error) => {
                self.rows.clear();
                self.loaded_row_count = 0;
                self.error = Some(error.message);
            }
        }
        self.reset_edits();
    }

    /// Cycles ascending → descending → unsorted for a result column.
    pub fn toggle_sort(&mut self, column_index: usize) -> bool {
        let Some(name) = self.result_columns.get(column_index).cloned() else { return false };
        if self.sort_column.as_ref() == Some(&name) {
            if self.sort_ascending {
                self.sort_ascending = false;
            } else {
                self.sort_column = None;
            }
        } else {
            self.sort_column = Some(name);
            self.sort_ascending = true;
        }
        self.page = 0;
        true
    }

    // Editing

    pub fn value(&self, row: usize, column: usize) -> Option<&str> {
        match self.edits.get(&(row, column)) {
            Some(edited) => edited.as_deref(),
            None => self.rows.get(row).and_then(|r| r.get(column)).and_then(|v| v.as_deref()),
        }
    }

    pub fn is_edited(&self, row: usize, column: usize) -> bool {
        self.edits.contains_key(&(row, column))
    }

    pub fn is_inserted(&self, row: usize) -> bool {
        row >= self.loaded_row_count
    }

    pub fn set_value(&mut self, value: Option<String>, row: usize, column: usize) {
        if !self.can_edit() || row >= self.rows.len() || self.deleted_rows.contains(&row) {
            return;
        }
        if !self.is_inserted(row) && value == self.rows[row][column] {
            self.edits.remove(&(row, column));
        } else {
            self.edits.insert((row, column), value);
        }
    }

    /// Appends an empty row and returns its index.
    pub fn add_row(&mut self) -> Option<usize> {
        if !self.can_edit() {
            return None;
        }
        self.rows.push(vec![None; self.result_columns.len()]);
        Some(self.rows.len() - 1)
    }

    pub fn delete_rows(&mut self, rows: &BTreeSet<usize>) {
        if !self.can_edit() {
            return;
        }
        // Inserted rows are simply dropped; their edits shift down.
        for &row in rows.iter().rev() {
            if self.is_inserted(row) && row < self.rows.len() {
                self.rows.remove(row);
                self.edits = std::mem::take(&mut self.edits)
                    .into_iter()
                    .filter(|&((r, _), _)| r != row)
                    .map(|((r, c), value)| ((if r > row { r - 1 } else { r }, c), value))
                    .collect();
            }
        }
        for &row in rows.iter().filter(|&&row| row < self.loaded_row_count) {
            self.deleted_rows.insert(row);
            self.edits.retain(|&(r, _), _| r != row);
        }
    }

    pub fn discard_changes(&mut self) {
        self.rows.truncate(self.loaded_row_count);
        self.reset_edits();
        self.error = None;
    }

    fn reset_edits(&mut self) {
        self.edits.clear();
        self.deleted_rows.clear();
    }

    /// Groups the staged changes into deletes, updates and inserts.
    fn save_plan(&self) -> Result<SavePlan, String> {
        let primary_key = self.primary_key();
        let pk_indexes: Vec<usize> =
            primary_key.iter().filter_map(|name| self.result_columns.iter().position(|c| c == name)).collect();
        if pk_indexes.len() != primary_key.len() {
            return Err("Primary key columns are missing from the result".into());
        }
        let mut edited_rows: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for &(row, column) in self.edits.keys() {
            edited_rows.entry(row).or_default().push(column);
        }

        let mut plan = SavePlan {
            kind: self.kind,
            name: self.kind.qualified_name(&self.table),
            primary_key,
            ..SavePlan::default()
        };
        for (row, values) in self.rows.iter().enumerate() {
            let key = || pk_indexes.iter().map(|&i| values[i].clone()).collect::<Vec<_>>();
            if self.deleted_rows.contains(&row) {
                plan.deletes.push(key());
                continue;
            }
            let mut columns = edited_rows.get(&row).cloned().unwrap_or_default();
            columns.sort_unstable();
            let assignments: Vec<(String, Option<String>)> = columns
                .into_iter()
                .map(|column| (self.result_columns[column].clone(), self.edits[&(row, column)].clone()))
                .collect();
            if self.is_inserted(row) {
                plan.inserts.push(assignments);
            } else if !assignments.is_empty() {
                plan.updates.push((key(), assignments));
            }
        }
        Ok(plan)
    }
}

struct SelectQuery {
    name: String,
    kind: DatabaseKind,
    filter: TableFilter,
    order: Option<String>,
    offset: usize,
}

impl SelectQuery {
    fn sql(&self, driver: &dyn Driver) -> String {
        let mut sql = format!("SELECT * FROM {}", self.name);
        if let Some(clause) = self.filter.sql(self.kind, &|value| driver.quote_literal(value)) {
            sql += &format!(" WHERE {clause}");
        }
        if let Some(order) = &self.order {
            sql += &format!(" ORDER BY {order}");
        }
        sql += &format!(" LIMIT {PAGE_SIZE} OFFSET {}", self.offset);
        sql
    }
}

type Assignment = (String, Option<String>);

#[derive(Default)]
struct SavePlan {
    kind: DatabaseKind,
    name: String,
    primary_key: Vec<String>,
    deletes: Vec<Vec<Option<String>>>,
    updates: Vec<(Vec<Option<String>>, Vec<Assignment>)>,
    inserts: Vec<Vec<Assignment>>,
}

impl SavePlan {
    fn statements(&self, driver: &dyn Driver) -> Vec<String> {
        let kind = self.kind;
        let name = &self.name;
        let literal = |value: &Option<String>| value.as_deref().map_or("NULL".to_string(), |v| driver.quote_literal(v));
        let condition = |key: &[Option<String>]| {
            self.primary_key
                .iter()
                .zip(key)
                .map(|(column, value)| {
                    let column = kind.quote_identifier(column);
                    match value {
                        None => format!("{column} IS NULL"),
                        Some(_) => format!("{column} = {}", literal(value)),
                    }
                })
                .collect::<Vec<_>>()
                .join(" AND ")
        };

        let mut statements = Vec::new();
        for key in &self.deletes {
            statements.push(format!("DELETE FROM {name} WHERE {}", condition(key)));
        }
        for (key, values) in &self.updates {
            let set = values
                .iter()
                .map(|(column, value)| format!("{} = {}", kind.quote_identifier(column), literal(value)))
                .collect::<Vec<_>>()
                .join(", ");
            statements.push(format!("UPDATE {name} SET {set} WHERE {}", condition(key)));
        }
        for values in &self.inserts {
            if values.is_empty() {
                statements.push(match kind {
                    DatabaseKind::Postgres => format!("INSERT INTO {name} DEFAULT VALUES"),
                    DatabaseKind::MySql => format!("INSERT INTO {name} () VALUES ()"),
                });
            } else {
                let columns = values.iter().map(|(c, _)| kind.quote_identifier(c)).collect::<Vec<_>>().join(", ");
                let literals = values.iter().map(|(_, v)| literal(v)).collect::<Vec<_>>().join(", ");
                statements.push(format!("INSERT INTO {name} ({columns}) VALUES ({literals})"));
            }
        }
        statements
    }
}

/// `TableState` plus the session it loads from. Async methods never hold
/// the state borrowed across an `.await`; `on_change` runs after every
/// state change so the UI can redraw.
pub struct TableModel {
    pub state: RefCell<TableState>,
    pub session: Arc<Session>,
    on_change: RefCell<Option<Box<dyn Fn()>>>,
}

impl TableModel {
    pub fn new(table: TableRef, session: Arc<Session>) -> Self {
        let kind = session.kind();
        Self { state: RefCell::new(TableState::new(table, kind)), session, on_change: RefCell::new(None) }
    }

    pub fn set_on_change(&self, callback: impl Fn() + 'static) {
        *self.on_change.borrow_mut() = Some(Box::new(callback));
    }

    pub fn notify(&self) {
        if let Some(callback) = self.on_change.borrow().as_ref() {
            callback();
        }
    }

    fn update(&self, change: impl FnOnce(&mut TableState)) {
        change(&mut self.state.borrow_mut());
        self.notify();
    }

    pub async fn load(&self) {
        self.update(|s| s.is_loading = true);
        if let Err(error) = self.fetch_structure().await {
            self.update(|s| s.error = Some(error.message));
        }
        self.reload().await;
    }

    async fn fetch_structure(&self) -> DbResult<()> {
        let table = self.state.borrow().table.clone();
        let (columns, indexes) =
            self.session.run(move |driver| Ok((driver.columns(&table)?, driver.indexes(&table)?))).await?;
        self.update(|s| {
            s.columns = columns;
            s.indexes = indexes;
            s.structure_version += 1;
        });
        Ok(())
    }

    pub async fn load_structure(&self) {
        let _ = self.fetch_structure().await;
    }

    pub async fn reload(&self) {
        self.update(|s| s.is_loading = true);
        let query = self.state.borrow().select_query();
        let started = Instant::now();
        let result = self
            .session
            .run(move |driver| {
                let sql = query.sql(driver);
                driver.query(&sql)
            })
            .await;
        let elapsed = started.elapsed();
        self.update(|s| {
            s.apply_page(result, elapsed);
            s.is_loading = false;
        });
    }

    pub async fn apply_filter(&self) {
        self.update(|s| {
            s.applied_filter = s.filter.clone();
            s.page = 0;
        });
        self.reload().await;
    }

    pub async fn clear_filter(&self) {
        self.update(|s| {
            s.filter = TableFilter { column: s.filter.column.clone(), op: s.filter.op, value: String::new() };
        });
        self.apply_filter().await;
    }

    pub async fn sort(&self, column_index: usize) {
        let changed = self.state.borrow_mut().toggle_sort(column_index);
        if changed {
            self.notify();
            self.reload().await;
        }
    }

    pub async fn go_to_page(&self, page: usize) {
        self.update(|s| s.page = page);
        self.reload().await;
    }

    pub fn set_value(&self, value: Option<String>, row: usize, column: usize) {
        self.update(|s| s.set_value(value, row, column));
    }

    pub fn add_row(&self) -> Option<usize> {
        let row = self.state.borrow_mut().add_row();
        self.notify();
        row
    }

    pub fn delete_rows(&self, rows: &BTreeSet<usize>) {
        self.update(|s| s.delete_rows(rows));
    }

    pub fn discard_changes(&self) {
        self.update(TableState::discard_changes);
    }

    /// Writes all staged edits, inserts and deletes in one transaction.
    pub async fn save_changes(&self) {
        let plan = {
            let state = self.state.borrow();
            if !state.has_changes() {
                return;
            }
            state.save_plan()
        };
        let plan = match plan {
            Ok(plan) => plan,
            Err(message) => return self.update(|s| s.error = Some(message)),
        };
        self.update(|s| s.is_loading = true);
        let result = self
            .session
            .run(move |driver| {
                let statements = plan.statements(driver);
                driver.execute(if plan.kind == DatabaseKind::MySql { "START TRANSACTION" } else { "BEGIN" })?;
                let outcome = statements.iter().try_for_each(|statement| driver.execute(statement).map(drop));
                match outcome {
                    Ok(()) => driver.execute("COMMIT").map(drop),
                    Err(error) => {
                        let _ = driver.execute("ROLLBACK");
                        Err(error)
                    }
                }
            })
            .await;
        match result {
            Ok(()) => self.reload().await,
            Err(error) => self.update(|s| {
                s.is_loading = false;
                s.error = Some(error.message);
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }

    #[test]
    fn builds_filter_clauses() {
        let filter = |column: &str, op, value: &str| TableFilter { column: column.into(), op, value: value.into() };
        let pg = DatabaseKind::Postgres;
        let my = DatabaseKind::MySql;
        assert_eq!(filter("", FilterOperator::Equals, "  ").sql(my, &quote), None);
        assert_eq!(filter("", FilterOperator::Equals, " id > 1 ").sql(my, &quote).unwrap(), "id > 1");
        assert_eq!(
            filter("name", FilterOperator::Contains, "a").sql(pg, &quote).unwrap(),
            "\"name\"::text ILIKE '%a%'"
        );
        assert_eq!(filter("name", FilterOperator::Contains, "a").sql(my, &quote).unwrap(), "`name` LIKE '%a%'");
        assert_eq!(filter("id", FilterOperator::In, "1, 2").sql(my, &quote).unwrap(), "`id` IN ('1', '2')");
        assert_eq!(filter("id", FilterOperator::IsNull, "x").sql(my, &quote).unwrap(), "`id` IS NULL");
        assert_eq!(filter("id", FilterOperator::GreaterOrEqual, "3").sql(pg, &quote).unwrap(), "\"id\" >= '3'");
    }

    fn sample() -> TableState {
        let mut state = TableState::new(TableRef::new(None, "items"), DatabaseKind::MySql);
        let column = |name: &str, pk| ColumnInfo {
            name: name.into(),
            data_type: "text".into(),
            nullable: true,
            default_value: None,
            is_primary_key: pk,
            extra: String::new(),
            comment: String::new(),
        };
        state.columns = vec![column("id", true), column("name", false)];
        state.result_columns = vec!["id".into(), "name".into()];
        state.rows = vec![vec![Some("1".into()), Some("a".into())], vec![Some("2".into()), None]];
        state.loaded_row_count = 2;
        state
    }

    #[test]
    fn stages_edits_and_counts_changes() {
        let mut state = sample();
        state.set_value(Some("a".into()), 0, 1);
        assert!(!state.has_changes(), "same value is not an edit");
        state.set_value(Some("b".into()), 0, 1);
        state.set_value(None, 0, 1);
        assert_eq!(state.value(0, 1), None);
        assert!(state.is_edited(0, 1));
        let row = state.add_row().unwrap();
        state.set_value(Some("c".into()), row, 1);
        state.delete_rows(&BTreeSet::from([1]));
        assert_eq!(state.change_count(), 3);

        state.delete_rows(&BTreeSet::from([row]));
        assert_eq!(state.rows.len(), 2);
        assert_eq!(state.change_count(), 2);
        state.discard_changes();
        assert!(!state.has_changes());
    }

    #[test]
    fn sort_cycles_through_three_states() {
        let mut state = sample();
        state.toggle_sort(1);
        assert_eq!((state.sort_column.as_deref(), state.sort_ascending), (Some("name"), true));
        state.toggle_sort(1);
        assert_eq!((state.sort_column.as_deref(), state.sort_ascending), (Some("name"), false));
        state.toggle_sort(1);
        assert_eq!(state.sort_column, None);
    }
}
