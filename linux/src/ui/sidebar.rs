use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gdk, gio};

use super::widgets::{detach_on_unrealize, popup_at};
use super::workspace::Workspace;
use crate::db::{TableKind, TableRef};
use crate::models::connection::DatabaseKind;

/// What the sidebar shows; set by the workspace after every change.
#[derive(Default)]
pub struct SidebarContent {
    pub kind: DatabaseKind,
    pub tables: Vec<TableRef>,
    pub schemas: Vec<String>,
    pub current_schema: Option<String>,
    pub current_database: Option<String>,
    pub error: Option<String>,
    pub loading: bool,
}

pub struct Sidebar {
    pub root: gtk::Box,
    schemas: gtk::StringList,
    schema: gtk::DropDown,
    schema_box: gtk::Box,
    list: gtk::ListBox,
    stack: gtk::Stack,
    message: gtk::Label,
    search: gtk::SearchEntry,
    menu: gtk::PopoverMenu,
    content: RefCell<SidebarContent>,
    /// Table of each list row, `None` for section headers.
    rows: RefCell<Vec<(gtk::ListBoxRow, Option<TableRef>)>>,
    selected: RefCell<Option<TableRef>>,
    menu_target: RefCell<Option<TableRef>>,
    syncing: Cell<bool>,
    workspace: RefCell<Weak<Workspace>>,
}

impl Sidebar {
    pub fn new() -> Rc<Self> {
        let schemas = gtk::StringList::new(&[]);
        let schema = gtk::DropDown::builder().model(&schemas).hexpand(true).tooltip_text("Schema").build();
        let schema_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let schema_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        schema_row.set_margin_top(6);
        schema_row.set_margin_bottom(6);
        schema_row.set_margin_start(8);
        schema_row.set_margin_end(8);
        schema_row.append(&schema);
        schema_box.append(&schema_row);
        schema_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        let list = gtk::ListBox::new();
        list.add_css_class("navigation-sidebar");
        list.set_selection_mode(gtk::SelectionMode::Single);
        let scroller =
            gtk::ScrolledWindow::builder().child(&list).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        let message = gtk::Label::builder()
            .wrap(true)
            .selectable(true)
            .xalign(0.0)
            .margin_start(12)
            .margin_end(12)
            .margin_top(12)
            .build();
        let spinner =
            gtk::Spinner::builder().spinning(true).halign(gtk::Align::Center).valign(gtk::Align::Center).build();
        let stack = gtk::Stack::new();
        stack.add_named(&scroller, Some("list"));
        stack.add_named(&spinner, Some("loading"));

        let search = gtk::SearchEntry::builder().placeholder_text("Search tables").build();
        search.set_margin_top(6);
        search.set_margin_bottom(6);
        search.set_margin_start(8);
        search.set_margin_end(8);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&schema_box);
        root.append(&message);
        root.append(&stack);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&search);
        root.set_size_request(200, -1);

        let menu = gtk::PopoverMenu::from_model(None::<&gio::MenuModel>);
        menu.set_has_arrow(false);
        menu.set_halign(gtk::Align::Start);

        let sidebar = Rc::new(Self {
            root,
            schemas,
            schema,
            schema_box,
            list,
            stack,
            message,
            search,
            menu,
            content: RefCell::default(),
            rows: RefCell::default(),
            selected: RefCell::new(None),
            menu_target: RefCell::new(None),
            syncing: Cell::new(false),
            workspace: RefCell::new(Weak::new()),
        });
        Self::connect(&sidebar);
        sidebar
    }

    pub fn set_workspace(&self, workspace: Weak<Workspace>) {
        *self.workspace.borrow_mut() = workspace;
    }

    fn workspace(&self) -> Option<Rc<Workspace>> {
        self.workspace.borrow().upgrade()
    }

    fn connect(this: &Rc<Self>) {
        let weak = Rc::downgrade(this);
        this.list.connect_row_selected(move |_, row| {
            let Some(sidebar) = weak.upgrade() else { return };
            if sidebar.syncing.get() {
                return;
            }
            let table = row.and_then(|row| sidebar.table_for(row));
            if let (Some(table), Some(workspace)) = (table, sidebar.workspace()) {
                workspace.open(&table, false);
            }
        });

        let weak = Rc::downgrade(this);
        this.search.connect_search_changed(move |_| {
            if let Some(sidebar) = weak.upgrade() {
                sidebar.rebuild();
            }
        });

        let weak = Rc::downgrade(this);
        this.schema.connect_selected_notify(move |dropdown| {
            let Some(sidebar) = weak.upgrade() else { return };
            if sidebar.syncing.get() {
                return;
            }
            let name = sidebar.schemas.string(dropdown.selected()).map(|s| s.to_string());
            if let (Some(name), Some(workspace)) = (name, sidebar.workspace()) {
                Workspace::select_schema(&workspace, name);
            }
        });

        let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        let weak = Rc::downgrade(this);
        click.connect_pressed(move |_, _, x, y| {
            let Some(sidebar) = weak.upgrade() else { return };
            let Some(row) = sidebar.list.row_at_y(y as i32) else { return };
            let Some(table) = sidebar.table_for(&row) else { return };
            sidebar.show_menu(table, x, y);
        });
        this.list.add_controller(click);

        let actions = gio::SimpleActionGroup::new();
        let add = |name: &str, handler: fn(&Rc<Workspace>, TableRef)| {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(this);
            action.connect_activate(move |_, _| {
                let Some(sidebar) = weak.upgrade() else { return };
                let target = sidebar.menu_target.borrow().clone();
                if let (Some(table), Some(workspace)) = (target, sidebar.workspace()) {
                    handler(&workspace, table);
                }
            });
            actions.add_action(&action);
        };
        add("open-new-tab", |workspace, table| workspace.open(&table, true));
        add("query-new-tab", |workspace, table| {
            let sql = format!("SELECT * FROM {} LIMIT 100;", workspace.config.kind.qualified_name(&table));
            Workspace::new_query_tab(workspace, &sql);
        });
        add("copy-name", |workspace, table| workspace.window.clipboard().set_text(&table.name));
        add("truncate", |workspace, table| Workspace::confirm_destructive(workspace, table, false));
        add("drop", |workspace, table| Workspace::confirm_destructive(workspace, table, true));
        this.list.insert_action_group("sidebar", Some(&actions));

        detach_on_unrealize(&this.menu, &this.list);
    }

    fn table_for(&self, row: &gtk::ListBoxRow) -> Option<TableRef> {
        self.rows.borrow().iter().find(|(r, _)| r == row).and_then(|(_, table)| table.clone())
    }

    fn show_menu(&self, table: TableRef, x: f64, y: f64) {
        let menu = gio::Menu::new();
        let open = gio::Menu::new();
        open.append(Some("Open in New Tab"), Some("sidebar.open-new-tab"));
        open.append(Some("Query in New Tab"), Some("sidebar.query-new-tab"));
        open.append(Some("Copy Name"), Some("sidebar.copy-name"));
        menu.append_section(None, &open);
        let danger = gio::Menu::new();
        if table.kind == TableKind::Table {
            danger.append(Some("Truncate…"), Some("sidebar.truncate"));
        }
        danger.append(Some("Drop…"), Some("sidebar.drop"));
        menu.append_section(None, &danger);
        *self.menu_target.borrow_mut() = Some(table);
        self.menu.set_menu_model(Some(&menu));
        popup_at(&self.menu, &self.list, x as i32, y as i32);
    }

    pub fn update(&self, content: SidebarContent) {
        self.syncing.set(true);
        let show_schemas = content.kind == DatabaseKind::Postgres && !content.schemas.is_empty();
        self.schema_box.set_visible(show_schemas);
        let names: Vec<&str> = content.schemas.iter().map(String::as_str).collect();
        let current: Vec<String> =
            (0..self.schemas.n_items()).filter_map(|i| self.schemas.string(i).map(Into::into)).collect();
        if current != content.schemas {
            self.schemas.splice(0, self.schemas.n_items(), &names);
        }
        if let Some(index) = content.current_schema.as_ref().and_then(|s| content.schemas.iter().position(|n| n == s)) {
            self.schema.set_selected(index as u32);
        }

        let message = match (&content.error, content.kind, &content.current_database) {
            (Some(error), _, _) => Some((error.clone(), "error")),
            (None, DatabaseKind::MySql, None) => Some(("Choose a database from the toolbar.".to_string(), "dim-label")),
            _ => None,
        };
        self.message.set_visible(message.is_some());
        if let Some((text, class)) = message {
            self.message.set_text(&text);
            self.message.set_css_classes(&[class, "caption"]);
        }
        self.stack.set_visible_child_name(if content.loading && content.tables.is_empty() {
            "loading"
        } else {
            "list"
        });
        *self.content.borrow_mut() = content;
        self.syncing.set(false);
        self.rebuild();
    }

    fn rebuild(&self) {
        self.syncing.set(true);
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        let mut rows = Vec::new();
        let query = self.search.text().trim().to_lowercase();
        let content = self.content.borrow();
        let matches = |table: &&TableRef| query.is_empty() || table.name.to_lowercase().contains(&query);
        for (kind, title) in [(TableKind::Table, "Tables"), (TableKind::View, "Views")] {
            let tables: Vec<&TableRef> = content.tables.iter().filter(|t| t.kind == kind).filter(matches).collect();
            if tables.is_empty() {
                continue;
            }
            let header = gtk::Label::builder().label(format!("{title} ({})", tables.len())).xalign(0.0).build();
            header.add_css_class("heading");
            header.add_css_class("dim-label");
            let header_row = gtk::ListBoxRow::builder().child(&header).selectable(false).activatable(false).build();
            self.list.append(&header_row);
            rows.push((header_row, None));
            for table in tables {
                let icon = gtk::Image::from_icon_name(if kind == TableKind::View {
                    "view-reveal-symbolic"
                } else {
                    "view-grid-symbolic"
                });
                if kind == TableKind::View {
                    icon.add_css_class("accent");
                }
                let label = gtk::Label::builder()
                    .label(&table.name)
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build();
                let item = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                item.append(&icon);
                item.append(&label);
                let row = gtk::ListBoxRow::builder().child(&item).build();
                self.list.append(&row);
                rows.push((row, Some(table.clone())));
            }
        }
        drop(content);
        *self.rows.borrow_mut() = rows;
        self.syncing.set(false);
        let selected = self.selected.borrow().clone();
        self.select_table(selected.as_ref());
    }

    /// Highlights the table of the current tab without opening it again.
    pub fn select_table(&self, table: Option<&TableRef>) {
        *self.selected.borrow_mut() = table.cloned();
        self.syncing.set(true);
        let row = table.and_then(|table| {
            self.rows.borrow().iter().find(|(_, t)| t.as_ref() == Some(table)).map(|(row, _)| row.clone())
        });
        match row {
            Some(row) => self.list.select_row(Some(&row)),
            None => self.list.unselect_all(),
        }
        self.syncing.set(false);
    }
}

/// Escapes `_` so menu labels are not read as mnemonics.
pub fn menu_label(text: &str) -> String {
    text.replace('_', "__")
}
