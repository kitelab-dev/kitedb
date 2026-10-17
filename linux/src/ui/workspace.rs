//! One open connection window: the session, the sidebar and the tabs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use adw::prelude::*;
use gtk::{gio, glib};

use super::App;
use super::query_tab::QueryTab;
use super::sidebar::{Sidebar, SidebarContent, menu_label};
use super::table_tab::TableTab;
use super::widgets::kind_badge;
use crate::db::{Session, TableKind, TableRef, make_driver};
use crate::models::connection::{ConnectionConfig, DatabaseKind};
use crate::models::secrets;

#[derive(Clone)]
enum Tab {
    Table(Rc<TableTab>),
    Query(Rc<QueryTab>),
}

impl Tab {
    fn table(&self) -> Option<TableRef> {
        match self {
            Tab::Table(tab) => Some(tab.table()),
            Tab::Query(_) => None,
        }
    }
}

#[derive(Clone, PartialEq)]
enum Status {
    Connecting,
    Connected,
    Failed(String),
}

struct State {
    session: Option<Arc<Session>>,
    status: Status,
    server_version: String,
    databases: Vec<String>,
    current_database: Option<String>,
    schemas: Vec<String>,
    current_schema: Option<String>,
    tables: Vec<TableRef>,
    loading_tables: bool,
    sidebar_error: Option<String>,
    query_counter: usize,
}

pub struct Workspace {
    app: Weak<App>,
    pub config: ConnectionConfig,
    pub window: adw::ApplicationWindow,
    state: RefCell<State>,
    tabs: RefCell<Vec<(adw::TabPage, Tab)>>,
    title: adw::WindowTitle,
    badge: gtk::Box,
    badge_status: gtk::Label,
    database_button: gtk::MenuButton,
    database_label: gtk::Label,
    refresh_button: gtk::Button,
    new_query_button: gtk::Button,
    stack: gtk::Stack,
    connecting_label: gtk::Label,
    failed_page: adw::StatusPage,
    tab_view: adw::TabView,
    tab_stack: gtk::Stack,
    sidebar: Rc<Sidebar>,
    actions: RefCell<HashMap<&'static str, gio::SimpleAction>>,
    database_action: gio::SimpleAction,
}

impl Workspace {
    pub fn new(app: &Rc<App>, config: ConnectionConfig) -> Rc<Self> {
        let window = adw::ApplicationWindow::builder()
            .application(&app.app)
            .title(config.display_name())
            .default_width(1280)
            .default_height(800)
            .build();
        window.set_size_request(900, 520);

        // Header bar
        let badge_status = gtk::Label::new(Some("Connecting…"));
        badge_status.add_css_class("caption");
        badge_status.add_css_class("dim-label");
        let badge = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        badge.add_css_class("connection-badge");
        badge.add_css_class(&format!("tint-{}", config.color.name()));
        badge.append(&kind_badge(config.kind, 20));
        badge.append(&badge_status);
        badge.set_valign(gtk::Align::Center);

        let title = adw::WindowTitle::new(config.display_name(), "");
        let database_label = gtk::Label::new(Some("Database"));
        let database_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        database_content.append(&gtk::Image::from_icon_name("network-server-symbolic"));
        database_content.append(&database_label);
        let database_button = gtk::MenuButton::builder()
            .child(&database_content)
            .tooltip_text("Switch database")
            .always_show_arrow(true)
            .build();
        let refresh_button =
            gtk::Button::builder().icon_name("view-refresh-symbolic").tooltip_text("Refresh (Ctrl+R)").build();
        refresh_button.set_action_name(Some("win.refresh"));
        let new_query_button =
            gtk::Button::builder().icon_name("tab-new-symbolic").tooltip_text("New query tab (Ctrl+T)").build();
        new_query_button.set_action_name(Some("win.new-query"));
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&main_menu())
            .tooltip_text("Main Menu")
            .primary(true)
            .build();

        let header = adw::HeaderBar::new();
        header.pack_start(&badge);
        header.set_title_widget(Some(&title));
        header.pack_end(&menu_button);
        header.pack_end(&database_button);
        header.pack_end(&new_query_button);
        header.pack_end(&refresh_button);

        // Content
        let connecting_label = gtk::Label::new(Some(&format!("Connecting to {}…", config.host)));
        connecting_label.add_css_class("dim-label");
        let connecting = gtk::Box::new(gtk::Orientation::Vertical, 12);
        connecting.set_valign(gtk::Align::Center);
        connecting.append(&gtk::Spinner::builder().spinning(true).width_request(32).height_request(32).build());
        connecting.append(&connecting_label);

        let retry = gtk::Button::builder().label("Try Again").halign(gtk::Align::Center).build();
        retry.add_css_class("pill");
        retry.add_css_class("suggested-action");
        let failed_page = adw::StatusPage::builder()
            .icon_name("network-error-symbolic")
            .title("Connection Failed")
            .child(&retry)
            .build();

        let sidebar = Sidebar::new();
        let tab_view = adw::TabView::new();
        let tab_bar = adw::TabBar::builder().view(&tab_view).autohide(false).build();
        let new_tab =
            gtk::Button::builder().icon_name("list-add-symbolic").tooltip_text("New query tab (Ctrl+T)").build();
        new_tab.add_css_class("flat");
        new_tab.set_action_name(Some("win.new-query"));
        tab_bar.set_end_action_widget(Some(&new_tab));

        let no_tabs_button = gtk::Button::builder().label("New Query").halign(gtk::Align::Center).build();
        no_tabs_button.add_css_class("pill");
        no_tabs_button.set_action_name(Some("win.new-query"));
        let no_tabs = adw::StatusPage::builder()
            .icon_name("view-grid-symbolic")
            .title("No Open Tabs")
            .description("Pick a table from the sidebar or open a new query with Ctrl+T.")
            .child(&no_tabs_button)
            .build();
        let tab_stack = gtk::Stack::new();
        tab_stack.add_named(&tab_view, Some("tabs"));
        tab_stack.add_named(&no_tabs, Some("empty"));
        tab_stack.set_vexpand(true);
        let detail = gtk::Box::new(gtk::Orientation::Vertical, 0);
        detail.append(&tab_bar);
        detail.append(&tab_stack);

        let paned = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&sidebar.root)
            .end_child(&detail)
            .resize_start_child(false)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .position(250)
            .build();

        let stack = gtk::Stack::new();
        stack.add_named(&connecting, Some("connecting"));
        stack.add_named(&failed_page, Some("failed"));
        stack.add_named(&paned, Some("connected"));

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&stack));
        window.set_content(Some(&toolbar));

        let database_action =
            gio::SimpleAction::new_stateful("select-database", Some(glib::VariantTy::STRING), &"".to_variant());

        let workspace = Rc::new_cyclic(|weak: &Weak<Self>| {
            sidebar.set_workspace(weak.clone());
            Self {
                app: Rc::downgrade(app),
                config,
                window,
                state: RefCell::new(State {
                    session: None,
                    status: Status::Connecting,
                    server_version: String::new(),
                    databases: Vec::new(),
                    current_database: None,
                    schemas: Vec::new(),
                    current_schema: None,
                    tables: Vec::new(),
                    loading_tables: false,
                    sidebar_error: None,
                    query_counter: 0,
                }),
                tabs: RefCell::default(),
                title,
                badge,
                badge_status,
                database_button,
                database_label,
                refresh_button,
                new_query_button,
                stack,
                connecting_label,
                failed_page,
                tab_view,
                tab_stack,
                sidebar,
                actions: RefCell::default(),
                database_action,
            }
        });
        Self::install_actions(&workspace);

        let weak = Rc::downgrade(&workspace);
        retry.connect_clicked(move |_| {
            if let Some(workspace) = weak.upgrade() {
                Workspace::connect(&workspace);
            }
        });
        let weak = Rc::downgrade(&workspace);
        workspace.tab_view.connect_selected_page_notify(move |_| {
            if let Some(workspace) = weak.upgrade() {
                workspace.selected_tab_changed();
            }
        });
        let weak = Rc::downgrade(&workspace);
        workspace.tab_view.connect_close_page(move |_, page| {
            if let Some(workspace) = weak.upgrade() {
                workspace.tabs.borrow_mut().retain(|(p, _)| p != page);
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(&workspace);
        workspace.tab_view.connect_n_pages_notify(move |view| {
            if let Some(workspace) = weak.upgrade() {
                workspace.tab_stack.set_visible_child_name(if view.n_pages() > 0 { "tabs" } else { "empty" });
                workspace.update_actions();
            }
        });
        let weak = Rc::downgrade(&workspace);
        workspace.window.connect_close_request(move |_| {
            if let Some(workspace) = weak.upgrade() {
                workspace.disconnect();
                if let Some(app) = workspace.app.upgrade() {
                    App::workspace_closed(&app, &workspace);
                }
            }
            glib::Propagation::Proceed
        });

        workspace.refresh_header();
        workspace.update_actions();
        workspace
    }

    fn install_actions(this: &Rc<Self>) {
        let weak = Rc::downgrade(this);
        let add = |name: &'static str, handler: fn(&Rc<Workspace>)| {
            let action = gio::SimpleAction::new(name, None);
            let weak = weak.clone();
            action.connect_activate(move |_, _| {
                if let Some(workspace) = weak.upgrade() {
                    handler(&workspace);
                }
            });
            this.window.add_action(&action);
            this.actions.borrow_mut().insert(name, action);
        };
        add("new-query", |w| Workspace::new_query_tab(w, ""));
        add("run-query", |w| {
            if let Some(Tab::Query(tab)) = w.selected_tab() {
                tab.run();
            }
        });
        add("cancel-query", |w| {
            if let Some(Tab::Query(tab)) = w.selected_tab() {
                tab.cancel();
            }
        });
        add("save-changes", |w| {
            if let Some(Tab::Table(tab)) = w.selected_tab() {
                tab.save();
            }
        });
        add("discard-changes", |w| {
            if let Some(Tab::Table(tab)) = w.selected_tab() {
                tab.discard();
            }
        });
        add("add-row", |w| {
            if let Some(Tab::Table(tab)) = w.selected_tab() {
                tab.add_row();
            }
        });
        add("refresh", |w| {
            if let Some(Tab::Table(tab)) = w.selected_tab() {
                tab.reload();
            }
            Workspace::refresh(w);
        });
        add("close-tab", |w| {
            if let Some(page) = w.tab_view.selected_page() {
                w.tab_view.close_page(&page);
            }
        });
        add("next-tab", |w| w.select_tab(1));
        add("previous-tab", |w| w.select_tab(-1));

        let weak = Rc::downgrade(this);
        this.database_action.connect_activate(move |_, parameter| {
            let (Some(workspace), Some(name)) = (weak.upgrade(), parameter.and_then(|p| p.get::<String>())) else {
                return;
            };
            Workspace::select_database(&workspace, name);
        });
        this.window.add_action(&this.database_action);
    }

    fn session(&self) -> Option<Arc<Session>> {
        self.state.borrow().session.clone()
    }

    fn selected_tab(&self) -> Option<Tab> {
        let page = self.tab_view.selected_page()?;
        self.tabs.borrow().iter().find(|(p, _)| *p == page).map(|(_, tab)| tab.clone())
    }

    // Connection

    pub fn connect(this: &Rc<Self>) {
        this.state.borrow_mut().status = Status::Connecting;
        this.refresh_header();
        let this = this.clone();
        glib::spawn_future_local(async move {
            let id = this.config.id;
            let password = gio::spawn_blocking(move || secrets::password(id)).await.unwrap_or_default();
            let session = Session::new(make_driver(&this.config, &password));
            let info = session
                .run(|driver| {
                    driver.connect()?;
                    let databases = driver.databases().unwrap_or_default();
                    Ok((driver.server_version(), databases, driver.current_database()))
                })
                .await;
            match info {
                Ok((version, databases, current)) => {
                    {
                        let mut state = this.state.borrow_mut();
                        state.session = Some(session);
                        state.server_version = version;
                        state.databases = databases;
                        state.current_database = current;
                        state.status = Status::Connected;
                    }
                    this.refresh_header();
                    Workspace::load_schemas_and_tables(&this).await;
                    if this.tabs.borrow().is_empty() {
                        Workspace::new_query_tab(&this, "");
                    }
                }
                Err(error) => {
                    this.state.borrow_mut().status = Status::Failed(error.message);
                    this.refresh_header();
                }
            }
        });
    }

    fn disconnect(&self) {
        if let Some(session) = self.state.borrow_mut().session.take() {
            session.close();
        }
    }

    // Sidebar

    pub fn refresh(this: &Rc<Self>) {
        let Some(session) = this.session() else { return };
        let this = this.clone();
        glib::spawn_future_local(async move {
            if let Ok(databases) = session.run(|driver| driver.databases()).await {
                this.state.borrow_mut().databases = databases;
                this.refresh_header();
            }
            Workspace::load_schemas_and_tables(&this).await;
        });
    }

    pub fn select_database(this: &Rc<Self>, name: String) {
        let Some(session) = this.session() else { return };
        if this.state.borrow().current_database.as_ref() == Some(&name) {
            return;
        }
        let this = this.clone();
        glib::spawn_future_local(async move {
            let target = name.clone();
            match session.run(move |driver| driver.use_database(&target)).await {
                Ok(()) => {
                    {
                        let mut state = this.state.borrow_mut();
                        state.current_database = Some(name);
                        state.current_schema = None;
                        state.tables.clear();
                    }
                    // Table tabs belong to the previous database.
                    let table_pages: Vec<adw::TabPage> = this
                        .tabs
                        .borrow()
                        .iter()
                        .filter(|(_, tab)| matches!(tab, Tab::Table(_)))
                        .map(|(page, _)| page.clone())
                        .collect();
                    for page in table_pages {
                        this.tab_view.close_page(&page);
                    }
                    this.refresh_header();
                    Workspace::load_schemas_and_tables(&this).await;
                }
                Err(error) => {
                    this.state.borrow_mut().sidebar_error = Some(error.message);
                    this.refresh_sidebar();
                    this.refresh_header();
                }
            }
        });
    }

    pub fn select_schema(this: &Rc<Self>, name: String) {
        if this.state.borrow().current_schema.as_ref() == Some(&name) {
            return;
        }
        this.state.borrow_mut().current_schema = Some(name);
        let this = this.clone();
        glib::spawn_future_local(async move { Workspace::load_tables(&this).await });
    }

    async fn load_schemas_and_tables(this: &Rc<Self>) {
        let Some(session) = this.session() else { return };
        if this.config.kind == DatabaseKind::Postgres {
            let schemas = session.run(|driver| driver.schemas()).await.unwrap_or_default();
            let mut state = this.state.borrow_mut();
            if state.current_schema.as_ref().is_none_or(|s| !schemas.contains(s)) {
                state.current_schema = schemas.first().cloned();
            }
            state.schemas = schemas;
        }
        Workspace::load_tables(this).await;
    }

    async fn load_tables(this: &Rc<Self>) {
        let Some(session) = this.session() else { return };
        this.state.borrow_mut().loading_tables = true;
        this.refresh_sidebar();
        let schema = this.state.borrow().current_schema.clone();
        let result = session.run(move |driver| driver.tables(schema.as_deref())).await;
        {
            let mut state = this.state.borrow_mut();
            state.loading_tables = false;
            match result {
                Ok(tables) => {
                    state.tables = tables;
                    state.sidebar_error = None;
                }
                Err(error) => {
                    state.tables.clear();
                    state.sidebar_error = Some(error.message);
                }
            }
        }
        this.refresh_sidebar();
        this.update_completion_words();
    }

    pub fn confirm_destructive(this: &Rc<Self>, table: TableRef, drop: bool) {
        let verb = if drop { "Drop" } else { "Truncate" };
        let dialog = adw::AlertDialog::new(Some(&format!("{verb} “{}”?", table.name)), Some("This cannot be undone."));
        dialog.add_responses(&[("cancel", "Cancel"), ("confirm", verb)]);
        dialog.set_response_appearance("confirm", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let this = this.clone();
        glib::spawn_future_local(async move {
            if dialog.choose_future(Some(&this.window)).await == "confirm" {
                Workspace::drop_or_truncate(&this, table, drop).await;
            }
        });
    }

    async fn drop_or_truncate(this: &Rc<Self>, table: TableRef, drop: bool) {
        let Some(session) = this.session() else { return };
        let name = this.config.kind.qualified_name(&table);
        let sql = if drop {
            format!("DROP {} {name}", if table.kind == TableKind::View { "VIEW" } else { "TABLE" })
        } else {
            format!("TRUNCATE TABLE {name}")
        };
        match session.run(move |driver| driver.execute(&sql)).await {
            Ok(_) => {
                let affected: Vec<(adw::TabPage, Tab)> = this
                    .tabs
                    .borrow()
                    .iter()
                    .filter(|(_, tab)| tab.table().as_ref() == Some(&table))
                    .cloned()
                    .collect();
                for (page, tab) in affected {
                    match (drop, tab) {
                        (true, _) => this.tab_view.close_page(&page),
                        (false, Tab::Table(tab)) => tab.reload(),
                        _ => {}
                    }
                }
                Workspace::load_tables(this).await;
            }
            Err(error) => {
                this.state.borrow_mut().sidebar_error = Some(error.message);
                this.refresh_sidebar();
            }
        }
    }

    // Tabs

    /// Opens a table. Reuses an existing tab for it, otherwise replaces the
    /// current table tab when it has no pending edits (like a preview tab).
    pub fn open(self: &Rc<Self>, table: &TableRef, new_tab: bool) {
        let Some(session) = self.session() else { return };
        let existing =
            self.tabs.borrow().iter().find(|(_, tab)| tab.table().as_ref() == Some(table)).map(|(p, _)| p.clone());
        if let Some(page) = existing {
            self.tab_view.set_selected_page(&page);
            return;
        }
        let tab = TableTab::new(table.clone(), session);
        let replace = match (new_tab, self.selected_tab(), self.tab_view.selected_page()) {
            (false, Some(Tab::Table(current)), Some(page)) if !current.has_changes() => Some(page),
            _ => None,
        };
        let page = match &replace {
            Some(current) => self.tab_view.insert(&tab.root, self.tab_view.page_position(current) + 1),
            None => self.tab_view.append(&tab.root),
        };
        page.set_title(&table.name);
        page.set_tooltip(&table.name);
        page.set_icon(Some(&gio::ThemedIcon::new(if table.kind == TableKind::View {
            "view-reveal-symbolic"
        } else {
            "view-grid-symbolic"
        })));
        let weak = Rc::downgrade(self);
        tab.set_on_state_change(move || {
            if let Some(workspace) = weak.upgrade() {
                workspace.update_actions();
            }
        });
        self.tabs.borrow_mut().push((page.clone(), Tab::Table(tab)));
        self.tab_view.set_selected_page(&page);
        if let Some(current) = replace {
            self.tab_view.close_page(&current);
        }
    }

    pub fn new_query_tab(this: &Rc<Self>, sql: &str) {
        let Some(session) = this.session() else { return };
        let title = {
            let mut state = this.state.borrow_mut();
            state.query_counter += 1;
            format!("Query {}", state.query_counter)
        };
        let tab = QueryTab::new(title.clone(), session, sql);
        tab.set_completion_words(this.completion_words());
        let page = this.tab_view.append(&tab.root);
        page.set_title(&title);
        page.set_icon(Some(&gio::ThemedIcon::new("utilities-terminal-symbolic")));
        let weak = Rc::downgrade(this);
        tab.set_on_state_change(move || {
            if let Some(workspace) = weak.upgrade() {
                workspace.update_actions();
            }
        });
        this.tabs.borrow_mut().push((page.clone(), Tab::Query(tab.clone())));
        this.tab_view.set_selected_page(&page);
        tab.focus();
    }

    fn select_tab(&self, offset: i32) {
        let count = self.tab_view.n_pages();
        let Some(page) = self.tab_view.selected_page() else { return };
        if count == 0 {
            return;
        }
        let index = (self.tab_view.page_position(&page) + offset).rem_euclid(count);
        self.tab_view.set_selected_page(&self.tab_view.nth_page(index));
    }

    fn selected_tab_changed(&self) {
        let tab = self.selected_tab();
        self.sidebar.select_table(tab.as_ref().and_then(Tab::table).as_ref());
        match &tab {
            Some(Tab::Query(tab)) => tab.focus(),
            Some(Tab::Table(tab)) => tab.focus(),
            None => {}
        }
        self.update_actions();
    }

    /// All table names, used by the SQL editor's completion.
    fn completion_words(&self) -> Vec<String> {
        self.state.borrow().tables.iter().map(|t| t.name.clone()).collect()
    }

    fn update_completion_words(&self) {
        let words = self.completion_words();
        for (_, tab) in self.tabs.borrow().iter() {
            if let Tab::Query(tab) = tab {
                tab.set_completion_words(words.clone());
            }
        }
    }

    // Syncing widgets

    fn update_actions(&self) {
        let connected = self.state.borrow().status == Status::Connected;
        let tab = self.selected_tab();
        let query = match &tab {
            Some(Tab::Query(tab)) => Some(tab),
            _ => None,
        };
        let table = match &tab {
            Some(Tab::Table(tab)) => Some(tab),
            _ => None,
        };
        let changes = table.is_some_and(|t| t.has_changes());
        let enable = |name: &str, enabled: bool| {
            if let Some(action) = self.actions.borrow().get(name) {
                action.set_enabled(enabled);
            }
        };
        enable("new-query", connected);
        enable("run-query", query.is_some_and(|q| !q.is_running()));
        enable("cancel-query", query.is_some_and(|q| q.is_running()));
        enable("save-changes", changes);
        enable("discard-changes", changes);
        enable("add-row", table.is_some_and(|t| t.can_edit()));
        enable("refresh", connected);
        enable("close-tab", tab.is_some());
        enable("next-tab", self.tab_view.n_pages() > 1);
        enable("previous-tab", self.tab_view.n_pages() > 1);
    }

    fn refresh_header(&self) {
        let state = self.state.borrow();
        let connected = state.status == Status::Connected;
        match &state.status {
            Status::Connecting => {
                self.connecting_label.set_text(&format!("Connecting to {}…", self.config.host));
                self.stack.set_visible_child_name("connecting");
                self.badge_status.set_text("Connecting…");
            }
            Status::Failed(message) => {
                self.failed_page.set_description(Some(&glib::markup_escape_text(message)));
                self.stack.set_visible_child_name("failed");
                self.badge_status.set_text("Disconnected");
            }
            Status::Connected => {
                self.stack.set_visible_child_name("connected");
                let version = state.server_version.split_whitespace().next().unwrap_or("");
                self.badge_status.set_text(&format!("{} {version}", self.config.kind.display_name()));
            }
        }
        self.badge.set_tooltip_text(Some(&self.config.summary()));
        self.title.set_subtitle(state.current_database.as_deref().unwrap_or(""));
        self.database_button.set_visible(connected);
        self.refresh_button.set_visible(connected);
        self.new_query_button.set_visible(connected);
        self.database_label.set_text(state.current_database.as_deref().unwrap_or("Database"));
        self.database_action.set_state(&state.current_database.clone().unwrap_or_default().to_variant());

        let menu = gio::Menu::new();
        for name in &state.databases {
            let item = gio::MenuItem::new(Some(&menu_label(name)), None);
            item.set_action_and_target_value(Some("win.select-database"), Some(&name.to_variant()));
            menu.append_item(&item);
        }
        self.database_button.set_menu_model(Some(&menu));
        drop(state);
        self.update_actions();
    }

    fn refresh_sidebar(&self) {
        let state = self.state.borrow();
        self.sidebar.update(SidebarContent {
            kind: self.config.kind,
            tables: state.tables.clone(),
            schemas: state.schemas.clone(),
            current_schema: state.current_schema.clone(),
            current_database: state.current_database.clone(),
            error: state.sidebar_error.clone(),
            loading: state.loading_tables,
        });
    }
}

fn main_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let query = gio::Menu::new();
    query.append(Some("New Query Tab"), Some("win.new-query"));
    query.append(Some("Run Query"), Some("win.run-query"));
    query.append(Some("Cancel Query"), Some("win.cancel-query"));
    menu.append_section(None, &query);
    let edit = gio::Menu::new();
    edit.append(Some("Save Changes"), Some("win.save-changes"));
    edit.append(Some("Discard Changes"), Some("win.discard-changes"));
    edit.append(Some("Add Row"), Some("win.add-row"));
    menu.append_section(None, &edit);
    let tabs = gio::Menu::new();
    tabs.append(Some("Refresh"), Some("win.refresh"));
    tabs.append(Some("Close Tab"), Some("win.close-tab"));
    menu.append_section(None, &tabs);
    menu.append_section(None, &super::app_menu_section());
    menu
}
