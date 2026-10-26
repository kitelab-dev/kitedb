//! Plain-text SQL editor with syntax colouring, auto-indent and keyword /
//! table-name completion (Ctrl+Space or Esc).

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::LazyLock;

use adw::prelude::*;
use gtk::{gdk, glib};
use regex::Regex;

use super::widgets::{detach_on_unrealize, popup_at};

pub const KEYWORDS: &str = "select from where and or not in is null like ilike between exists as on join left right \
    inner outer full cross natural using group by order asc desc having limit offset distinct union all intersect \
    except insert into values update set delete returning create table view index unique primary key foreign \
    references alter add drop column rename to truncate cascade restrict default constraint check if case when \
    then else end begin commit rollback transaction start with recursive explain analyze show databases tables \
    describe use grant revoke database schema sequence trigger function procedure true false count sum avg min \
    max coalesce nullif cast interval now current_timestamp current_date auto_increment serial bigserial int \
    integer bigint smallint varchar char text boolean bool date timestamp timestamptz time numeric decimal float \
    double real json jsonb uuid bytea blob enum lateral window over partition filter any some do nothing \
    conflict replace ignore duplicate";

struct Rule {
    tag: &'static str,
    regex: Regex,
}

/// Later rules win: strings and comments override keywords inside them.
static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    let keywords = KEYWORDS.split_whitespace().collect::<Vec<_>>().join("|");
    let rule = |tag, pattern: &str| Rule { tag, regex: Regex::new(pattern).expect("valid pattern") };
    vec![
        rule("keyword", &format!(r"(?i)\b(?:{keywords})\b")),
        rule("number", r"\b\d+(?:\.\d+)?\b"),
        rule("identifier", r#"`[^`]*`|"[^"]*""#),
        rule("string", r"'(?:[^'\\]|\\.|'')*'?"),
        rule("comment", r"--[^\n]*|/\*[\s\S]*?(?:\*/|$)"),
    ]
});

/// (tag, light colour, dark colour)
const COLORS: [(&str, &str, &str); 5] = [
    ("keyword", "#c01c7a", "#ff7eb6"),
    ("number", "#813d9c", "#c397ff"),
    ("identifier", "#0e7c86", "#5fd7d7"),
    ("string", "#c64600", "#ffa348"),
    ("comment", "#77767b", "#9a9996"),
];

/// Skip colouring for huge scripts to keep typing responsive.
const HIGHLIGHT_LIMIT: usize = 300_000;

struct Inner {
    scroller: gtk::ScrolledWindow,
    view: gtk::TextView,
    buffer: gtk::TextBuffer,
    popover: gtk::Popover,
    list: gtk::ListBox,
    completion_words: RefCell<Vec<String>>,
    highlight_pending: Cell<bool>,
    on_run: RefCell<Option<Box<dyn Fn()>>>,
}

#[derive(Clone)]
pub struct SqlEditor {
    inner: Rc<Inner>,
}

impl SqlEditor {
    pub fn new() -> Self {
        let buffer = gtk::TextBuffer::new(None);
        buffer.set_enable_undo(true);
        let view = gtk::TextView::with_buffer(&buffer);
        view.set_monospace(true);
        view.set_wrap_mode(gtk::WrapMode::None);
        view.set_left_margin(8);
        view.set_right_margin(8);
        view.set_top_margin(8);
        view.set_bottom_margin(8);
        view.add_css_class("sql-editor");
        let scroller = gtk::ScrolledWindow::builder().child(&view).hexpand(true).vexpand(true).build();

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Browse);
        list.add_css_class("completion-list");
        let list_scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(260)
            .min_content_width(220)
            .build();
        let popover = gtk::Popover::builder().child(&list_scroller).has_arrow(false).build();
        popover.set_position(gtk::PositionType::Bottom);
        popover.set_halign(gtk::Align::Start);

        let inner = Rc::new(Inner {
            scroller,
            view,
            buffer,
            popover,
            list,
            completion_words: RefCell::default(),
            highlight_pending: Cell::new(false),
            on_run: RefCell::new(None),
        });
        Inner::setup(&inner);
        Self { inner }
    }

    pub fn widget(&self) -> &gtk::ScrolledWindow {
        &self.inner.scroller
    }

    pub fn buffer(&self) -> &gtk::TextBuffer {
        &self.inner.buffer
    }

    pub fn text(&self) -> String {
        let (start, end) = self.inner.buffer.bounds();
        self.inner.buffer.text(&start, &end, false).to_string()
    }

    pub fn set_text(&self, text: &str) {
        self.inner.buffer.set_text(text);
        self.inner.buffer.place_cursor(&self.inner.buffer.end_iter());
    }

    pub fn selected_text(&self) -> String {
        self.inner
            .buffer
            .selection_bounds()
            .map(|(start, end)| self.inner.buffer.text(&start, &end, false).to_string())
            .unwrap_or_default()
    }

    pub fn set_completion_words(&self, words: Vec<String>) {
        *self.inner.completion_words.borrow_mut() = words;
    }

    /// Ctrl+Enter inside the editor.
    pub fn connect_run(&self, f: impl Fn() + 'static) {
        *self.inner.on_run.borrow_mut() = Some(Box::new(f));
    }

    pub fn grab_focus(&self) {
        self.inner.view.grab_focus();
    }
}

impl Inner {
    fn setup(this: &Rc<Self>) {
        for (name, _, _) in COLORS {
            this.buffer.create_tag(Some(name), &[]);
        }
        this.apply_colors();
        let style = adw::StyleManager::default();
        let weak = Rc::downgrade(this);
        style.connect_dark_notify(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.apply_colors();
            }
        });

        let weak = Rc::downgrade(this);
        this.buffer.connect_changed(move |_| {
            if let Some(inner) = weak.upgrade() {
                Inner::schedule_highlight(&inner);
            }
        });

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(this);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(inner) = weak.upgrade() else { return glib::Propagation::Proceed };
            inner.key_pressed(key, modifiers)
        });
        this.view.add_controller(keys);

        let weak = Rc::downgrade(this);
        this.list.connect_row_activated(move |_, row| {
            if let Some(inner) = weak.upgrade() {
                let word = row.widget_name().to_string();
                inner.popover.popdown();
                inner.insert_completion(&word);
            }
        });

        detach_on_unrealize(&this.popover, &this.view);
    }

    fn apply_colors(&self) {
        let dark = adw::StyleManager::default().is_dark();
        let table = self.buffer.tag_table();
        for (name, light, dark_color) in COLORS {
            if let Some(tag) = table.lookup(name) {
                tag.set_foreground(Some(if dark { dark_color } else { light }));
            }
        }
    }

    fn schedule_highlight(this: &Rc<Self>) {
        if this.highlight_pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(this);
        glib::idle_add_local_once(move || {
            if let Some(inner) = weak.upgrade() {
                inner.highlight_pending.set(false);
                inner.highlight();
            }
        });
    }

    fn highlight(&self) {
        let (start, end) = self.buffer.bounds();
        self.buffer.remove_all_tags(&start, &end);
        let text = self.buffer.text(&start, &end, false);
        if text.len() >= HIGHLIGHT_LIMIT {
            return;
        }
        // Regex works on byte offsets; the buffer on character offsets.
        let mut char_at = vec![0i32; text.len() + 1];
        let mut count = 0;
        for (byte, c) in text.char_indices() {
            char_at[byte..byte + c.len_utf8()].fill(count);
            count += 1;
        }
        char_at[text.len()] = count;

        let table = self.buffer.tag_table();
        for rule in RULES.iter() {
            let Some(tag) = table.lookup(rule.tag) else { continue };
            for found in rule.regex.find_iter(&text) {
                let from = self.buffer.iter_at_offset(char_at[found.start()]);
                let to = self.buffer.iter_at_offset(char_at[found.end()]);
                // Strings and comments replace colours inside them.
                if matches!(rule.tag, "string" | "comment") {
                    self.buffer.remove_all_tags(&from, &to);
                }
                self.buffer.apply_tag(&tag, &from, &to);
            }
        }
    }

    fn key_pressed(&self, key: gdk::Key, modifiers: gdk::ModifierType) -> glib::Propagation {
        let control = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
        let alt = modifiers.contains(gdk::ModifierType::ALT_MASK);
        match key {
            gdk::Key::Return | gdk::Key::KP_Enter if control => {
                if let Some(run) = self.on_run.borrow().as_ref() {
                    run();
                }
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter if !alt => {
                self.insert_newline();
                glib::Propagation::Stop
            }
            gdk::Key::space if control => {
                self.complete();
                glib::Propagation::Stop
            }
            gdk::Key::Escape if !control && !alt => {
                self.complete();
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    }

    /// Keeps the indentation of the current line.
    fn insert_newline(&self) {
        let buffer = &self.buffer;
        let cursor = buffer.iter_at_mark(&buffer.get_insert());
        let mut line_start = cursor;
        line_start.set_line_offset(0);
        let line = buffer.text(&line_start, &cursor, false);
        let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        buffer.begin_user_action();
        buffer.delete_selection(true, true);
        buffer.insert_at_cursor(&format!("\n{indent}"));
        buffer.end_user_action();
        self.view.scroll_mark_onscreen(&buffer.get_insert());
    }

    /// Start of the word that ends at the cursor.
    fn word_start(&self) -> gtk::TextIter {
        let cursor = self.buffer.iter_at_mark(&self.buffer.get_insert());
        let mut start = cursor;
        while start.backward_char() {
            if !(start.char().is_alphanumeric() || start.char() == '_') {
                start.forward_char();
                break;
            }
        }
        start
    }

    fn candidates(&self, partial: &str) -> Vec<String> {
        let partial = partial.to_lowercase();
        let words = self.completion_words.borrow();
        let keywords = KEYWORDS.split_whitespace().map(str::to_uppercase);
        let mut seen = HashSet::new();
        words
            .iter()
            .cloned()
            .chain(keywords)
            .filter(|word| word.to_lowercase().starts_with(&partial) && seen.insert(word.clone()))
            .take(200)
            .collect()
    }

    fn complete(&self) {
        let cursor = self.buffer.iter_at_mark(&self.buffer.get_insert());
        let start = self.word_start();
        let partial = self.buffer.text(&start, &cursor, false);
        if partial.is_empty() {
            return;
        }
        let candidates = self.candidates(&partial);
        match candidates.as_slice() {
            [] => self.view.error_bell(),
            [only] => self.insert_completion(only),
            _ => {
                while let Some(row) = self.list.row_at_index(0) {
                    self.list.remove(&row);
                }
                for word in &candidates {
                    let label = gtk::Label::builder().label(word).xalign(0.0).build();
                    let row = gtk::ListBoxRow::builder().child(&label).name(word).build();
                    self.list.append(&row);
                }
                self.list.select_row(self.list.row_at_index(0).as_ref());
                let location = self.view.iter_location(&start);
                let (x, y) = self.view.buffer_to_window_coords(
                    gtk::TextWindowType::Widget,
                    location.x(),
                    location.y() + location.height(),
                );
                popup_at(&self.popover, &self.view, x, y);
                if let Some(row) = self.list.row_at_index(0) {
                    row.grab_focus();
                }
            }
        }
    }

    fn insert_completion(&self, word: &str) {
        let mut start = self.word_start();
        let mut cursor = self.buffer.iter_at_mark(&self.buffer.get_insert());
        self.buffer.begin_user_action();
        self.buffer.delete(&mut start, &mut cursor);
        self.buffer.insert(&mut start, word);
        self.buffer.end_user_action();
        self.view.grab_focus();
    }
}
