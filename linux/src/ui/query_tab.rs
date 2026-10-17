use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::glib;

use super::editor::SqlEditor;
use super::grid::{DataGrid, GridCell, GridSource};
use super::widgets::{export_csv, icon_button};
use crate::db::Session;
use crate::models::query::QueryModel;

struct ResultSource(Rc<QueryModel>);

impl GridSource for ResultSource {
    fn columns(&self) -> Vec<String> {
        self.0.state.borrow().result.as_ref().map(|r| r.columns.clone()).unwrap_or_default()
    }

    fn row_count(&self) -> usize {
        self.0.state.borrow().result.as_ref().map_or(0, |r| r.rows.len())
    }

    fn cell(&self, row: usize, column: usize) -> GridCell {
        let state = self.0.state.borrow();
        let value = state.result.as_ref().and_then(|r| r.rows.get(row)?.get(column).cloned()).flatten();
        GridCell::plain(value)
    }
}

pub struct QueryTab {
    pub model: Rc<QueryModel>,
    pub root: gtk::Paned,
    editor: SqlEditor,
    run_button: gtk::Button,
    run_label: adw::ButtonContent,
    cancel_button: gtk::Button,
    spinner: gtk::Spinner,
    results: gtk::Stack,
    error_label: gtk::Label,
    placeholder_icon: gtk::Image,
    placeholder_label: gtk::Label,
    grid: DataGrid,
    message: gtk::Label,
    export_button: gtk::Button,
    shown_version: Cell<u64>,
    on_state_change: RefCell<Option<Box<dyn Fn()>>>,
}

impl QueryTab {
    pub fn new(title: String, session: Arc<Session>, sql: &str) -> Rc<Self> {
        let model = Rc::new(QueryModel::new(title, session));

        let editor = SqlEditor::new();
        editor.set_text(sql);

        let run_label = adw::ButtonContent::builder().icon_name("media-playback-start-symbolic").label("Run").build();
        let run_button = gtk::Button::builder().child(&run_label).tooltip_text("Run (Ctrl+Enter)").build();
        run_button.add_css_class("suggested-action");
        let cancel_button = gtk::Button::builder()
            .child(&adw::ButtonContent::builder().icon_name("media-playback-stop-symbolic").label("Cancel").build())
            .tooltip_text("Cancel (Ctrl+.)")
            .visible(false)
            .build();
        cancel_button.add_css_class("destructive-action");
        let spinner = gtk::Spinner::builder().visible(false).build();
        let hint = gtk::Label::new(Some("Ctrl+Enter run · Ctrl+Space complete"));
        hint.add_css_class("dim-label");
        hint.add_css_class("caption");
        hint.set_hexpand(true);
        hint.set_xalign(1.0);
        let run_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        run_bar.add_css_class("toolbar");
        run_bar.append(&run_button);
        run_bar.append(&cancel_button);
        run_bar.append(&spinner);
        run_bar.append(&hint);

        let top = gtk::Box::new(gtk::Orientation::Vertical, 0);
        top.append(editor.widget());
        top.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        top.append(&run_bar);

        let error_icon = gtk::Image::from_icon_name("dialog-error-symbolic");
        error_icon.add_css_class("error");
        error_icon.set_valign(gtk::Align::Start);
        let error_label =
            gtk::Label::builder().xalign(0.0).yalign(0.0).wrap(true).selectable(true).hexpand(true).build();
        error_label.add_css_class("monospace-small");
        let error_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        error_box.set_margin_top(12);
        error_box.set_margin_start(12);
        error_box.set_margin_end(12);
        error_box.append(&error_icon);
        error_box.append(&error_label);
        let error_page = gtk::ScrolledWindow::builder().child(&error_box).vexpand(true).build();

        let placeholder_icon = gtk::Image::from_icon_name("utilities-terminal-symbolic");
        placeholder_icon.set_pixel_size(32);
        placeholder_icon.add_css_class("dim-label");
        let placeholder_label = gtk::Label::new(Some("Write a query and press Ctrl+Enter"));
        placeholder_label.add_css_class("dim-label");
        let placeholder = gtk::Box::new(gtk::Orientation::Vertical, 8);
        placeholder.set_valign(gtk::Align::Center);
        placeholder.set_vexpand(true);
        placeholder.append(&placeholder_icon);
        placeholder.append(&placeholder_label);

        let grid = DataGrid::new(Rc::new(ResultSource(model.clone())), false);
        let results = gtk::Stack::new();
        results.add_named(&placeholder, Some("empty"));
        results.add_named(&error_page, Some("error"));
        results.add_named(grid.widget(), Some("grid"));

        let message = gtk::Label::builder().xalign(0.0).hexpand(true).build();
        message.add_css_class("dim-label");
        message.add_css_class("caption");
        message.add_css_class("numeric");
        let export_button = icon_button("document-save-as-symbolic", "Export results as CSV");
        export_button.set_visible(false);
        let status_bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        status_bar.add_css_class("bottom-bar");
        status_bar.append(&message);
        status_bar.append(&export_button);

        let bottom = gtk::Box::new(gtk::Orientation::Vertical, 0);
        bottom.append(&results);
        bottom.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        bottom.append(&status_bar);

        let root = gtk::Paned::builder()
            .orientation(gtk::Orientation::Vertical)
            .start_child(&top)
            .end_child(&bottom)
            .resize_start_child(true)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .position(280)
            .build();
        top.set_size_request(-1, 140);
        bottom.set_size_request(-1, 140);

        let tab = Rc::new(Self {
            model,
            root,
            editor,
            run_button,
            run_label,
            cancel_button,
            spinner,
            results,
            error_label,
            placeholder_icon,
            placeholder_label,
            grid,
            message,
            export_button,
            shown_version: Cell::new(0),
            on_state_change: RefCell::new(None),
        });
        Self::connect(&tab);
        tab.refresh();
        tab
    }

    fn connect(this: &Rc<Self>) {
        let weak = Rc::downgrade(this);
        this.model.set_on_change(move || {
            if let Some(tab) = weak.upgrade() {
                tab.refresh();
            }
        });
        let weak = Rc::downgrade(this);
        this.run_button.connect_clicked(move |_| {
            if let Some(tab) = weak.upgrade() {
                tab.run();
            }
        });
        let weak = Rc::downgrade(this);
        this.editor.connect_run(move || {
            if let Some(tab) = weak.upgrade() {
                tab.run();
            }
        });
        let weak = Rc::downgrade(this);
        this.cancel_button.connect_clicked(move |_| {
            if let Some(tab) = weak.upgrade() {
                tab.model.cancel();
            }
        });
        let weak = Rc::downgrade(this);
        this.export_button.connect_clicked(move |button| {
            if let Some(tab) = weak.upgrade()
                && let Some(result) = tab.model.state.borrow().result.clone()
            {
                export_csv(button.upcast_ref(), "query", result);
            }
        });
        let weak = Rc::downgrade(this);
        this.editor.buffer().connect_changed(move |_| {
            if let Some(tab) = weak.upgrade() {
                tab.refresh_run_button();
            }
        });
        let weak = Rc::downgrade(this);
        this.editor.buffer().connect_has_selection_notify(move |_| {
            if let Some(tab) = weak.upgrade() {
                tab.refresh_run_button();
            }
        });
    }

    pub fn set_on_state_change(&self, f: impl Fn() + 'static) {
        *self.on_state_change.borrow_mut() = Some(Box::new(f));
    }

    pub fn set_completion_words(&self, words: Vec<String>) {
        self.editor.set_completion_words(words);
    }

    pub fn focus(&self) {
        self.editor.grab_focus();
    }

    pub fn is_running(&self) -> bool {
        self.model.is_running()
    }

    /// Runs the selection when there is one, otherwise the whole editor.
    pub fn run(&self) {
        let selection = self.editor.selected_text();
        let sql = if selection.trim().is_empty() { self.editor.text() } else { selection };
        let model = self.model.clone();
        glib::spawn_future_local(async move { model.run(&sql).await });
    }

    pub fn cancel(&self) {
        self.model.cancel();
    }

    fn refresh_run_button(&self) {
        let has_selection = self.editor.buffer().has_selection();
        self.run_label.set_label(if has_selection { "Run Selection" } else { "Run" });
        self.run_button.set_sensitive(!self.editor.text().trim().is_empty());
    }

    fn refresh(&self) {
        let (running, version, has_result, error, message) = {
            let state = self.model.state.borrow();
            (state.is_running, state.result_version, state.result.is_some(), state.error.clone(), state.message.clone())
        };
        self.run_button.set_visible(!running);
        self.cancel_button.set_visible(running);
        self.spinner.set_visible(running);
        self.spinner.set_spinning(running);
        self.refresh_run_button();

        if self.shown_version.replace(version) != version {
            self.grid.reload();
        }
        if let Some(error) = &error {
            self.error_label.set_text(error);
            self.results.set_visible_child_name("error");
        } else if has_result {
            self.results.set_visible_child_name("grid");
        } else {
            let done = message.is_some();
            self.placeholder_icon.set_icon_name(Some(if done {
                "object-select-symbolic"
            } else {
                "utilities-terminal-symbolic"
            }));
            self.placeholder_label.set_text(if done {
                "Statement executed"
            } else {
                "Write a query and press Ctrl+Enter"
            });
            self.results.set_visible_child_name("empty");
        }
        self.message.set_text(message.as_deref().unwrap_or(""));
        self.export_button.set_visible(has_result);

        if let Some(callback) = self.on_state_change.borrow().as_ref() {
            callback();
        }
    }
}
