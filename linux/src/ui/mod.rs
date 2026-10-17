mod connection_form;
mod editor;
mod grid;
mod query_tab;
mod sidebar;
mod table_tab;
mod welcome;
pub mod widgets;
mod workspace;

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use uuid::Uuid;

use self::welcome::Welcome;
use self::widgets::{APP_ID, DONATE, ISSUES, VERSION, WEBSITE, open_uri};
use self::workspace::Workspace;
use crate::models::connection::ConnectionConfig;
use crate::models::secrets;
use crate::models::store::ConnectionStore;

/// App-wide state: the saved connections and the open windows.
pub struct App {
    pub app: adw::Application,
    pub store: RefCell<ConnectionStore>,
    welcome: RefCell<Option<Rc<Welcome>>>,
    workspaces: RefCell<Vec<Rc<Workspace>>>,
}

pub fn run() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    let state = Rc::new(App {
        app: app.clone(),
        store: RefCell::new(ConnectionStore::load()),
        welcome: RefCell::new(None),
        workspaces: RefCell::default(),
    });

    let startup_state = state.clone();
    app.connect_startup(move |_| App::startup(&startup_state));
    let activate_state = state.clone();
    app.connect_activate(move |_| App::show_welcome(&activate_state));
    let code = app.run();
    // Break the App <-> Application reference cycle.
    state.workspaces.borrow_mut().clear();
    state.welcome.borrow_mut().take();
    code
}

/// "Connections…", help links and About, shared by every main menu.
pub fn app_menu_section() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some("Connections…"), Some("app.connections"));
    menu.append(Some("KiteDB Website"), Some("app.website"));
    menu.append(Some("Report an Issue…"), Some("app.report-issue"));
    menu.append(Some("Donate via Byl…"), Some("app.donate"));
    menu.append(Some("About KiteDB"), Some("app.about"));
    menu
}

impl App {
    fn startup(this: &Rc<Self>) {
        widgets::load_css();
        gtk::Window::set_default_icon_name(APP_ID);
        // When running from the source tree the icon is not installed yet.
        let dev_icons = concat!(env!("CARGO_MANIFEST_DIR"), "/data/icons");
        if let Some(display) = gtk::gdk::Display::default()
            && std::path::Path::new(dev_icons).is_dir()
        {
            gtk::IconTheme::for_display(&display).add_search_path(dev_icons);
        }

        let add = |name: &str, handler: fn(&Rc<App>)| {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(this);
            action.connect_activate(move |_, _| {
                if let Some(app) = weak.upgrade() {
                    handler(&app);
                }
            });
            this.app.add_action(&action);
        };
        add("connections", App::show_welcome);
        add("website", |app| open_uri(app.app.active_window().as_ref(), WEBSITE));
        add("report-issue", |app| open_uri(app.app.active_window().as_ref(), ISSUES));
        add("donate", |app| open_uri(app.app.active_window().as_ref(), DONATE));
        add("about", App::show_about);
        add("quit", |app| app.app.quit());

        let accels: [(&str, &[&str]); 14] = [
            ("app.connections", &["<Control><Shift>k"]),
            ("app.quit", &["<Control>q"]),
            ("win.new-connection", &["<Control>n"]),
            ("win.new-query", &["<Control>t"]),
            ("win.run-query", &["<Control>Return", "<Control>KP_Enter"]),
            ("win.cancel-query", &["<Control>period"]),
            ("win.save-changes", &["<Control>s"]),
            ("win.add-row", &["<Control>i"]),
            ("win.refresh", &["<Control>r", "F5"]),
            ("win.close-tab", &["<Control>w"]),
            ("win.next-tab", &["<Control><Shift>bracketright"]),
            ("win.previous-tab", &["<Control><Shift>bracketleft"]),
            ("win.discard-changes", &[]),
            ("app.about", &[]),
        ];
        for (action, keys) in accels {
            this.app.set_accels_for_action(action, keys);
        }
    }

    pub fn show_welcome(this: &Rc<Self>) {
        let existing = this.welcome.borrow().clone();
        let welcome = match existing {
            Some(welcome) => welcome,
            None => {
                let welcome = Welcome::new(this);
                let weak = Rc::downgrade(this);
                welcome.window.connect_close_request(move |_| {
                    if let Some(app) = weak.upgrade() {
                        app.welcome.borrow_mut().take();
                    }
                    glib::Propagation::Proceed
                });
                *this.welcome.borrow_mut() = Some(welcome.clone());
                welcome
            }
        };
        welcome.rebuild();
        welcome.window.present();
    }

    /// Opens (or focuses) the window for a saved connection and closes the
    /// connections window.
    pub fn open_workspace(this: &Rc<Self>, id: Uuid) {
        let existing = this.workspaces.borrow().iter().find(|w| w.config.id == id).cloned();
        let workspace = match existing {
            Some(workspace) => workspace,
            None => {
                let Some(config) = this.store.borrow().connection(id).cloned() else { return };
                let workspace = Workspace::new(this, config);
                this.workspaces.borrow_mut().push(workspace.clone());
                Workspace::connect(&workspace);
                workspace
            }
        };
        workspace.window.present();
        // Take it out first: closing re-enters `welcome` via close-request.
        let welcome = this.welcome.borrow_mut().take();
        if let Some(welcome) = welcome {
            welcome.window.close();
        }
    }

    pub fn workspace_closed(this: &Rc<Self>, workspace: &Rc<Workspace>) {
        this.workspaces.borrow_mut().retain(|w| !Rc::ptr_eq(w, workspace));
        if this.workspaces.borrow().is_empty() {
            App::show_welcome(this);
        }
    }

    /// Saves a connection, stores its password, then optionally connects.
    pub fn save_connection(this: &Rc<Self>, config: ConnectionConfig, password: String, connect: bool) {
        let id = config.id;
        this.store.borrow_mut().save(config);
        let this = this.clone();
        glib::spawn_future_local(async move {
            let _ = gio::spawn_blocking(move || secrets::set_password(id, &password)).await;
            if connect {
                App::open_workspace(&this, id);
            }
        });
    }

    pub fn duplicate_connection(this: &Rc<Self>, id: Uuid) {
        let Some(copy) = this.store.borrow_mut().duplicate(id) else { return };
        let copy_id = copy.id;
        // Runs right away; nothing needs to wait for it.
        drop(gio::spawn_blocking(move || {
            let password = secrets::password(id);
            secrets::set_password(copy_id, &password);
        }));
        let welcome = this.welcome.borrow().clone();
        if let Some(welcome) = welcome {
            welcome.rebuild();
            welcome.select(copy_id);
        }
    }

    pub fn delete_connection(this: &Rc<Self>, id: Uuid) {
        this.store.borrow_mut().delete(id);
        drop(gio::spawn_blocking(move || secrets::delete_password(id)));
    }

    fn show_about(this: &Rc<Self>) {
        let about = adw::AboutDialog::builder()
            .application_name("KiteDB")
            .application_icon(APP_ID)
            .version(VERSION)
            .comments("A simple, native client for MySQL and PostgreSQL.")
            .website(WEBSITE)
            .issue_url(ISSUES)
            .license_type(gtk::License::MitX11)
            .developer_name("Kitelab")
            .build();
        about.add_link("Donate via Byl", DONATE);
        about.present(this.app.active_window().as_ref());
    }
}
