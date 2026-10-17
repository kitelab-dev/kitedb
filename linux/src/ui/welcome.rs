//! The connections window: saved connections, search and the side panel.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use uuid::Uuid;

use super::widgets::{DONATE, VERSION, app_icon, color_dot, kind_badge, open_uri};
use super::{App, connection_form};
use crate::models::connection::ConnectionConfig;

pub struct Welcome {
    app: Weak<App>,
    pub window: adw::ApplicationWindow,
    list: gtk::ListBox,
    search: gtk::SearchEntry,
    stack: gtk::Stack,
    menu: gtk::PopoverMenu,
    rows: RefCell<Vec<(gtk::ListBoxRow, Uuid)>>,
    menu_target: RefCell<Option<Uuid>>,
}

impl Welcome {
    pub fn new(app: &Rc<App>) -> Rc<Self> {
        let window = adw::ApplicationWindow::builder()
            .application(&app.app)
            .title("KiteDB")
            .default_width(820)
            .default_height(560)
            .build();
        window.set_size_request(700, 420);

        // Side panel
        let icon = gtk::Image::from_paintable(Some(&app_icon()));
        icon.set_pixel_size(96);
        icon.set_margin_top(12);
        let name = gtk::Label::new(Some("KiteDB"));
        name.add_css_class("title-2");
        let tagline = gtk::Label::new(Some("MySQL & PostgreSQL client"));
        tagline.add_css_class("dim-label");
        let new_button = gtk::Button::builder()
            .child(&adw::ButtonContent::builder().icon_name("list-add-symbolic").label("New Connection…").build())
            .action_name("win.new-connection")
            .build();
        new_button.add_css_class("pill");
        new_button.add_css_class("suggested-action");
        let donate = gtk::Button::builder()
            .child(&adw::ButtonContent::builder().icon_name("emote-love-symbolic").label("Support KiteDB").build())
            .tooltip_text("Donate via Byl")
            .halign(gtk::Align::Center)
            .build();
        donate.add_css_class("flat");
        let version = gtk::Label::new(Some(&format!("Version {}", VERSION)));
        version.add_css_class("caption");
        version.add_css_class("dim-label");
        let side = gtk::Box::new(gtk::Orientation::Vertical, 10);
        side.set_margin_start(24);
        side.set_margin_end(24);
        side.set_margin_bottom(16);
        side.append(&icon);
        side.append(&name);
        side.append(&tagline);
        side.append(&gtk::Box::builder().vexpand(true).build());
        side.append(&new_button);
        side.append(&donate);
        side.append(&version);
        let side_header = adw::HeaderBar::builder().show_end_title_buttons(false).show_title(false).build();
        let side_view = adw::ToolbarView::new();
        side_view.add_css_class("welcome-side");
        side_view.add_top_bar(&side_header);
        side_view.set_content(Some(&side));
        side_view.set_size_request(250, -1);

        // Connection list
        let search = gtk::SearchEntry::builder().placeholder_text("Search connections").hexpand(true).build();
        let list = gtk::ListBox::new();
        list.add_css_class("navigation-sidebar");
        list.set_activate_on_single_click(false);
        let scroller =
            gtk::ScrolledWindow::builder().child(&list).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        let empty_button = gtk::Button::builder()
            .label("New Connection…")
            .action_name("win.new-connection")
            .halign(gtk::Align::Center)
            .build();
        empty_button.add_css_class("pill");
        let empty = adw::StatusPage::builder()
            .icon_name("network-server-symbolic")
            .title("No Connections")
            .description("Create a connection to a MySQL or PostgreSQL server.")
            .child(&empty_button)
            .build();
        let stack = gtk::Stack::new();
        stack.add_named(&scroller, Some("list"));
        stack.add_named(&empty, Some("empty"));

        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&{
                let menu = gio::Menu::new();
                menu.append_section(None, &super::app_menu_section());
                menu
            })
            .tooltip_text("Main Menu")
            .primary(true)
            .build();
        let header = adw::HeaderBar::builder().show_start_title_buttons(false).title_widget(&search).build();
        header.pack_end(&menu_button);
        let main_view = adw::ToolbarView::new();
        main_view.add_top_bar(&header);
        main_view.set_content(Some(&stack));
        main_view.set_hexpand(true);

        let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        content.append(&side_view);
        content.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        content.append(&main_view);
        window.set_content(Some(&content));

        let menu = gtk::PopoverMenu::from_model(None::<&gio::MenuModel>);
        menu.set_has_arrow(false);
        menu.set_halign(gtk::Align::Start);
        menu.set_parent(&list);

        let welcome = Rc::new(Self {
            app: Rc::downgrade(app),
            window,
            list,
            search,
            stack,
            menu,
            rows: RefCell::default(),
            menu_target: RefCell::new(None),
        });
        Self::connect(&welcome, &donate);
        welcome.rebuild();
        welcome
    }

    fn app(&self) -> Option<Rc<App>> {
        self.app.upgrade()
    }

    fn connect(this: &Rc<Self>, donate: &gtk::Button) {
        let window = this.window.clone();
        donate.connect_clicked(move |_| open_uri(Some(&window), DONATE));

        let weak = Rc::downgrade(this);
        this.search.connect_search_changed(move |_| {
            if let Some(welcome) = weak.upgrade() {
                welcome.rebuild();
            }
        });
        let weak = Rc::downgrade(this);
        this.list.connect_row_activated(move |_, row| {
            if let Some(welcome) = weak.upgrade()
                && let Some(id) = welcome.id_for(row)
            {
                welcome.open(id);
            }
        });

        let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        let weak = Rc::downgrade(this);
        click.connect_pressed(move |_, _, x, y| {
            let Some(welcome) = weak.upgrade() else { return };
            let Some(row) = welcome.list.row_at_y(y as i32) else { return };
            let Some(id) = welcome.id_for(&row) else { return };
            welcome.list.select_row(Some(&row));
            *welcome.menu_target.borrow_mut() = Some(id);
            welcome.menu.set_menu_model(Some(&row_menu()));
            welcome.menu.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            welcome.menu.popup();
        });
        this.list.add_controller(click);

        let list = this.list.clone();
        let menu = this.menu.clone();
        list.connect_destroy(move |_| menu.unparent());

        // Window actions; row actions work on the right-clicked or selected row.
        let add = |name: &str, handler: fn(&Rc<Welcome>, Option<Uuid>)| {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(this);
            action.connect_activate(move |_, _| {
                let Some(welcome) = weak.upgrade() else { return };
                let target = welcome.menu_target.borrow_mut().take().or_else(|| welcome.selected_id());
                handler(&welcome, target);
            });
            this.window.add_action(&action);
        };
        add("new-connection", |welcome, _| welcome.edit(ConnectionConfig::default(), true));
        add("connect", |welcome, id| {
            if let Some(id) = id {
                welcome.open(id);
            }
        });
        add("edit", |welcome, id| {
            let config = id.and_then(|id| welcome.app()?.store.borrow().connection(id).cloned());
            if let Some(config) = config {
                welcome.edit(config, false);
            }
        });
        add("duplicate", |welcome, id| {
            if let (Some(id), Some(app)) = (id, welcome.app()) {
                App::duplicate_connection(&app, id);
            }
        });
        add("move-up", |welcome, id| welcome.shift(id, -1));
        add("move-down", |welcome, id| welcome.shift(id, 1));
        add("delete", |welcome, id| {
            if let Some(id) = id {
                Welcome::confirm_delete(welcome, id);
            }
        });
    }

    fn id_for(&self, row: &gtk::ListBoxRow) -> Option<Uuid> {
        self.rows.borrow().iter().find(|(r, _)| r == row).map(|(_, id)| *id)
    }

    fn selected_id(&self) -> Option<Uuid> {
        self.list.selected_row().and_then(|row| self.id_for(&row))
    }

    fn open(&self, id: Uuid) {
        if let Some(app) = self.app() {
            App::open_workspace(&app, id);
        }
    }

    fn shift(&self, id: Option<Uuid>, offset: isize) {
        let (Some(id), Some(app)) = (id, self.app()) else { return };
        app.store.borrow_mut().shift(id, offset);
        self.rebuild();
        self.select(id);
    }

    pub fn select(&self, id: Uuid) {
        let row = self.rows.borrow().iter().find(|(_, i)| *i == id).map(|(row, _)| row.clone());
        if let Some(row) = row {
            self.list.select_row(Some(&row));
            row.grab_focus();
        }
    }

    pub fn edit(self: &Rc<Self>, config: ConnectionConfig, is_new: bool) {
        let weak = Rc::downgrade(self);
        connection_form::present(&self.window, config, is_new, move |config, password, connect| {
            let Some(welcome) = weak.upgrade() else { return };
            let Some(app) = welcome.app() else { return };
            let id = config.id;
            App::save_connection(&app, config, password, connect);
            welcome.rebuild();
            welcome.select(id);
        });
    }

    fn confirm_delete(this: &Rc<Self>, id: Uuid) {
        let Some(app) = this.app() else { return };
        let Some(name) = app.store.borrow().connection(id).map(|c| c.display_name().to_string()) else { return };
        let dialog = adw::AlertDialog::new(
            Some(&format!("Delete “{name}”?")),
            Some("The saved connection and its password will be removed."),
        );
        dialog.add_responses(&[("cancel", "Cancel"), ("delete", "Delete")]);
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let this = this.clone();
        glib::spawn_future_local(async move {
            if dialog.choose_future(Some(&this.window)).await == "delete" {
                App::delete_connection(&app, id);
                this.rebuild();
            }
        });
    }

    pub fn rebuild(&self) {
        let Some(app) = self.app() else { return };
        let selected = self.selected_id();
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        let query = self.search.text().trim().to_lowercase();
        let store = app.store.borrow();
        let mut rows = Vec::new();
        for config in store.connections() {
            if !query.is_empty()
                && !config.display_name().to_lowercase().contains(&query)
                && !config.summary().to_lowercase().contains(&query)
            {
                continue;
            }
            let row = connection_row(config);
            self.list.append(&row);
            rows.push((row, config.id));
        }
        self.stack.set_visible_child_name(if store.connections().is_empty() { "empty" } else { "list" });
        drop(store);
        *self.rows.borrow_mut() = rows;
        if let Some(id) = selected {
            self.select(id);
        }
    }
}

fn connection_row(config: &ConnectionConfig) -> gtk::ListBoxRow {
    let name = gtk::Label::builder()
        .label(config.display_name())
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    name.add_css_class("heading");
    let summary =
        gtk::Label::builder().label(config.summary()).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build();
    summary.add_css_class("caption");
    summary.add_css_class("dim-label");
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
    labels.set_hexpand(true);
    labels.set_valign(gtk::Align::Center);
    labels.append(&name);
    labels.append(&summary);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    content.set_margin_top(4);
    content.set_margin_bottom(4);
    content.append(&kind_badge(config.kind, 32));
    content.append(&labels);
    if let Some(dot) = color_dot(config.color) {
        content.append(&dot);
    }
    gtk::ListBoxRow::builder().child(&content).build()
}

fn row_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let connect = gio::Menu::new();
    connect.append(Some("Connect"), Some("win.connect"));
    menu.append_section(None, &connect);
    let edit = gio::Menu::new();
    edit.append(Some("Edit…"), Some("win.edit"));
    edit.append(Some("Duplicate"), Some("win.duplicate"));
    edit.append(Some("Move Up"), Some("win.move-up"));
    edit.append(Some("Move Down"), Some("win.move-down"));
    menu.append_section(None, &edit);
    let delete = gio::Menu::new();
    delete.append(Some("Delete…"), Some("win.delete"));
    menu.append_section(None, &delete);
    menu
}
