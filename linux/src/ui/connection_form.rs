use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::db::{Session, make_driver};
use crate::models::connection::{ConnectionColor, ConnectionConfig, DatabaseKind, SslMode};
use crate::models::secrets;

type SaveHandler = Box<dyn Fn(ConnectionConfig, String, bool)>;

struct Form {
    dialog: adw::Dialog,
    config: RefCell<ConnectionConfig>,
    kind_buttons: Vec<(DatabaseKind, gtk::ToggleButton)>,
    name: adw::EntryRow,
    color_buttons: Vec<(ConnectionColor, gtk::ToggleButton)>,
    host: adw::EntryRow,
    port: adw::EntryRow,
    user: adw::EntryRow,
    password: adw::PasswordEntryRow,
    database: adw::EntryRow,
    ssl: adw::ComboRow,
    test_status: gtk::Label,
    test_spinner: gtk::Spinner,
    test_button: gtk::Button,
    test_run: Cell<u64>,
    on_save: SaveHandler,
}

/// Shows the new / edit connection dialog. `on_save` gets the config, the
/// password and whether to connect right away.
pub fn present(
    parent: &impl IsA<gtk::Widget>,
    config: ConnectionConfig,
    is_new: bool,
    on_save: impl Fn(ConnectionConfig, String, bool) + 'static,
) {
    let dialog = adw::Dialog::builder()
        .title(if is_new { "New Connection" } else { "Edit Connection" })
        .content_width(500)
        .build();

    // Type and appearance
    let kind_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    kind_box.add_css_class("linked");
    kind_box.set_valign(gtk::Align::Center);
    let mut kind_buttons: Vec<(DatabaseKind, gtk::ToggleButton)> = Vec::new();
    for kind in DatabaseKind::ALL {
        let button = gtk::ToggleButton::with_label(kind.display_name());
        if let Some((_, first)) = kind_buttons.first() {
            button.set_group(Some(first));
        }
        kind_box.append(&button);
        kind_buttons.push((kind, button));
    }
    let kind_row = adw::ActionRow::builder().title("Type").build();
    kind_row.add_suffix(&kind_box);

    let name = adw::EntryRow::builder().title("Name").build();

    let colors_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    colors_box.set_valign(gtk::Align::Center);
    let mut color_buttons: Vec<(ConnectionColor, gtk::ToggleButton)> = Vec::new();
    for color in ConnectionColor::ALL {
        let button =
            gtk::ToggleButton::builder().tooltip_text(capitalize(color.name())).valign(gtk::Align::Center).build();
        button.add_css_class("color-swatch");
        button.add_css_class(&format!("tint-{}", color.name()));
        if color == ConnectionColor::None {
            button.add_css_class("none");
        }
        if let Some((_, first)) = color_buttons.first() {
            button.set_group(Some(first));
        }
        colors_box.append(&button);
        color_buttons.push((color, button));
    }
    let color_row = adw::ActionRow::builder().title("Color").build();
    color_row.add_suffix(&colors_box);

    let general = adw::PreferencesGroup::new();
    general.add(&kind_row);
    general.add(&name);
    general.add(&color_row);

    // Server
    let host = adw::EntryRow::builder().title("Host").build();
    let port = adw::EntryRow::builder().title("Port").input_purpose(gtk::InputPurpose::Digits).build();
    let user = adw::EntryRow::builder().title("User").build();
    let password = adw::PasswordEntryRow::builder().title("Password").build();
    let database = adw::EntryRow::builder().title("Database").build();
    let ssl_names: Vec<&str> = SslMode::ALL.iter().map(|m| m.display_name()).collect();
    let ssl = adw::ComboRow::builder().title("SSL").model(&gtk::StringList::new(&ssl_names)).build();
    let server = adw::PreferencesGroup::new();
    for row in [
        host.upcast_ref::<gtk::Widget>(),
        port.upcast_ref(),
        user.upcast_ref(),
        password.upcast_ref(),
        database.upcast_ref(),
        ssl.upcast_ref(),
    ] {
        server.add(row);
    }

    let page = adw::PreferencesPage::new();
    page.add(&general);
    page.add(&server);

    // Buttons
    let test_button = gtk::Button::with_label("Test");
    let test_spinner = gtk::Spinner::builder().visible(false).build();
    let test_status = gtk::Label::builder()
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .lines(2)
        .wrap(true)
        .build();
    test_status.add_css_class("caption");
    let save_button = gtk::Button::with_label("Save");
    let connect_button = gtk::Button::with_label("Connect");
    connect_button.add_css_class("suggested-action");
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_margin_top(12);
    buttons.set_margin_bottom(12);
    buttons.set_margin_start(12);
    buttons.set_margin_end(12);
    buttons.append(&test_button);
    buttons.append(&test_spinner);
    buttons.append(&test_status);
    buttons.append(&save_button);
    buttons.append(&connect_button);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&page));
    toolbar.add_bottom_bar(&buttons);
    dialog.set_child(Some(&toolbar));
    dialog.set_default_widget(Some(&connect_button));

    let form = Rc::new(Form {
        dialog: dialog.clone(),
        config: RefCell::new(config.clone()),
        kind_buttons,
        name,
        color_buttons,
        host,
        port,
        user,
        password,
        database,
        ssl,
        test_status,
        test_spinner,
        test_button: test_button.clone(),
        test_run: Cell::new(0),
        on_save: Box::new(on_save),
    });
    form.load(&config);

    if !is_new {
        let form = form.clone();
        let id = config.id;
        glib::spawn_future_local(async move {
            let password = gio::spawn_blocking(move || secrets::password(id)).await.unwrap_or_default();
            if form.password.text().is_empty() {
                form.password.set_text(&password);
            }
        });
    }

    for (kind, button) in &form.kind_buttons {
        let weak = Rc::downgrade(&form);
        let kind = *kind;
        button.connect_toggled(move |button| {
            if let Some(form) = weak.upgrade()
                && button.is_active()
            {
                form.kind_changed(kind);
            }
        });
    }
    for (color, button) in &form.color_buttons {
        let weak = Rc::downgrade(&form);
        let color = *color;
        button.connect_toggled(move |button| {
            if let Some(form) = weak.upgrade()
                && button.is_active()
            {
                form.config.borrow_mut().color = color;
            }
        });
    }
    let weak = Rc::downgrade(&form);
    test_button.connect_clicked(move |_| {
        if let Some(form) = weak.upgrade() {
            Form::test(&form);
        }
    });
    let weak = Rc::downgrade(&form);
    save_button.connect_clicked(move |_| {
        if let Some(form) = weak.upgrade() {
            form.save(false);
        }
    });
    let weak = Rc::downgrade(&form);
    connect_button.connect_clicked(move |_| {
        if let Some(form) = weak.upgrade() {
            form.save(true);
        }
    });
    for entry in [&form.name, &form.host, &form.port, &form.user, &form.database] {
        let weak = Rc::downgrade(&form);
        entry.connect_entry_activated(move |_| {
            if let Some(form) = weak.upgrade() {
                form.save(true);
            }
        });
    }
    let weak = Rc::downgrade(&form);
    form.password.connect_entry_activated(move |_| {
        if let Some(form) = weak.upgrade() {
            form.save(true);
        }
    });

    // The dialog owns the form for as long as it is open.
    let keep = RefCell::new(Some(form));
    dialog.connect_closed(move |_| {
        keep.borrow_mut().take();
    });
    dialog.present(Some(parent));
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|first| first.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

impl Form {
    fn load(&self, config: &ConnectionConfig) {
        for (kind, button) in &self.kind_buttons {
            button.set_active(*kind == config.kind);
        }
        for (color, button) in &self.color_buttons {
            button.set_active(*color == config.color);
        }
        self.name.set_text(&config.name);
        self.host.set_text(&config.host);
        self.port.set_text(&config.port.to_string());
        self.user.set_text(&config.user);
        self.database.set_text(&config.database);
        self.update_database_title(config.kind);
        let ssl = SslMode::ALL.iter().position(|m| *m == config.ssl_mode).unwrap_or(1);
        self.ssl.set_selected(ssl as u32);
    }

    fn update_database_title(&self, kind: DatabaseKind) {
        self.database.set_title(match kind {
            DatabaseKind::Postgres => "Database (default: postgres)",
            DatabaseKind::MySql => "Database (optional)",
        });
    }

    fn kind_changed(&self, kind: DatabaseKind) {
        let old = self.config.borrow().kind;
        if old == kind {
            return;
        }
        self.config.borrow_mut().kind = kind;
        if self.port.text() == old.default_port().to_string() {
            self.port.set_text(&kind.default_port().to_string());
        }
        if self.user.text() == old.default_user() {
            self.user.set_text(kind.default_user());
        }
        self.update_database_title(kind);
    }

    /// The config as currently entered in the form.
    fn current(&self) -> ConnectionConfig {
        let mut config = self.config.borrow().clone();
        config.name = self.name.text().trim().to_string();
        config.host = self.host.text().trim().to_string();
        if config.host.is_empty() {
            config.host = "127.0.0.1".into();
        }
        config.port = self.port.text().trim().parse().unwrap_or(config.kind.default_port());
        config.user = self.user.text().trim().to_string();
        config.database = self.database.text().trim().to_string();
        config.ssl_mode = SslMode::ALL.get(self.ssl.selected() as usize).copied().unwrap_or(SslMode::Prefer);
        config
    }

    fn test(this: &Rc<Self>) {
        let config = this.current();
        let password = this.password.text().to_string();
        let run = this.test_run.get() + 1;
        this.test_run.set(run);
        this.test_button.set_sensitive(false);
        this.test_spinner.set_visible(true);
        this.test_spinner.set_spinning(true);
        this.test_status.set_text("");
        let weak = Rc::downgrade(this);
        glib::spawn_future_local(async move {
            let session = Session::new(make_driver(&config, &password));
            let result = session
                .run(|driver| {
                    driver.connect()?;
                    let version = driver.server_version();
                    driver.close();
                    Ok(version)
                })
                .await;
            let Some(form) = weak.upgrade() else { return };
            if form.test_run.get() != run {
                return;
            }
            form.test_button.set_sensitive(true);
            form.test_spinner.set_visible(false);
            form.test_spinner.set_spinning(false);
            match result {
                Ok(version) => {
                    form.test_status.set_css_classes(&["caption", "success"]);
                    form.test_status.set_text(&format!("Connected · {version}"));
                    form.test_status.set_tooltip_text(None);
                }
                Err(error) => {
                    form.test_status.set_css_classes(&["caption", "error"]);
                    form.test_status.set_text(&error.message);
                    form.test_status.set_tooltip_text(Some(&error.message));
                }
            }
        });
    }

    fn save(&self, connect: bool) {
        let config = self.current();
        let password = self.password.text().to_string();
        self.dialog.close();
        (self.on_save)(config, password, connect);
    }
}
