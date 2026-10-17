use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::glib;

use super::grid::{DataGrid, GridCell, GridSource, RowState, StaticSource};
use super::widgets::{ErrorBanner, export_csv, icon_button};
use crate::db::{QueryResult, Session, TableRef};
use crate::models::table::{FilterOperator, TableMode, TableModel};

struct TableSource(Rc<TableModel>);

impl GridSource for TableSource {
    fn columns(&self) -> Vec<String> {
        self.0.state.borrow().result_columns.clone()
    }

    fn row_count(&self) -> usize {
        self.0.state.borrow().rows.len()
    }

    fn cell(&self, row: usize, column: usize) -> GridCell {
        let state = self.0.state.borrow();
        let inserted = state.is_inserted(row);
        let edited = state.is_edited(row, column);
        GridCell {
            value: state.value(row, column).map(str::to_string),
            placeholder: (inserted && !edited).then_some("DEFAULT"),
            edited: edited && !inserted,
        }
    }

    fn row_state(&self, row: usize) -> RowState {
        let state = self.0.state.borrow();
        if state.deleted_rows.contains(&row) {
            RowState::Deleted
        } else if state.is_inserted(row) {
            RowState::Inserted
        } else {
            RowState::Normal
        }
    }

    fn editable(&self) -> bool {
        self.0.state.borrow().can_edit()
    }

    fn sort(&self) -> Option<(String, bool)> {
        let state = self.0.state.borrow();
        state.sort_column.clone().map(|column| (column, state.sort_ascending))
    }
}

struct FilterBar {
    root: gtk::Box,
    columns: gtk::StringList,
    column: gtk::DropDown,
    operator: gtk::DropDown,
    value: gtk::Entry,
    apply: gtk::Button,
    clear: gtk::Button,
}

impl FilterBar {
    fn new() -> Self {
        let columns = gtk::StringList::new(&["Raw SQL"]);
        let column = gtk::DropDown::builder().model(&columns).tooltip_text("Column").build();
        column.set_size_request(150, -1);
        let labels: Vec<&str> = FilterOperator::ALL.iter().map(|op| op.label()).collect();
        let operator = gtk::DropDown::from_strings(&labels);
        operator.set_size_request(110, -1);
        let value = gtk::Entry::builder().hexpand(true).build();
        let apply = gtk::Button::with_label("Apply");
        let clear = gtk::Button::with_label("Clear");
        let icon = gtk::Image::from_icon_name("edit-find-symbolic");
        icon.add_css_class("dim-label");
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        root.set_margin_top(6);
        root.set_margin_bottom(6);
        root.set_margin_start(8);
        root.set_margin_end(8);
        root.append(&icon);
        root.append(&column);
        root.append(&operator);
        root.append(&value);
        root.append(&apply);
        root.append(&clear);
        Self { root, columns, column, operator, value, apply, clear }
    }
}

pub struct TableTab {
    pub model: Rc<TableModel>,
    pub root: gtk::Box,
    stack: gtk::Stack,
    filter: FilterBar,
    grid: DataGrid,
    loading: gtk::Spinner,
    columns_grid: DataGrid,
    indexes_grid: DataGrid,
    error: Rc<ErrorBanner>,
    data_toggle: gtk::ToggleButton,
    structure_toggle: gtk::ToggleButton,
    data_controls: gtk::Box,
    add_button: gtk::Button,
    delete_button: gtk::Button,
    lock: gtk::Box,
    lock_label: gtk::Label,
    status: gtk::Label,
    changes: gtk::Label,
    discard_button: gtk::Button,
    save_button: gtk::Button,
    previous_button: gtk::Button,
    page_label: gtk::Label,
    next_button: gtk::Button,
    export_button: gtk::Button,
    reload_button: gtk::Button,

    shown_page: Cell<u64>,
    shown_structure: Cell<u64>,
    filter_columns: RefCell<Vec<String>>,
    /// True while widgets are being synced from the model.
    syncing: Cell<bool>,
    on_state_change: RefCell<Option<Box<dyn Fn()>>>,
}

impl TableTab {
    pub fn new(table: TableRef, session: Arc<Session>) -> Rc<Self> {
        let model = Rc::new(TableModel::new(table, session));

        let filter = FilterBar::new();
        let grid = DataGrid::new(Rc::new(TableSource(model.clone())), true);
        let loading = gtk::Spinner::builder()
            .spinning(true)
            .visible(false)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Start)
            .margin_top(36)
            .width_request(24)
            .height_request(24)
            .build();
        let overlay = gtk::Overlay::builder().child(grid.widget()).build();
        overlay.add_overlay(&loading);
        let data_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        data_page.append(&filter.root);
        data_page.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        data_page.append(&overlay);

        let empty = || Rc::new(StaticSource { columns: Vec::new(), rows: Vec::new() });
        let columns_grid = DataGrid::new(empty(), false);
        let indexes_grid = DataGrid::new(empty(), false);
        let indexes_title =
            gtk::Label::builder().label("Indexes").xalign(0.0).margin_start(10).margin_top(6).margin_bottom(6).build();
        indexes_title.add_css_class("heading");
        let indexes_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        indexes_box.append(&indexes_title);
        indexes_box.append(indexes_grid.widget());
        columns_grid.widget().set_size_request(-1, 160);
        indexes_box.set_size_request(-1, 120);
        let structure_page = gtk::Paned::builder()
            .orientation(gtk::Orientation::Vertical)
            .start_child(columns_grid.widget())
            .end_child(&indexes_box)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .position(360)
            .build();

        let stack = gtk::Stack::new();
        stack.add_named(&data_page, Some("data"));
        stack.add_named(&structure_page, Some("structure"));
        stack.set_vexpand(true);

        let weak_model = Rc::downgrade(&model);
        let error = ErrorBanner::new(move || {
            if let Some(model) = weak_model.upgrade() {
                model.state.borrow_mut().error = None;
                model.notify();
            }
        });

        let data_toggle = gtk::ToggleButton::builder().label("Data").active(true).build();
        let structure_toggle = gtk::ToggleButton::builder().label("Structure").group(&data_toggle).build();
        let modes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        modes.add_css_class("linked");
        modes.append(&data_toggle);
        modes.append(&structure_toggle);

        let add_button = icon_button("list-add-symbolic", "Add row (Ctrl+I)");
        let delete_button = icon_button("list-remove-symbolic", "Delete selected rows (Delete)");
        let lock_label = gtk::Label::new(None);
        lock_label.add_css_class("caption");
        let lock = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        lock.add_css_class("dim-label");
        lock.append(&gtk::Image::from_icon_name("changes-prevent-symbolic"));
        lock.append(&lock_label);
        let data_controls = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        data_controls.append(&add_button);
        data_controls.append(&delete_button);
        data_controls.append(&lock);

        let status = gtk::Label::new(None);
        status.add_css_class("dim-label");
        status.add_css_class("caption");
        status.add_css_class("numeric");
        let changes = gtk::Label::new(None);
        changes.add_css_class("caption");
        changes.add_css_class("warning");
        let discard_button = gtk::Button::with_label("Discard");
        let save_button = gtk::Button::builder().label("Save").tooltip_text("Commit changes (Ctrl+S)").build();
        save_button.add_css_class("suggested-action");
        let previous_button = icon_button("go-previous-symbolic", "Previous page");
        let page_label = gtk::Label::new(Some("1"));
        page_label.add_css_class("caption");
        page_label.add_css_class("numeric");
        page_label.set_width_chars(3);
        let next_button = icon_button("go-next-symbolic", "Next page");
        let export_button = icon_button("document-save-as-symbolic", "Export this page as CSV");
        let reload_button = icon_button("view-refresh-symbolic", "Reload (Ctrl+R)");
        let spacer = gtk::Box::builder().hexpand(true).build();

        let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bottom.add_css_class("bottom-bar");
        bottom.append(&modes);
        bottom.append(&data_controls);
        bottom.append(&spacer);
        bottom.append(&changes);
        bottom.append(&discard_button);
        bottom.append(&save_button);
        bottom.append(&status);
        bottom.append(&previous_button);
        bottom.append(&page_label);
        bottom.append(&next_button);
        bottom.append(&export_button);
        bottom.append(&reload_button);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&stack);
        root.append(&error.root);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&bottom);

        let tab = Rc::new(Self {
            model,
            root,
            stack,
            filter,
            grid,
            loading,
            columns_grid,
            indexes_grid,
            error,
            data_toggle,
            structure_toggle,
            data_controls,
            add_button,
            delete_button,
            lock,
            lock_label,
            status,
            changes,
            discard_button,
            save_button,
            previous_button,
            page_label,
            next_button,
            export_button,
            reload_button,
            shown_page: Cell::new(0),
            shown_structure: Cell::new(0),
            filter_columns: RefCell::default(),
            syncing: Cell::new(false),
            on_state_change: RefCell::new(None),
        });
        Self::connect(&tab);
        tab.refresh();
        tab.spawn(|model| async move { model.load().await });
        tab
    }

    pub fn table(&self) -> TableRef {
        self.model.state.borrow().table.clone()
    }

    pub fn has_changes(&self) -> bool {
        self.model.state.borrow().has_changes()
    }

    pub fn can_edit(&self) -> bool {
        self.model.state.borrow().can_edit()
    }

    pub fn set_on_state_change(&self, f: impl Fn() + 'static) {
        *self.on_state_change.borrow_mut() = Some(Box::new(f));
    }

    pub fn focus(&self) {
        self.grid.grab_focus();
    }

    fn spawn<F, Fut>(&self, work: F)
    where
        F: FnOnce(Rc<TableModel>) -> Fut,
        Fut: Future<Output = ()> + 'static,
    {
        glib::spawn_future_local(work(self.model.clone()));
    }

    pub fn reload(&self) {
        if self.has_changes() {
            return;
        }
        let structure = self.model.state.borrow().mode == TableMode::Structure;
        self.spawn(
            move |model| async move { if structure { model.load_structure().await } else { model.reload().await } },
        );
    }

    pub fn save(&self) {
        self.spawn(|model| async move { model.save_changes().await });
    }

    pub fn discard(&self) {
        self.model.discard_changes();
    }

    pub fn add_row(&self) {
        if let Some(row) = self.model.add_row() {
            self.grid.select_row(row);
        }
    }

    fn apply_filter(&self) {
        if self.has_changes() {
            return;
        }
        self.read_filter_widgets();
        self.spawn(|model| async move { model.apply_filter().await });
    }

    /// Copies the filter widgets into the model's draft filter.
    fn read_filter_widgets(&self) {
        let column = match self.filter.column.selected() {
            0 | gtk::INVALID_LIST_POSITION => String::new(),
            index => self.filter_columns.borrow().get(index as usize - 1).cloned().unwrap_or_default(),
        };
        let op = FilterOperator::ALL.get(self.filter.operator.selected() as usize).copied().unwrap_or_default();
        let mut state = self.model.state.borrow_mut();
        state.filter.column = column;
        state.filter.op = op;
        state.filter.value = self.filter.value.text().to_string();
    }

    fn connect(this: &Rc<Self>) {
        let weak = Rc::downgrade(this);
        this.model.set_on_change(move || {
            if let Some(tab) = weak.upgrade() {
                tab.refresh();
            }
        });

        let weak = Rc::downgrade(this);
        this.grid.connect_edit(move |row, column, value| {
            if let Some(tab) = weak.upgrade() {
                tab.model.set_value(value, row, column);
            }
        });
        let weak = Rc::downgrade(this);
        this.grid.connect_sort(move |column| {
            if let Some(tab) = weak.upgrade() {
                if tab.has_changes() {
                    return;
                }
                tab.spawn(move |model| async move { model.sort(column).await });
            }
        });
        let weak = Rc::downgrade(this);
        this.grid.connect_delete(move |rows| {
            if let Some(tab) = weak.upgrade() {
                tab.model.delete_rows(&rows);
            }
        });
        let weak = Rc::downgrade(this);
        this.grid.connect_selection_changed(move || {
            if let Some(tab) = weak.upgrade() {
                tab.delete_button.set_sensitive(tab.can_edit() && !tab.grid.selected_rows().is_empty());
            }
        });

        let weak = Rc::downgrade(this);
        this.data_toggle.connect_toggled(move |button| {
            if let Some(tab) = weak.upgrade()
                && !tab.syncing.get()
            {
                tab.model.state.borrow_mut().mode =
                    if button.is_active() { TableMode::Data } else { TableMode::Structure };
                tab.model.notify();
            }
        });

        let weak = Rc::downgrade(this);
        let filter_changed = move || {
            if let Some(tab) = weak.upgrade()
                && !tab.syncing.get()
            {
                tab.read_filter_widgets();
                tab.refresh();
            }
        };
        let changed = filter_changed.clone();
        this.filter.column.connect_selected_notify(move |_| changed());
        let changed = filter_changed.clone();
        this.filter.operator.connect_selected_notify(move |_| changed());
        this.filter.value.connect_changed(move |_| filter_changed());

        let weak = Rc::downgrade(this);
        this.filter.value.connect_activate(move |_| {
            if let Some(tab) = weak.upgrade() {
                tab.apply_filter();
            }
        });
        let weak = Rc::downgrade(this);
        this.filter.apply.connect_clicked(move |_| {
            if let Some(tab) = weak.upgrade() {
                tab.apply_filter();
            }
        });
        let weak = Rc::downgrade(this);
        this.filter.clear.connect_clicked(move |_| {
            if let Some(tab) = weak.upgrade() {
                tab.spawn(|model| async move { model.clear_filter().await });
            }
        });

        let weak = Rc::downgrade(this);
        this.add_button.connect_clicked(move |_| {
            if let Some(tab) = weak.upgrade() {
                tab.add_row();
            }
        });
        let weak = Rc::downgrade(this);
        this.delete_button.connect_clicked(move |_| {
            if let Some(tab) = weak.upgrade() {
                tab.model.delete_rows(&tab.grid.selected_rows());
            }
        });
        let weak = Rc::downgrade(this);
        this.discard_button.connect_clicked(move |_| {
            if let Some(tab) = weak.upgrade() {
                tab.discard();
            }
        });
        let weak = Rc::downgrade(this);
        this.save_button.connect_clicked(move |_| {
            if let Some(tab) = weak.upgrade() {
                tab.save();
            }
        });
        let weak = Rc::downgrade(this);
        this.previous_button.connect_clicked(move |_| {
            if let Some(tab) = weak.upgrade() {
                let page = tab.model.state.borrow().page.saturating_sub(1);
                tab.spawn(move |model| async move { model.go_to_page(page).await });
            }
        });
        let weak = Rc::downgrade(this);
        this.next_button.connect_clicked(move |_| {
            if let Some(tab) = weak.upgrade() {
                let page = tab.model.state.borrow().page + 1;
                tab.spawn(move |model| async move { model.go_to_page(page).await });
            }
        });
        let weak = Rc::downgrade(this);
        this.export_button.connect_clicked(move |button| {
            if let Some(tab) = weak.upgrade() {
                let state = tab.model.state.borrow();
                let result = QueryResult {
                    columns: state.result_columns.clone(),
                    rows: state.rows.clone(),
                    ..QueryResult::default()
                };
                export_csv(button.upcast_ref(), &state.table.name, result);
            }
        });
        let weak = Rc::downgrade(this);
        this.reload_button.connect_clicked(move |_| {
            if let Some(tab) = weak.upgrade() {
                tab.reload();
            }
        });
    }

    /// Syncs every widget with the model.
    fn refresh(&self) {
        self.syncing.set(true);
        let state = self.model.state.borrow();
        let data_mode = state.mode == TableMode::Data;
        let has_changes = state.has_changes();
        let can_edit = state.can_edit();

        self.stack.set_visible_child_name(if data_mode { "data" } else { "structure" });
        self.data_toggle.set_active(data_mode);
        self.structure_toggle.set_active(!data_mode);

        // Filter bar
        let names: Vec<String> = if state.result_columns.is_empty() {
            state.columns.iter().map(|c| c.name.clone()).collect()
        } else {
            state.result_columns.clone()
        };
        if *self.filter_columns.borrow() != names {
            let mut items = vec!["Raw SQL"];
            items.extend(names.iter().map(String::as_str));
            self.filter.columns.splice(0, self.filter.columns.n_items(), &items);
            *self.filter_columns.borrow_mut() = names.clone();
        }
        let column_index = match state.filter.column.as_str() {
            "" => 0,
            name => names.iter().position(|n| n == name).map_or(0, |i| i as u32 + 1),
        };
        self.filter.column.set_selected(column_index);
        let op_index = FilterOperator::ALL.iter().position(|op| *op == state.filter.op).unwrap_or(0);
        self.filter.operator.set_selected(op_index as u32);
        let raw = state.filter.is_raw();
        self.filter.operator.set_visible(!raw);
        self.filter.value.set_visible(raw || state.filter.op.needs_value());
        self.filter.value.set_placeholder_text(Some(if raw {
            "WHERE clause, e.g. id > 10 AND status = 'active'"
        } else {
            "Value"
        }));
        if raw {
            self.filter.value.add_css_class("monospace");
        } else {
            self.filter.value.remove_css_class("monospace");
        }
        if self.filter.value.text() != state.filter.value {
            self.filter.value.set_text(&state.filter.value);
        }
        self.filter.apply.set_sensitive(!has_changes);
        self.filter.clear.set_visible(!state.applied_filter.is_empty() && state.applied_filter != Default::default());
        self.filter.clear.set_sensitive(!has_changes);

        // Grids
        let page_version = state.page_version;
        let structure_version = state.structure_version;
        let structure = (structure_version != self.shown_structure.get()).then(|| structure_sources(&state));
        self.loading.set_visible(state.is_loading);
        self.error.set_message(state.error.as_deref());

        // Bottom bar
        self.data_controls.set_visible(data_mode);
        self.add_button.set_sensitive(can_edit);
        let reason = state.edit_disabled_reason();
        self.lock.set_visible(reason.is_some());
        self.lock_label.set_text(reason.unwrap_or(""));
        self.changes.set_visible(has_changes);
        self.changes.set_text(&state.changes_text());
        self.discard_button.set_visible(has_changes);
        self.save_button.set_visible(has_changes);
        self.status.set_visible(!has_changes && data_mode);
        self.status.set_text(&state.status_text());
        for widget in [
            self.previous_button.upcast_ref::<gtk::Widget>(),
            self.page_label.upcast_ref(),
            self.next_button.upcast_ref(),
            self.export_button.upcast_ref(),
        ] {
            widget.set_visible(data_mode);
        }
        self.previous_button.set_sensitive(state.page > 0 && !has_changes);
        self.page_label.set_text(&(state.page + 1).to_string());
        self.next_button.set_sensitive(state.has_next_page() && !has_changes);
        self.export_button.set_sensitive(!state.rows.is_empty());
        self.reload_button.set_sensitive(!has_changes);
        drop(state);

        if self.shown_page.replace(page_version) != page_version {
            self.grid.reload();
        } else {
            self.grid.refresh();
        }
        self.delete_button.set_sensitive(can_edit && !self.grid.selected_rows().is_empty());
        if let Some((columns, indexes)) = structure {
            self.shown_structure.set(structure_version);
            self.columns_grid.set_source(Rc::new(columns));
            self.indexes_grid.set_source(Rc::new(indexes));
        }
        self.syncing.set(false);

        if let Some(callback) = self.on_state_change.borrow().as_ref() {
            callback();
        }
    }
}

fn structure_sources(state: &crate::models::table::TableState) -> (StaticSource, StaticSource) {
    let text = |s: &str| Some(s.to_string());
    let yes_no = |b: bool| text(if b { "YES" } else { "NO" });
    let columns = StaticSource {
        columns: ["#", "Column", "Type", "Nullable", "Default", "Extra", "Comment"].map(String::from).to_vec(),
        rows: state
            .columns
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let name = if c.is_primary_key { format!("🔑 {}", c.name) } else { c.name.clone() };
                vec![
                    Some((i + 1).to_string()),
                    Some(name),
                    text(&c.data_type),
                    yes_no(c.nullable),
                    c.default_value.clone(),
                    text(&c.extra),
                    text(&c.comment),
                ]
            })
            .collect(),
    };
    let indexes = StaticSource {
        columns: ["Name", "Columns", "Unique", "Definition"].map(String::from).to_vec(),
        rows: state
            .indexes
            .iter()
            .map(|i| {
                let name = if i.primary { format!("🔑 {}", i.name) } else { i.name.clone() };
                vec![Some(name), text(&i.columns), yes_no(i.unique), text(&i.definition)]
            })
            .collect(),
    };
    (columns, indexes)
}
