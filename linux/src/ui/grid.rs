//! A spreadsheet-like grid on top of `GtkColumnView`. Cells pull their
//! values lazily from a `GridSource`, so large results stay fast.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use gtk::{gdk, gio, glib, graphene};

use super::widgets::{detach_on_unrealize, popup_at};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowState {
    Normal,
    Inserted,
    Deleted,
}

pub struct GridCell {
    pub value: Option<String>,
    /// Shown dimmed instead of `NULL` when the value is missing (e.g. DEFAULT).
    pub placeholder: Option<&'static str>,
    pub edited: bool,
}

impl GridCell {
    pub fn plain(value: Option<String>) -> Self {
        Self { value, placeholder: None, edited: false }
    }
}

pub trait GridSource {
    fn columns(&self) -> Vec<String>;
    fn row_count(&self) -> usize;
    fn cell(&self, row: usize, column: usize) -> GridCell;
    fn row_state(&self, _row: usize) -> RowState {
        RowState::Normal
    }
    fn editable(&self) -> bool {
        false
    }
    /// Server-side sort shown in the headers, when the grid is sortable.
    fn sort(&self) -> Option<(String, bool)> {
        None
    }
}

/// A source with fixed contents (query results, structure listings).
pub struct StaticSource {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
}

impl GridSource for StaticSource {
    fn columns(&self) -> Vec<String> {
        self.columns.clone()
    }

    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn cell(&self, row: usize, column: usize) -> GridCell {
        GridCell::plain(self.rows.get(row).and_then(|r| r.get(column)).cloned().flatten())
    }
}

/// Display form of a value: one line, at most 400 characters.
pub fn display_text(value: &str) -> String {
    let mut text: String = value.chars().take(400).collect();
    if text.len() < value.len() {
        text.push('…');
    }
    text.replace("\r\n", " ").replace(['\n', '\r'], " ")
}

#[derive(Clone)]
struct CellWidget {
    root: gtk::Box,
    label: gtk::Inscription,
    position: Rc<Cell<Option<(usize, usize)>>>,
}

struct Editing {
    row: usize,
    column: usize,
    entry: gtk::Entry,
    cell: CellWidget,
}

type EditHandler = Box<dyn Fn(usize, usize, Option<String>)>;
type RowsHandler = Box<dyn Fn(BTreeSet<usize>)>;
type Handler<T> = RefCell<Option<Box<dyn Fn(T)>>>;

struct Inner {
    scroller: gtk::ScrolledWindow,
    view: gtk::ColumnView,
    model: gtk::StringList,
    selection: gtk::MultiSelection,
    menu: gtk::PopoverMenu,
    actions: gio::SimpleActionGroup,
    sortable: bool,

    source: RefCell<Rc<dyn GridSource>>,
    column_names: RefCell<Vec<String>>,
    view_columns: RefCell<Vec<gtk::ColumnViewColumn>>,
    /// Cell widgets per list item, and the bound ones per (row, column).
    widgets: RefCell<HashMap<glib::Object, CellWidget>>,
    bound: RefCell<HashMap<(usize, usize), CellWidget>>,
    editing: RefCell<Option<Editing>>,
    /// Cell of the last click; the context menu and Enter act on it.
    clicked: Cell<(usize, usize)>,
    syncing_sort: Cell<bool>,

    on_edit: RefCell<Option<EditHandler>>,
    on_sort: Handler<usize>,
    on_delete: RefCell<Option<RowsHandler>>,
    on_selection: RefCell<Option<Box<dyn Fn()>>>,
}

#[derive(Clone)]
pub struct DataGrid {
    inner: Rc<Inner>,
}

impl DataGrid {
    pub fn new(source: Rc<dyn GridSource>, sortable: bool) -> Self {
        let model = gtk::StringList::new(&[]);
        let selection = gtk::MultiSelection::new(Some(model.clone()));
        let view = gtk::ColumnView::new(Some(selection.clone()));
        view.add_css_class("data-grid");
        view.set_show_column_separators(true);
        view.set_show_row_separators(true);
        view.set_reorderable(false);
        let scroller = gtk::ScrolledWindow::builder().child(&view).hexpand(true).vexpand(true).build();
        let menu = gtk::PopoverMenu::from_model(None::<&gio::MenuModel>);
        menu.set_has_arrow(false);
        menu.set_halign(gtk::Align::Start);
        let actions = gio::SimpleActionGroup::new();
        view.insert_action_group("grid", Some(&actions));

        let inner = Rc::new(Inner {
            scroller,
            view,
            model,
            selection,
            menu,
            actions,
            sortable,
            source: RefCell::new(source),
            column_names: RefCell::default(),
            view_columns: RefCell::default(),
            widgets: RefCell::default(),
            bound: RefCell::default(),
            editing: RefCell::new(None),
            clicked: Cell::new((0, 0)),
            syncing_sort: Cell::new(false),
            on_edit: RefCell::new(None),
            on_sort: RefCell::new(None),
            on_delete: RefCell::new(None),
            on_selection: RefCell::new(None),
        });
        Inner::connect(&inner);
        let grid = Self { inner };
        grid.reload();
        grid
    }

    pub fn widget(&self) -> &gtk::ScrolledWindow {
        &self.inner.scroller
    }

    pub fn set_source(&self, source: Rc<dyn GridSource>) {
        *self.inner.source.borrow_mut() = source;
        self.reload();
    }

    pub fn connect_edit(&self, f: impl Fn(usize, usize, Option<String>) + 'static) {
        *self.inner.on_edit.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_sort(&self, f: impl Fn(usize) + 'static) {
        *self.inner.on_sort.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_delete(&self, f: impl Fn(BTreeSet<usize>) + 'static) {
        *self.inner.on_delete.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_selection_changed(&self, f: impl Fn() + 'static) {
        *self.inner.on_selection.borrow_mut() = Some(Box::new(f));
    }

    /// Rebuilds the columns when they changed and rebinds every row.
    pub fn reload(&self) {
        let inner = &self.inner;
        inner.finish_editing(false, false);
        let columns = inner.source().columns();
        if *inner.column_names.borrow() != columns {
            inner.rebuild_columns(&columns);
        }
        let count = inner.source().row_count();
        let items = vec![""; count];
        inner.model.splice(0, inner.model.n_items(), &items);
        inner.sync_sort();
    }

    /// Redraws visible cells after edits, and adds or drops rows at the end.
    pub fn refresh(&self) {
        let inner = &self.inner;
        let count = inner.source().row_count() as u32;
        let current = inner.model.n_items();
        if count > current {
            let items = vec![""; (count - current) as usize];
            inner.model.splice(current, 0, &items);
        } else if count < current {
            inner.model.splice(count, current - count, &[]);
        }
        let bound: Vec<CellWidget> = inner.bound.borrow().values().cloned().collect();
        for cell in bound {
            inner.configure(&cell);
        }
        inner.sync_sort();
    }

    pub fn selected_rows(&self) -> BTreeSet<usize> {
        self.inner.selected_rows()
    }

    /// Selects and scrolls to a row; deferred so a row that was just
    /// appended is laid out first.
    pub fn select_row(&self, row: usize) {
        let view = self.inner.view.clone();
        glib::idle_add_local_once(move || {
            view.scroll_to(row as u32, None, gtk::ListScrollFlags::SELECT | gtk::ListScrollFlags::FOCUS, None);
        });
    }

    pub fn grab_focus(&self) {
        self.inner.view.grab_focus();
    }
}

impl Inner {
    fn source(&self) -> Rc<dyn GridSource> {
        self.source.borrow().clone()
    }

    fn connect(this: &Rc<Self>) {
        let weak = Rc::downgrade(this);
        this.view.connect_activate(move |_, position| {
            if let Some(inner) = weak.upgrade() {
                let column = inner.clicked.get().1;
                Inner::begin_editing(&inner, position as usize, column);
            }
        });

        let weak = Rc::downgrade(this);
        this.selection.connect_selection_changed(move |_, _, _| {
            if let Some(inner) = weak.upgrade()
                && let Some(callback) = inner.on_selection.borrow().as_ref()
            {
                callback();
            }
        });

        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(this);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(inner) = weak.upgrade() else { return glib::Propagation::Proceed };
            if inner.editing.borrow().is_some() {
                return glib::Propagation::Proceed;
            }
            let control = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
            match key {
                gdk::Key::Delete | gdk::Key::KP_Delete | gdk::Key::BackSpace if !control => {
                    inner.delete_selected();
                    glib::Propagation::Stop
                }
                gdk::Key::c | gdk::Key::C if control => {
                    inner.copy_rows(&inner.selected_rows());
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        this.view.add_controller(keys);

        if let Some(sorter) = this.view.sorter() {
            let weak = Rc::downgrade(this);
            sorter.connect_changed(move |sorter, _| {
                let Some(inner) = weak.upgrade() else { return };
                if inner.syncing_sort.get() {
                    return;
                }
                let clicked = sorter
                    .downcast_ref::<gtk::ColumnViewSorter>()
                    .and_then(gtk::ColumnViewSorter::primary_sort_column)
                    .and_then(|column| inner.view_columns.borrow().iter().position(|c| *c == column));
                if let Some(index) = clicked
                    && let Some(callback) = inner.on_sort.borrow().as_ref()
                {
                    callback(index);
                }
                // The owner decides the new order; show that, not GTK's.
                inner.sync_sort();
            });
        }

        detach_on_unrealize(&this.menu, &this.view);
        Self::install_actions(this);
    }

    fn rebuild_columns(self: &Rc<Self>, columns: &[String]) {
        for column in self.view_columns.borrow_mut().drain(..) {
            self.view.remove_column(&column);
        }
        let source = self.source();
        let sample = source.row_count().min(60);
        for (index, name) in columns.iter().enumerate() {
            let factory = gtk::SignalListItemFactory::new();
            let weak = Rc::downgrade(self);
            factory.connect_setup(move |_, item| {
                if let Some(inner) = weak.upgrade() {
                    Inner::setup_cell(&inner, item);
                }
            });
            let weak = Rc::downgrade(self);
            factory.connect_bind(move |_, item| {
                if let Some(inner) = weak.upgrade() {
                    inner.bind_cell(item, index);
                }
            });
            let weak = Rc::downgrade(self);
            factory.connect_unbind(move |_, item| {
                if let Some(inner) = weak.upgrade() {
                    inner.unbind_cell(item);
                }
            });
            let weak = Rc::downgrade(self);
            factory.connect_teardown(move |_, item| {
                if let Some(inner) = weak.upgrade() {
                    inner.widgets.borrow_mut().remove(item);
                }
            });

            let column = gtk::ColumnViewColumn::new(Some(name), Some(factory));
            column.set_resizable(true);
            column.set_fixed_width(self.ideal_width(&*source, index, name, sample));
            if self.sortable {
                column.set_sorter(Some(&gtk::CustomSorter::new(|_, _| gtk::Ordering::Equal)));
            }
            self.view.append_column(&column);
            self.view_columns.borrow_mut().push(column);
        }
        *self.column_names.borrow_mut() = columns.to_vec();
    }

    fn ideal_width(&self, source: &dyn GridSource, column: usize, title: &str, sample: usize) -> i32 {
        let measure = |text: &str| self.view.create_pango_layout(Some(text)).pixel_size().0;
        let mut width = measure(title) + 34;
        for row in 0..sample {
            if let Some(value) = source.cell(row, column).value {
                width = width.max(measure(&display_text(&value)) + 18);
            }
        }
        width.clamp(64, 360)
    }

    fn setup_cell(this: &Rc<Self>, item: &glib::Object) {
        let label = gtk::Inscription::new(None);
        label.set_text_overflow(gtk::InscriptionOverflow::EllipsizeEnd);
        label.set_hexpand(true);
        label.set_xalign(0.0);
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.append(&label);
        let cell = CellWidget { root: root.clone(), label, position: Rc::new(Cell::new(None)) };

        // Remember which cell was clicked (for Enter, double-click and the menu).
        let click = gtk::GestureClick::builder().button(0).build();
        let weak = Rc::downgrade(this);
        let position = cell.position.clone();
        click.connect_pressed(move |gesture, _, x, y| {
            let (Some(inner), Some((row, column))) = (weak.upgrade(), position.get()) else { return };
            inner.clicked.set((row, column));
            if gesture.current_button() == gdk::BUTTON_SECONDARY {
                let widget = gesture.widget().expect("gesture is attached");
                Inner::show_menu(&inner, &widget, x, y);
            }
        });
        // Double-click edits. The row claims the release, so start from the
        // second press, after the row has handled selection and focus.
        let weak = Rc::downgrade(this);
        let position = cell.position.clone();
        click.connect_pressed(move |gesture, n_press, _, _| {
            if n_press != 2 || gesture.current_button() != gdk::BUTTON_PRIMARY {
                return;
            }
            if let (Some(inner), Some((row, column))) = (weak.upgrade(), position.get()) {
                glib::timeout_add_local_once(std::time::Duration::from_millis(30), move || {
                    Inner::begin_editing(&inner, row, column)
                });
            }
        });
        root.add_controller(click);

        if let Some(list_cell) = item.downcast_ref::<gtk::ColumnViewCell>() {
            list_cell.set_child(Some(&root));
        }
        this.widgets.borrow_mut().insert(item.clone(), cell);
    }

    fn bind_cell(&self, item: &glib::Object, column: usize) {
        let Some(list_cell) = item.downcast_ref::<gtk::ColumnViewCell>() else { return };
        let Some(cell) = self.widgets.borrow().get(item).cloned() else { return };
        let row = list_cell.position() as usize;
        cell.position.set(Some((row, column)));
        self.bound.borrow_mut().insert((row, column), cell.clone());
        self.configure(&cell);
    }

    fn unbind_cell(&self, item: &glib::Object) {
        let Some(cell) = self.widgets.borrow().get(item).cloned() else { return };
        if let Some(position) = cell.position.take() {
            let editing_here = self.editing.borrow().as_ref().is_some_and(|e| (e.row, e.column) == position);
            if editing_here {
                self.finish_editing(true, false);
            }
            let mut bound = self.bound.borrow_mut();
            if bound.get(&position).is_some_and(|c| c.root == cell.root) {
                bound.remove(&position);
            }
        }
    }

    fn configure(&self, cell: &CellWidget) {
        let Some((row, column)) = cell.position.get() else { return };
        let source = self.source();
        if row >= source.row_count() {
            return;
        }
        let data = source.cell(row, column);
        let row_state = source.row_state(row);
        let mut classes = vec!["grid-cell"];
        match row_state {
            RowState::Normal => {}
            RowState::Inserted => classes.push("row-inserted"),
            RowState::Deleted => classes.push("row-deleted"),
        }
        if data.edited {
            classes.push("cell-edited");
        }
        cell.root.set_css_classes(&classes);
        let mut label_classes = Vec::new();
        match &data.value {
            Some(value) => cell.label.set_text(Some(&display_text(value))),
            None => {
                label_classes.push("null-value");
                cell.label.set_text(Some(data.placeholder.unwrap_or("NULL")));
            }
        }
        if row_state == RowState::Deleted {
            label_classes.push("deleted-value");
        }
        cell.label.set_css_classes(&label_classes);
    }

    fn selected_rows(&self) -> BTreeSet<usize> {
        let set = self.selection.selection();
        let mut rows = BTreeSet::new();
        if let Some((iter, first)) = gtk::BitsetIter::init_first(&set) {
            rows.insert(first as usize);
            rows.extend(iter.map(|row| row as usize));
        }
        rows
    }

    fn sync_sort(&self) {
        if !self.sortable {
            return;
        }
        self.syncing_sort.set(true);
        let sort = self.source().sort();
        let column = sort.as_ref().and_then(|(name, _)| {
            let index = self.column_names.borrow().iter().position(|c| c == name)?;
            self.view_columns.borrow().get(index).cloned()
        });
        let order = if sort.is_some_and(|(_, ascending)| !ascending) {
            gtk::SortType::Descending
        } else {
            gtk::SortType::Ascending
        };
        self.view.sort_by_column(column.as_ref(), order);
        self.syncing_sort.set(false);
    }

    // Editing

    fn can_edit(&self, row: usize) -> bool {
        let source = self.source();
        source.editable()
            && self.on_edit.borrow().is_some()
            && row < source.row_count()
            && source.row_state(row) != RowState::Deleted
    }

    fn begin_editing(this: &Rc<Self>, row: usize, column: usize) {
        if !this.can_edit(row) || column >= this.column_names.borrow().len() {
            return;
        }
        if this.editing.borrow().as_ref().is_some_and(|e| (e.row, e.column) == (row, column)) {
            return;
        }
        this.finish_editing(true, false);
        let Some(cell) = this.bound.borrow().get(&(row, column)).cloned() else {
            // Not on screen yet: scroll there and try again once laid out.
            let view_column = this.view_columns.borrow().get(column).cloned();
            this.view.scroll_to(row as u32, view_column.as_ref(), gtk::ListScrollFlags::FOCUS, None);
            let weak = Rc::downgrade(this);
            glib::idle_add_local_once(move || {
                if let Some(inner) = weak.upgrade()
                    && inner.bound.borrow().contains_key(&(row, column))
                {
                    Inner::begin_editing(&inner, row, column);
                }
            });
            return;
        };

        let entry = gtk::Entry::new();
        entry.add_css_class("grid-editor");
        entry.set_hexpand(true);
        entry.set_text(this.source().cell(row, column).value.as_deref().unwrap_or(""));
        cell.label.set_visible(false);
        cell.root.append(&entry);
        *this.editing.borrow_mut() = Some(Editing { row, column, entry: entry.clone(), cell });

        let weak = Rc::downgrade(this);
        entry.connect_activate(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.finish_editing(true, true);
            }
        });

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(this);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(inner) = weak.upgrade() else { return glib::Propagation::Proceed };
            let next = match key {
                gdk::Key::Escape => {
                    inner.finish_editing(false, true);
                    return glib::Propagation::Stop;
                }
                gdk::Key::Tab | gdk::Key::KP_Tab => column + 1,
                gdk::Key::ISO_Left_Tab => column.wrapping_sub(1),
                _ => return glib::Propagation::Proceed,
            };
            inner.finish_editing(true, true);
            if next < inner.column_names.borrow().len() {
                Inner::begin_editing(&inner, row, next);
            }
            glib::Propagation::Stop
        });
        entry.add_controller(keys);

        // Clicking elsewhere commits, like a spreadsheet.
        let focus = gtk::EventControllerFocus::new();
        let weak: Weak<Self> = Rc::downgrade(this);
        let editing_entry = entry.clone();
        focus.connect_leave(move |_| {
            let Some(inner) = weak.upgrade() else { return };
            let entry = editing_entry.clone();
            glib::idle_add_local_once(move || {
                let still_editing = inner.editing.borrow().as_ref().is_some_and(|e| e.entry == entry);
                if still_editing && !entry.has_focus() && !entry.is_focus() {
                    inner.finish_editing(true, false);
                }
            });
        });
        entry.add_controller(focus);

        entry.grab_focus();
    }

    fn finish_editing(&self, commit: bool, refocus: bool) {
        let Some(editing) = self.editing.borrow_mut().take() else { return };
        let new = editing.entry.text().to_string();
        editing.cell.root.remove(&editing.entry);
        editing.cell.label.set_visible(true);
        if commit && editing.row < self.source().row_count() {
            let old = self.source().cell(editing.row, editing.column).value;
            // Leaving a NULL cell empty keeps it NULL.
            if (new != old.as_deref().unwrap_or("") || (old.is_none() && !new.is_empty()))
                && let Some(callback) = self.on_edit.borrow().as_ref()
            {
                callback(editing.row, editing.column, Some(new));
            }
        }
        self.configure(&editing.cell);
        if refocus {
            self.view.grab_focus();
        }
    }

    // Copy, delete and the context menu

    fn row_text(&self, row: usize) -> String {
        let source = self.source();
        (0..self.column_names.borrow().len())
            .map(|column| source.cell(row, column).value.unwrap_or_else(|| "NULL".into()))
            .collect::<Vec<_>>()
            .join("\t")
    }

    fn copy_rows(&self, rows: &BTreeSet<usize>) {
        if rows.is_empty() {
            return;
        }
        let text = rows.iter().map(|&row| self.row_text(row)).collect::<Vec<_>>().join("\n");
        self.view.clipboard().set_text(&text);
    }

    fn delete_selected(&self) {
        if !self.source().editable() {
            return;
        }
        let rows = self.selected_rows();
        if rows.is_empty() {
            return;
        }
        if let Some(callback) = self.on_delete.borrow().as_ref() {
            callback(rows);
        }
    }

    fn set_selected_values(&self, value: Option<String>) {
        let column = self.clicked.get().1;
        for row in self.selected_rows() {
            if self.can_edit(row)
                && let Some(callback) = self.on_edit.borrow().as_ref()
            {
                callback(row, column, value.clone());
            }
        }
    }

    fn install_actions(this: &Rc<Self>) {
        let add = |name: &str, handler: fn(&Rc<Inner>)| {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(this);
            action.connect_activate(move |_, _| {
                if let Some(inner) = weak.upgrade() {
                    handler(&inner);
                }
            });
            this.actions.add_action(&action);
        };
        add("copy-value", |inner| {
            let (row, column) = inner.clicked.get();
            let value = inner.source().cell(row, column).value.unwrap_or_else(|| "NULL".into());
            inner.view.clipboard().set_text(&value);
        });
        add("copy-rows", |inner| inner.copy_rows(&inner.selected_rows()));
        add("copy-column-name", |inner| {
            let name = inner.column_names.borrow().get(inner.clicked.get().1).cloned();
            if let Some(name) = name {
                inner.view.clipboard().set_text(&name);
            }
        });
        add("edit-value", |inner| {
            let (row, column) = inner.clicked.get();
            Inner::begin_editing(inner, row, column);
        });
        add("set-null", |inner| inner.set_selected_values(None));
        add("set-empty", |inner| inner.set_selected_values(Some(String::new())));
        add("delete-rows", |inner| inner.delete_selected());
    }

    fn show_menu(this: &Rc<Self>, widget: &gtk::Widget, x: f64, y: f64) {
        let (row, _) = this.clicked.get();
        if !this.selection.is_selected(row as u32) {
            this.selection.select_item(row as u32, true);
        }
        let rows = this.selected_rows().len();
        let plural = if rows > 1 { "s" } else { "" };

        let menu = gio::Menu::new();
        let copy = gio::Menu::new();
        copy.append(Some("Copy Value"), Some("grid.copy-value"));
        copy.append(Some(&format!("Copy Row{plural}")), Some("grid.copy-rows"));
        copy.append(Some("Copy Column Name"), Some("grid.copy-column-name"));
        menu.append_section(None, &copy);
        if this.can_edit(row) {
            let edit = gio::Menu::new();
            edit.append(Some("Edit Value"), Some("grid.edit-value"));
            edit.append(Some("Set NULL"), Some("grid.set-null"));
            edit.append(Some("Set Empty String"), Some("grid.set-empty"));
            menu.append_section(None, &edit);
        }
        if this.source().editable() && this.on_delete.borrow().is_some() {
            let delete = gio::Menu::new();
            delete.append(Some(&format!("Delete Row{plural}")), Some("grid.delete-rows"));
            menu.append_section(None, &delete);
        }
        this.menu.set_menu_model(Some(&menu));

        let point = widget
            .compute_point(&this.view, &graphene::Point::new(x as f32, y as f32))
            .unwrap_or_else(|| graphene::Point::new(0.0, 0.0));
        popup_at(&this.menu, &this.view, point.x() as i32, point.y() as i32);
    }
}
