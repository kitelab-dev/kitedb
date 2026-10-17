use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::db::QueryResult;
use crate::models::connection::{ConnectionColor, DatabaseKind};

pub const APP_ID: &str = "com.kitelab.KiteDB";
/// Release builds set `KITEDB_VERSION` (from the git tag); dev builds use Cargo's.
pub const VERSION: &str = match option_env!("KITEDB_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};
pub const WEBSITE: &str = "https://kitedb.dotly.mn";
pub const ISSUES: &str = "https://github.com/kitelab-dev/kitedb/issues";
/// Byl.mn checkout page for donations.
pub const DONATE: &str = "https://byl.mn/h/pay/eTdxkcjkQ69m";

const CSS: &str = r#"
.kind-badge {
    color: white;
    font-weight: 800;
    border-radius: 7px;
}
.kind-badge.mysql { background: linear-gradient(to bottom, #12889c, #00677a); }
.kind-badge.postgres { background: linear-gradient(to bottom, #4f7fd1, #2f5ea8); }

.color-dot { border-radius: 99px; min-width: 9px; min-height: 9px; }
.color-swatch { border-radius: 99px; min-width: 18px; min-height: 18px; padding: 0; }
.color-swatch.none { background: none; box-shadow: inset 0 0 0 1px alpha(currentColor, 0.4); }
.color-swatch:checked { box-shadow: inset 0 0 0 2px alpha(white, 0.9), 0 0 0 2px alpha(@window_fg_color, 0.75); }

.connection-badge { border-radius: 8px; padding: 2px 10px 2px 4px; }

.data-grid > listview > row > cell { padding: 0; }
.data-grid .grid-cell { padding: 2px 6px; min-height: 22px; }
.data-grid .null-value { opacity: 0.45; font-style: italic; }
.data-grid .deleted-value { opacity: 0.55; }
.data-grid .grid-cell.row-inserted { background-color: alpha(@success_color, 0.16); }
.data-grid .grid-cell.row-deleted { background-color: alpha(@error_color, 0.20); }
.data-grid .grid-cell.cell-edited { background-color: alpha(@warning_color, 0.28); }
.data-grid entry.grid-editor { min-height: 0; padding: 0 4px; border-radius: 0; }

.error-banner { background-color: alpha(@error_color, 0.10); padding: 6px 10px; }
.error-banner label { font-family: monospace; font-size: 0.9em; }

.bottom-bar { padding: 3px 8px; min-height: 30px; }
.bottom-bar button { min-height: 24px; padding: 0 8px; }
.welcome-side { background-color: alpha(currentColor, 0.04); }
.sql-editor { font-size: 10.5pt; }
.monospace-small { font-family: monospace; font-size: 0.9em; }
"#;

pub fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CSS);
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
    let colors: String = ConnectionColor::ALL
        .iter()
        .filter_map(|c| c.hex().map(|hex| format!(".tint-{} {{ background-color: {hex}; color: {hex}; }}\n", c.name())))
        .chain(ConnectionColor::ALL.iter().filter_map(|c| {
            c.hex().map(|hex| {
                format!(
                    ".connection-badge.tint-{} {{ background-color: alpha({hex}, 0.18); color: inherit; }}\n",
                    c.name()
                )
            })
        }))
        .collect();
    let tints = gtk::CssProvider::new();
    tints.load_from_string(&colors);
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &tints, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}

/// The app icon, embedded so it shows even when not installed.
pub fn app_icon() -> gdk::Texture {
    let bytes =
        glib::Bytes::from_static(include_bytes!("../../data/icons/hicolor/256x256/apps/com.kitelab.KiteDB.png"));
    gdk::Texture::from_bytes(&bytes).expect("valid app icon")
}

pub fn kind_badge(kind: DatabaseKind, size: i32) -> gtk::Label {
    let label = gtk::Label::new(Some(kind.badge_text()));
    label.add_css_class("kind-badge");
    label.add_css_class(match kind {
        DatabaseKind::MySql => "mysql",
        DatabaseKind::Postgres => "postgres",
    });
    label.set_size_request(size, size);
    label.set_valign(gtk::Align::Center);
    label.set_halign(gtk::Align::Center);
    let attributes = gtk::pango::AttrList::new();
    attributes.insert(gtk::pango::AttrSize::new_size_absolute(size * gtk::pango::SCALE * 2 / 5));
    label.set_attributes(Some(&attributes));
    label
}

pub fn color_dot(color: ConnectionColor) -> Option<gtk::Widget> {
    color.hex()?;
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("color-dot");
    dot.add_css_class(&format!("tint-{}", color.name()));
    dot.set_valign(gtk::Align::Center);
    Some(dot.upcast())
}

pub fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.set_tooltip_text(Some(tooltip));
    button.add_css_class("flat");
    button
}

/// A dismissable red strip showing a (selectable) error message.
pub struct ErrorBanner {
    pub root: gtk::Box,
    label: gtk::Label,
}

impl ErrorBanner {
    pub fn new(on_close: impl Fn() + 'static) -> Rc<Self> {
        let icon = gtk::Image::from_icon_name("dialog-error-symbolic");
        icon.add_css_class("error");
        icon.set_valign(gtk::Align::Center);
        let label = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .lines(4)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .selectable(true)
            .hexpand(true)
            .build();
        let close = icon_button("window-close-symbolic", "Dismiss");
        close.set_valign(gtk::Align::Center);
        close.connect_clicked(move |_| on_close());
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        root.add_css_class("error-banner");
        root.set_visible(false);
        root.append(&icon);
        root.append(&label);
        root.append(&close);
        Rc::new(Self { root, label })
    }

    pub fn set_message(&self, message: Option<&str>) {
        if let Some(message) = message {
            self.label.set_text(message);
        }
        self.root.set_visible(message.is_some());
    }
}

pub fn open_uri(parent: Option<&impl IsA<gtk::Window>>, uri: &str) {
    let launcher = gtk::UriLauncher::new(uri);
    launcher.launch(parent, gio::Cancellable::NONE, |_| {});
}

fn csv_escape(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

pub fn csv(columns: &[String], rows: &[Vec<Option<String>>]) -> String {
    let mut lines = vec![columns.iter().map(|c| csv_escape(c)).collect::<Vec<_>>().join(",")];
    for row in rows {
        lines.push(row.iter().map(|v| v.as_deref().map(csv_escape).unwrap_or_default()).collect::<Vec<_>>().join(","));
    }
    lines.join("\n") + "\n"
}

/// Asks for a file name and writes the rows as CSV.
pub fn export_csv(parent: &gtk::Widget, name: &str, result: QueryResult) {
    let window = parent.root().and_downcast::<gtk::Window>();
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("CSV files"));
    filter.add_suffix("csv");
    filter.add_mime_type("text/csv");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title("Export as CSV")
        .initial_name(format!("{name}.csv"))
        .filters(&filters)
        .modal(true)
        .build();
    glib::spawn_future_local(async move {
        let Ok(file) = dialog.save_future(window.as_ref()).await else { return };
        let Some(path) = file.path() else { return };
        let text = csv(&result.columns, &result.rows);
        if let Err(error) = std::fs::write(&path, text) {
            let alert = adw::AlertDialog::new(Some("Export Failed"), Some(&error.to_string()));
            alert.add_response("ok", "OK");
            alert.present(window.as_ref());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::csv;

    #[test]
    fn writes_csv() {
        let rows = vec![vec![Some("a,b".into()), None, Some("say \"hi\"".into())]];
        assert_eq!(csv(&["x".into(), "y".into(), "z".into()], &rows), "x,y,z\n\"a,b\",,\"say \"\"hi\"\"\"\n");
    }
}
