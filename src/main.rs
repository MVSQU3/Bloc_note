#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

mod doc;
mod lists;

use doc::{Doc, Enc, Eol};
use eframe::egui::{
    self, text::{CCursor, CCursorRange}, text_edit::TextEditState, FontId, Id, Key, KeyboardShortcut,
    Modifiers, RichText,
};
use lists::ListKind;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const APP_NAME: &str = "Bloc_note";

// ───────────────────────────── Réglages & session ─────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct Settings {
    wrap: bool,
    zoom: f32,
    status_bar: bool,
    theme: Theme,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
enum Theme {
    System,
    Light,
    Dark,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { wrap: true, zoom: 1.0, status_bar: true, theme: Theme::System }
    }
}

#[derive(Serialize, Deserialize)]
struct TabSave {
    path: Option<PathBuf>,
    /// Contenu conservé pour les onglets modifiés ou sans titre.
    text: Option<String>,
    enc: Enc,
    eol: Eol,
    untitled_n: usize,
}

#[derive(Serialize, Deserialize, Default)]
struct Session {
    tabs: Vec<TabSave>,
    active: usize,
    settings: Settings,
}

fn session_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join(APP_NAME).join("session.json"))
}

// ───────────────────────────── Recherche ─────────────────────────────

#[derive(Default)]
struct FindState {
    open: bool,
    replace: bool,
    query: String,
    repl: String,
    case: bool,
    word: bool,
    regex: bool,
    msg: String,
    focus: bool,
}

impl FindState {
    fn build(&self) -> Result<regex::Regex, String> {
        if self.query.is_empty() {
            return Err(String::new());
        }
        let mut pat = if self.regex { self.query.clone() } else { regex::escape(&self.query) };
        if self.word {
            pat = format!(r"\b(?:{pat})\b");
        }
        regex::RegexBuilder::new(&pat)
            .case_insensitive(!self.case)
            .multi_line(true)
            .build()
            .map_err(|e| format!("Expression invalide : {e}"))
    }
}

// ───────────────────────────── Utilitaires de texte ─────────────────────────────

fn byte_of(s: &str, ci: usize) -> usize {
    s.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(s.len())
}

fn char_of(s: &str, bi: usize) -> usize {
    s[..bi].chars().count()
}

/// Octets (début, fin) de la ligne contenant le caractère `ci`.
fn line_bounds(s: &str, ci: usize) -> (usize, usize) {
    let b = byte_of(s, ci);
    let start = s[..b].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let end = s[b..].find('\n').map(|i| b + i).unwrap_or(s.len());
    (start, end)
}

fn set_cursor(ctx: &egui::Context, id: Id, a: usize, b: usize) {
    let mut st = TextEditState::load(ctx, id).unwrap_or_default();
    st.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
    st.store(ctx, id);
}

fn get_cursor(ctx: &egui::Context, id: Id) -> Option<(usize, usize)> {
    let r = TextEditState::load(ctx, id)?.cursor.char_range()?;
    Some((r.primary.index.0, r.secondary.index.0))
}

fn sorted(a: usize, b: usize) -> (usize, usize) {
    if a <= b { (a, b) } else { (b, a) }
}

// ───────────────────────────── Application ─────────────────────────────

struct App {
    docs: Vec<Doc>,
    active: usize,
    next_id: u64,
    next_untitled: usize,
    settings: Settings,
    find: FindState,
    goto: Option<String>,
    confirm_close: Option<usize>,
    error: Option<String>,
    pos: (usize, usize),
    sel_chars: usize,
    total_chars: usize,
    select: Option<(usize, usize)>,
    focus_editor: bool,
    scroll_to_cursor: bool,
    dirty: bool,
    last_save: Instant,
    clipboard: Option<arboard::Clipboard>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_fonts(&cc.egui_ctx);
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);

        let mut app = App {
            docs: vec![],
            active: 0,
            next_id: 1,
            next_untitled: 1,
            settings: Settings::default(),
            find: FindState::default(),
            goto: None,
            confirm_close: None,
            error: None,
            pos: (1, 1),
            sel_chars: 0,
            total_chars: 0,
            select: None,
            focus_editor: true,
            scroll_to_cursor: false,
            dirty: false,
            last_save: Instant::now(),
            clipboard: arboard::Clipboard::new().ok(),
        };
        app.restore_session();
        for arg in std::env::args().skip(1) {
            app.open_path(Path::new(&arg));
        }
        if app.docs.is_empty() {
            app.new_tab();
        }
        app.active = app.active.min(app.docs.len() - 1);
        app.apply_theme(&cc.egui_ctx);
        app
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        ctx.set_theme(match self.settings.theme {
            Theme::System => egui::ThemePreference::System,
            Theme::Light => egui::ThemePreference::Light,
            Theme::Dark => egui::ThemePreference::Dark,
        });
    }

    // ── session ──

    fn restore_session(&mut self) {
        let Some(p) = session_path() else { return };
        let Ok(data) = std::fs::read_to_string(p) else { return };
        let Ok(s) = serde_json::from_str::<Session>(&data) else { return };
        self.settings = s.settings;
        for t in s.tabs {
            let id = self.next_id;
            let mut doc = match &t.path {
                Some(p) if p.exists() => match Doc::open(id, p) {
                    Ok(d) => d,
                    Err(_) => continue,
                },
                Some(p) => {
                    let mut d = Doc::new_empty(id, 0);
                    d.path = Some(p.clone());
                    d
                }
                None => Doc::new_empty(id, t.untitled_n),
            };
            if let Some(text) = t.text {
                doc.text = text;
                doc.enc = t.enc;
                doc.eol = t.eol;
                doc.refresh_modified();
                if doc.path.is_none() && doc.text.is_empty() {
                    // onglet vide sans titre : rien à signaler
                }
            }
            self.next_id += 1;
            self.next_untitled = self.next_untitled.max(doc.untitled_n + 1);
            self.docs.push(doc);
        }
        self.active = s.active;
    }

    fn save_session(&mut self) {
        let Some(p) = session_path() else { return };
        let tabs = self
            .docs
            .iter()
            .map(|d| TabSave {
                path: d.path.clone(),
                text: (d.modified || d.path.is_none()).then(|| d.text.clone()),
                enc: d.enc,
                eol: d.eol,
                untitled_n: d.untitled_n,
            })
            .collect();
        let s = Session { tabs, active: self.active, settings: self.settings.clone() };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string(&s) {
            let _ = std::fs::write(p, json);
        }
        self.dirty = false;
        self.last_save = Instant::now();
    }

    // ── onglets & fichiers ──

    fn new_tab(&mut self) {
        let doc = Doc::new_empty(self.next_id, self.next_untitled);
        self.next_id += 1;
        self.next_untitled += 1;
        self.docs.push(doc);
        self.active = self.docs.len() - 1;
        self.focus_editor = true;
        self.dirty = true;
    }

    fn open_path(&mut self, path: &Path) {
        if let Some(i) = self.docs.iter().position(|d| d.path.as_deref() == Some(path)) {
            self.active = i;
            return;
        }
        match Doc::open(self.next_id, path) {
            Ok(doc) => {
                self.next_id += 1;
                // Remplace un onglet vide et vierge
                if self.docs.len() == 1 && self.docs[0].path.is_none() && self.docs[0].text.is_empty() {
                    self.docs.clear();
                }
                self.docs.push(doc);
                self.active = self.docs.len() - 1;
                self.focus_editor = true;
                self.dirty = true;
            }
            Err(e) => self.error = Some(format!("Impossible d'ouvrir « {} » :\n{e}", path.display())),
        }
    }

    fn open_dialog(&mut self) {
        if let Some(files) = rfd::FileDialog::new()
            .add_filter("Documents texte (*.txt)", &["txt"])
            .add_filter("Tous les fichiers", &["*"])
            .pick_files()
        {
            for f in files {
                self.open_path(&f);
            }
        }
    }

    fn save(&mut self, i: usize) -> bool {
        match self.docs[i].path.clone() {
            Some(p) => self.write(i, &p),
            None => self.save_as(i),
        }
    }

    fn save_as(&mut self, i: usize) -> bool {
        let mut dlg = rfd::FileDialog::new()
            .add_filter("Documents texte (*.txt)", &["txt"])
            .add_filter("Tous les fichiers", &["*"])
            .set_file_name(format!("{}.txt", self.docs[i].title().trim_end_matches(".txt")));
        if let Some(dir) = self.docs[i].path.as_ref().and_then(|p| p.parent()) {
            dlg = dlg.set_directory(dir);
        }
        match dlg.save_file() {
            Some(p) => self.write(i, &p),
            None => false,
        }
    }

    fn write(&mut self, i: usize, p: &Path) -> bool {
        match self.docs[i].save_to(p) {
            Ok(()) => {
                self.dirty = true;
                true
            }
            Err(e) => {
                self.error = Some(format!("Échec de l'enregistrement :\n{e}"));
                false
            }
        }
    }

    fn save_all(&mut self) {
        for i in 0..self.docs.len() {
            if self.docs[i].modified && !self.save(i) {
                break;
            }
        }
    }

    fn request_close(&mut self, i: usize) {
        if self.docs[i].modified {
            self.confirm_close = Some(i);
            self.active = i;
        } else {
            self.close_now(i);
        }
    }

    fn close_now(&mut self, i: usize) {
        self.docs.remove(i);
        if self.docs.is_empty() {
            self.new_tab();
        }
        self.active = self.active.min(self.docs.len() - 1);
        if i < self.active {
            self.active -= 1;
        }
        self.focus_editor = true;
        self.dirty = true;
    }

    // ── édition ──

    fn editor_id(&self) -> Id {
        Id::new(("editeur", self.docs[self.active].id))
    }

    /// Remplace les caractères [a,b) par `with` et place le curseur/la sélection.
    fn edit(&mut self, ctx: &egui::Context, a: usize, b: usize, with: &str, sel: (usize, usize)) {
        let id = self.editor_id();
        let doc = &mut self.docs[self.active];
        let (ba, bb) = (byte_of(&doc.text, a), byte_of(&doc.text, b));
        doc.text.replace_range(ba..bb, with);
        doc.refresh_modified();
        set_cursor(ctx, id, sel.0, sel.1);
        self.focus_editor = true;
        self.scroll_to_cursor = true;
        self.dirty = true;
    }

    fn cursor(&self, ctx: &egui::Context) -> (usize, usize) {
        let (a, b) = get_cursor(ctx, self.editor_id()).unwrap_or((0, 0));
        sorted(a, b)
    }

    fn selected_text(&self, ctx: &egui::Context) -> String {
        let (a, b) = self.cursor(ctx);
        let t = &self.docs[self.active].text;
        t[byte_of(t, a)..byte_of(t, b)].to_string()
    }

    fn copy(&mut self, ctx: &egui::Context) {
        let s = self.selected_text(ctx);
        if !s.is_empty() {
            ctx.copy_text(s);
        }
    }

    fn cut(&mut self, ctx: &egui::Context) {
        let (a, b) = self.cursor(ctx);
        if a != b {
            self.copy(ctx);
            self.edit(ctx, a, b, "", (a, a));
        }
    }

    fn paste(&mut self, ctx: &egui::Context) {
        let Some(text) = self.clipboard.as_mut().and_then(|c| c.get_text().ok()) else { return };
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let (a, b) = self.cursor(ctx);
        let n = text.chars().count();
        self.edit(ctx, a, b, &text, (a + n, a + n));
    }

    fn select_all(&mut self, ctx: &egui::Context) {
        let n = self.docs[self.active].text.chars().count();
        set_cursor(ctx, self.editor_id(), 0, n);
        self.focus_editor = true;
    }

    fn undo_redo(&mut self, ctx: &egui::Context, redo: bool) {
        let id = self.editor_id();
        let mut st = TextEditState::load(ctx, id).unwrap_or_default();
        let mut undoer = st.undoer();
        let doc = &mut self.docs[self.active];
        let cur = st.cursor.char_range().unwrap_or_default();
        let state = (cur, doc.text.clone());
        let res = if redo { undoer.redo(&state).cloned() } else { undoer.undo(&state).cloned() };
        if let Some((range, text)) = res {
            doc.text = text;
            doc.refresh_modified();
            st.cursor.set_char_range(Some(range));
            self.dirty = true;
        }
        st.set_undoer(undoer);
        st.store(ctx, id);
        self.focus_editor = true;
    }

    fn insert_datetime(&mut self, ctx: &egui::Context) {
        let s = chrono::Local::now().format("%H:%M %d/%m/%Y").to_string();
        let (a, b) = self.cursor(ctx);
        let n = s.chars().count();
        self.edit(ctx, a, b, &s, (a + n, a + n));
    }

    /// Applique une transformation sur les lignes touchées par la sélection.
    fn transform_lines(&mut self, ctx: &egui::Context, f: impl Fn(&[String]) -> Vec<String>) {
        let (a, mut b) = self.cursor(ctx);
        let text = &self.docs[self.active].text;
        // Une sélection qui se termine en début de ligne n'inclut pas cette ligne.
        if b > a && text[..byte_of(text, b)].ends_with('\n') {
            b -= 1;
        }
        let (start, _) = line_bounds(text, a);
        let (_, end) = line_bounds(text, b);
        let lines: Vec<String> = text[start..end].split('\n').map(String::from).collect();
        let new = f(&lines).join("\n");
        let (cs, ce) = (char_of(text, start), char_of(text, end));
        let n = new.chars().count();
        let sel = if lines.len() > 1 || a != b { (cs, cs + n) } else { (cs + n, cs + n) };
        self.edit(ctx, cs, ce, &new, sel);
    }

    fn apply_list(&mut self, ctx: &egui::Context, kind: ListKind) {
        self.transform_lines(ctx, |l| lists::apply(l, kind));
    }

    /// Entrée / Tab sur une ligne de liste (appelé avant l'affichage du TextEdit).
    fn handle_list_keys(&mut self, ctx: &egui::Context) {
        let id = self.editor_id();
        if !ctx.memory(|m| m.has_focus(id)) {
            return;
        }
        let (a, b) = self.cursor(ctx);
        let text = &self.docs[self.active].text;
        let (ls, le) = line_bounds(text, a);
        let line = &text[ls..le];
        let item = lists::parse(line);

        if let Some(item) = item.filter(|_| a == b) {
            let col_b = byte_of(text, a) - ls;
            if col_b >= item.prefix_len || item.empty {
                if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
                    let cs = char_of(text, ls);
                    if item.empty {
                        // Entrée sur un élément vide : on termine la liste.
                        let ce = char_of(text, le);
                        self.edit(ctx, cs, ce, "", (cs, cs));
                    } else {
                        let ins = format!("\n{}", lists::next_marker(&item));
                        let n = ins.chars().count();
                        self.edit(ctx, a, a, &ins, (a + n, a + n));
                    }
                    return;
                }
            }
        }
        let is_list_line = item_is_list(self, a, b);
        if (a != b || is_list_line) && ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Tab)) {
            self.transform_lines(ctx, lists::outdent);
        } else if (a != b || is_list_line) && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab)) {
            self.transform_lines(ctx, lists::indent);
        }
    }

    // ── recherche ──

    fn open_find(&mut self, ctx: &egui::Context, replace: bool) {
        let sel = self.selected_text(ctx);
        if !sel.is_empty() && !sel.contains('\n') {
            self.find.query = sel;
        }
        self.find.open = true;
        self.find.replace = replace;
        self.find.focus = true;
    }

    fn find_next(&mut self, ctx: &egui::Context, forward: bool) {
        let re = match self.find.build() {
            Ok(r) => r,
            Err(e) => {
                self.find.msg = e;
                return;
            }
        };
        let text = &self.docs[self.active].text;
        let (a, b) = self.cursor(ctx);
        let (ba, bb) = (byte_of(text, a), byte_of(text, b));
        let all: Vec<(usize, usize)> =
            re.find_iter(text).filter(|m| !m.is_empty()).map(|m| (m.start(), m.end())).collect();
        let found = if forward {
            all.iter().find(|m| m.0 >= bb && (m.0 > ba || b > a || m.0 >= bb)).or(all.first())
        } else {
            all.iter().rev().find(|m| m.1 <= ba).or(all.last())
        };
        match found {
            Some(&(s, e)) => {
                let wrapped = if forward { s < bb } else { e > ba };
                self.find.msg = if wrapped { "Recherche relancée depuis le début.".into() } else { String::new() };
                self.select = Some((char_of(text, s), char_of(text, e)));
            }
            None => self.find.msg = "Aucun résultat.".into(),
        }
    }

    fn replace_one(&mut self, ctx: &egui::Context) {
        let Ok(re) = self.find.build() else { return };
        let (a, b) = self.cursor(ctx);
        let text = &self.docs[self.active].text;
        let (ba, bb) = (byte_of(text, a), byte_of(text, b));
        let is_match = a != b && re.find_at(text, ba).is_some_and(|m| m.start() == ba && m.end() == bb);
        if is_match {
            let rep = if self.find.regex {
                re.replace(&text[ba..bb], self.find.repl.as_str()).into_owned()
            } else {
                self.find.repl.clone()
            };
            let n = rep.chars().count();
            self.edit(ctx, a, b, &rep, (a + n, a + n));
        }
        self.find_next(ctx, true);
    }

    fn replace_all(&mut self, ctx: &egui::Context) {
        let Ok(re) = self.find.build() else { return };
        let doc = &mut self.docs[self.active];
        let count = re.find_iter(&doc.text).filter(|m| !m.is_empty()).count();
        if count == 0 {
            self.find.msg = "Aucun résultat.".into();
            return;
        }
        let new = if self.find.regex {
            re.replace_all(&doc.text, self.find.repl.as_str()).into_owned()
        } else {
            re.replace_all(&doc.text, regex::NoExpand(&self.find.repl)).into_owned()
        };
        doc.text = new;
        doc.refresh_modified();
        self.find.msg = format!("{count} remplacement(s) effectué(s).");
        self.dirty = true;
        let _ = ctx;
    }

    fn goto_line(&mut self, ctx: &egui::Context, line: usize) {
        let text = &self.docs[self.active].text;
        let mut start = 0usize;
        let mut ci = 0usize;
        let mut cur = 1;
        for (i, c) in text.char_indices() {
            if cur == line {
                break;
            }
            if c == '\n' {
                cur += 1;
                start = i + 1;
            }
            ci = char_of(text, i) + 1;
        }
        let _ = start;
        if cur < line {
            ci = text.chars().count();
        }
        set_cursor(ctx, self.editor_id(), ci, ci);
        self.focus_editor = true;
        self.scroll_to_cursor = true;
    }
}

fn item_is_list(app: &App, a: usize, _b: usize) -> bool {
    let text = &app.docs[app.active].text;
    let (ls, le) = line_bounds(text, a);
    lists::parse(&text[ls..le]).is_some()
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let dir = Path::new(r"C:\Windows\Fonts");
    let mut add = |name: &str, file: &str, family: egui::FontFamily| {
        if let Ok(bytes) = std::fs::read(dir.join(file)) {
            fonts.font_data.insert(name.into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
            fonts.families.entry(family).or_default().insert(0, name.into());
        }
    };
    add("segoeui", "segoeui.ttf", egui::FontFamily::Proportional);
    add("consolas", "consola.ttf", egui::FontFamily::Monospace);
    ctx.set_fonts(fonts);
}

// ───────────────────────────── Interface ─────────────────────────────

fn shortcut(ctx: &egui::Context, m: Modifiers, k: Key) -> bool {
    ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)))
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;

        // Fichiers déposés
        let dropped: Vec<PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        for p in dropped {
            self.open_path(&p);
        }

        self.shortcuts(ctx);
        if self.confirm_close.is_none() {
            self.handle_list_keys(ctx);
        }

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui, ctx));
        egui::Panel::top("onglets").show(ui, |ui| self.tab_bar(ui));
        if self.find.open {
            egui::Panel::top("recherche").show(ui, |ui| self.find_bar(ui, ctx));
        }
        if self.settings.status_bar {
            egui::Panel::bottom("etat").show(ui, |ui| self.status_bar(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| self.editor(ui, ctx));

        self.dialogs(ctx);

        let title = {
            let d = &self.docs[self.active];
            format!("{}{} - {APP_NAME}", if d.modified { "*" } else { "" }, d.title())
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        if self.dirty && self.last_save.elapsed() > Duration::from_secs(3) {
            self.save_session();
        }
        if self.dirty {
            ctx.request_repaint_after(Duration::from_secs(3));
        }
    }

    fn on_exit(&mut self) {
        self.save_session();
    }
}

impl App {
    fn shortcuts(&mut self, ctx: &egui::Context) {
        let c = Modifiers::COMMAND;
        let cs = Modifiers::COMMAND | Modifiers::SHIFT;
        if shortcut(ctx, cs, Key::N) {
            let _ = std::env::current_exe().map(|e| std::process::Command::new(e).spawn());
        }
        if shortcut(ctx, cs, Key::S) {
            self.save_as(self.active);
        }
        if shortcut(ctx, c, Key::N) || shortcut(ctx, c, Key::T) {
            self.new_tab();
        }
        if shortcut(ctx, c, Key::O) {
            self.open_dialog();
        }
        if shortcut(ctx, c, Key::S) {
            self.save(self.active);
        }
        if shortcut(ctx, c, Key::W) {
            self.request_close(self.active);
        }
        if shortcut(ctx, c, Key::F) {
            self.open_find(ctx, false);
        }
        if shortcut(ctx, c, Key::H) {
            self.open_find(ctx, true);
        }
        if shortcut(ctx, c, Key::G) {
            self.goto = Some(String::new());
        }
        if shortcut(ctx, Modifiers::SHIFT, Key::F3) {
            self.find_next(ctx, false);
        }
        if shortcut(ctx, Modifiers::NONE, Key::F3) {
            self.find_next(ctx, true);
        }
        if shortcut(ctx, Modifiers::NONE, Key::F5) {
            self.insert_datetime(ctx);
        }
        if shortcut(ctx, Modifiers::NONE, Key::Escape) && self.find.open {
            self.find.open = false;
            self.focus_editor = true;
        }
        let n = self.docs.len();
        if shortcut(ctx, Modifiers::CTRL | Modifiers::SHIFT, Key::Tab) {
            self.active = (self.active + n - 1) % n;
        } else if shortcut(ctx, Modifiers::CTRL, Key::Tab) {
            self.active = (self.active + 1) % n;
        }
        // Listes : Ctrl+Maj+8 (puces) / Ctrl+Maj+7 (numérotée)
        if shortcut(ctx, cs, Key::Num8) {
            self.apply_list(ctx, ListKind::Bullet);
        }
        if shortcut(ctx, cs, Key::Num7) {
            self.apply_list(ctx, ListKind::Numbered);
        }
        // Zoom
        if shortcut(ctx, c, Key::Plus) || shortcut(ctx, c, Key::Equals) {
            self.zoom(0.1);
        }
        if shortcut(ctx, c, Key::Minus) {
            self.zoom(-0.1);
        }
        if shortcut(ctx, c, Key::Num0) {
            self.settings.zoom = 1.0;
            self.dirty = true;
        }
        let scroll = ctx.input(|i| if i.modifiers.command { i.smooth_scroll_delta.y } else { 0.0 });
        if scroll != 0.0 {
            self.zoom(scroll.signum() * 0.1);
        }
    }

    fn zoom(&mut self, d: f32) {
        self.settings.zoom = (self.settings.zoom + d).clamp(0.5, 5.0);
        self.dirty = true;
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Fichier", |ui| {
                if ui.add(egui::Button::new("Nouvel onglet").shortcut_text("Ctrl+N")).clicked() {
                    self.new_tab();
                    ui.close();
                }
                if ui.add(egui::Button::new("Nouvelle fenêtre").shortcut_text("Ctrl+Maj+N")).clicked() {
                    let _ = std::env::current_exe().map(|e| std::process::Command::new(e).spawn());
                    ui.close();
                }
                if ui.add(egui::Button::new("Ouvrir…").shortcut_text("Ctrl+O")).clicked() {
                    ui.close();
                    self.open_dialog();
                }
                ui.separator();
                if ui.add(egui::Button::new("Enregistrer").shortcut_text("Ctrl+S")).clicked() {
                    ui.close();
                    self.save(self.active);
                }
                if ui.add(egui::Button::new("Enregistrer sous…").shortcut_text("Ctrl+Maj+S")).clicked() {
                    ui.close();
                    self.save_as(self.active);
                }
                if ui.button("Enregistrer tout").clicked() {
                    ui.close();
                    self.save_all();
                }
                ui.separator();
                if ui.add(egui::Button::new("Fermer l'onglet").shortcut_text("Ctrl+W")).clicked() {
                    ui.close();
                    self.request_close(self.active);
                }
                if ui.button("Quitter").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Édition", |ui| {
                if ui.add(egui::Button::new("Annuler").shortcut_text("Ctrl+Z")).clicked() {
                    ui.close();
                    self.undo_redo(ctx, false);
                }
                if ui.add(egui::Button::new("Rétablir").shortcut_text("Ctrl+Y")).clicked() {
                    ui.close();
                    self.undo_redo(ctx, true);
                }
                ui.separator();
                if ui.add(egui::Button::new("Couper").shortcut_text("Ctrl+X")).clicked() {
                    ui.close();
                    self.cut(ctx);
                }
                if ui.add(egui::Button::new("Copier").shortcut_text("Ctrl+C")).clicked() {
                    ui.close();
                    self.copy(ctx);
                }
                if ui.add(egui::Button::new("Coller").shortcut_text("Ctrl+V")).clicked() {
                    ui.close();
                    self.paste(ctx);
                }
                if ui.add(egui::Button::new("Supprimer").shortcut_text("Suppr")).clicked() {
                    ui.close();
                    let (a, b) = self.cursor(ctx);
                    self.edit(ctx, a, b, "", (a, a));
                }
                ui.separator();
                if ui.add(egui::Button::new("Rechercher…").shortcut_text("Ctrl+F")).clicked() {
                    ui.close();
                    self.open_find(ctx, false);
                }
                if ui.add(egui::Button::new("Suivant").shortcut_text("F3")).clicked() {
                    ui.close();
                    self.find_next(ctx, true);
                }
                if ui.add(egui::Button::new("Précédent").shortcut_text("Maj+F3")).clicked() {
                    ui.close();
                    self.find_next(ctx, false);
                }
                if ui.add(egui::Button::new("Remplacer…").shortcut_text("Ctrl+H")).clicked() {
                    ui.close();
                    self.open_find(ctx, true);
                }
                if ui.add(egui::Button::new("Atteindre la ligne…").shortcut_text("Ctrl+G")).clicked() {
                    ui.close();
                    self.goto = Some(String::new());
                }
                ui.separator();
                if ui.add(egui::Button::new("Sélectionner tout").shortcut_text("Ctrl+A")).clicked() {
                    ui.close();
                    self.select_all(ctx);
                }
                if ui.add(egui::Button::new("Heure/Date").shortcut_text("F5")).clicked() {
                    ui.close();
                    self.insert_datetime(ctx);
                }
            });
            ui.menu_button("Format", |ui| {
                ui.menu_button("Listes", |ui| {
                    for (kind, label) in ListKind::ALL {
                        if ui.button(label).clicked() {
                            ui.close();
                            self.apply_list(ctx, kind);
                        }
                    }
                    ui.separator();
                    if ui.button("Supprimer la liste").clicked() {
                        ui.close();
                        self.transform_lines(ctx, lists::remove);
                    }
                });
                if ui.add(egui::Button::new("Augmenter le retrait").shortcut_text("Tab")).clicked() {
                    ui.close();
                    self.transform_lines(ctx, lists::indent);
                }
                if ui.add(egui::Button::new("Diminuer le retrait").shortcut_text("Maj+Tab")).clicked() {
                    ui.close();
                    self.transform_lines(ctx, lists::outdent);
                }
                ui.separator();
                if ui.checkbox(&mut self.settings.wrap, "Retour automatique à la ligne").changed() {
                    self.dirty = true;
                }
            });
            ui.menu_button("Affichage", |ui| {
                ui.menu_button("Zoom", |ui| {
                    if ui.add(egui::Button::new("Zoom avant").shortcut_text("Ctrl+Plus")).clicked() {
                        self.zoom(0.1);
                    }
                    if ui.add(egui::Button::new("Zoom arrière").shortcut_text("Ctrl+Moins")).clicked() {
                        self.zoom(-0.1);
                    }
                    if ui.add(egui::Button::new("Restaurer le zoom par défaut").shortcut_text("Ctrl+0")).clicked() {
                        self.settings.zoom = 1.0;
                        self.dirty = true;
                    }
                });
                if ui.checkbox(&mut self.settings.status_bar, "Barre d'état").changed() {
                    self.dirty = true;
                }
                ui.separator();
                ui.label("Thème");
                let before = self.settings.theme;
                ui.radio_value(&mut self.settings.theme, Theme::System, "Système");
                ui.radio_value(&mut self.settings.theme, Theme::Light, "Clair");
                ui.radio_value(&mut self.settings.theme, Theme::Dark, "Sombre");
                if before != self.settings.theme {
                    self.apply_theme(ctx);
                    self.dirty = true;
                }
            });
        });
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let mut close: Option<usize> = None;
        let mut activate: Option<usize> = None;
        let mut add = false;
        egui::ScrollArea::horizontal().id_salt("tabs").show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, d) in self.docs.iter().enumerate() {
                    let mut label = d.title();
                    if d.modified {
                        label.push_str(" •");
                    }
                    let r = ui.selectable_label(i == self.active, label);
                    if r.clicked() {
                        activate = Some(i);
                    }
                    if r.middle_clicked() {
                        close = Some(i);
                    }
                    if ui.small_button("×").on_hover_text("Fermer l'onglet").clicked() {
                        close = Some(i);
                    }
                    ui.separator();
                }
                if ui.button("+").on_hover_text("Nouvel onglet (Ctrl+N)").clicked() {
                    add = true;
                }
            });
        });
        if let Some(i) = activate {
            self.active = i;
            self.focus_editor = true;
            self.dirty = true;
        }
        if add {
            self.new_tab();
        }
        if let Some(i) = close {
            self.request_close(i);
        }
    }

    fn find_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_space(4.0);
        let mut next = false;
        let mut prev = false;
        let mut rep = false;
        let mut rep_all = false;
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.find.query).hint_text("Rechercher").desired_width(220.0));
            if self.find.focus {
                r.request_focus();
                self.find.focus = false;
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                next = true;
                r.request_focus();
            }
            if ui.button("↑").on_hover_text("Précédent (Maj+F3)").clicked() {
                prev = true;
            }
            if ui.button("↓").on_hover_text("Suivant (F3)").clicked() {
                next = true;
            }
            ui.toggle_value(&mut self.find.case, "Aa").on_hover_text("Respecter la casse");
            ui.toggle_value(&mut self.find.word, "Mot").on_hover_text("Mot entier");
            ui.toggle_value(&mut self.find.regex, ".*").on_hover_text("Expression régulière");
            if ui.toggle_value(&mut self.find.replace, "Remplacer").changed() {}
            if ui.button("×").clicked() {
                self.find.open = false;
            }
            ui.label(&self.find.msg);
        });
        if self.find.replace {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.find.repl).hint_text("Remplacer par").desired_width(220.0));
                rep = ui.button("Remplacer").clicked();
                rep_all = ui.button("Tout remplacer").clicked();
            });
        }
        ui.add_space(4.0);
        if next {
            self.find_next(ctx, true);
        }
        if prev {
            self.find_next(ctx, false);
        }
        if rep {
            self.replace_one(ctx);
        }
        if rep_all {
            self.replace_all(ctx);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let idx = self.active;
        ui.horizontal(|ui| {
            ui.label(format!("Ln {}, col {}", self.pos.0, self.pos.1));
            ui.separator();
            if self.sel_chars > 0 {
                ui.label(format!("{} car. sélectionnés", self.sel_chars));
            } else {
                ui.label(format!("{} caractères", self.total_chars));
            }
            ui.separator();
            ui.label(format!("{:.0} %", self.settings.zoom * 100.0));
            ui.separator();
            ui.menu_button(self.docs[idx].eol.label(), |ui| {
                for e in Eol::ALL {
                    if ui.selectable_label(self.docs[idx].eol == e, e.label()).clicked() {
                        self.docs[idx].eol = e;
                        self.docs[idx].modified = true;
                        self.dirty = true;
                        ui.close();
                    }
                }
            });
            ui.separator();
            ui.menu_button(self.docs[idx].enc.label(), |ui| {
                for e in Enc::ALL {
                    if ui.selectable_label(self.docs[idx].enc == e, e.label()).clicked() {
                        self.docs[idx].enc = e;
                        self.docs[idx].modified = true;
                        self.dirty = true;
                        ui.close();
                    }
                }
            });
        });
    }

    fn editor(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let id = self.editor_id();
        let font = FontId::monospace(14.0 * self.settings.zoom);
        if let Some((a, b)) = self.select.take() {
            set_cursor(ctx, id, a, b);
            self.focus_editor = true;
            self.scroll_to_cursor = true;
        }
        let wrap = self.settings.wrap;
        let size = ui.available_size();
        let doc = &mut self.docs[self.active];

        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
            let mut job = egui::text::LayoutJob::simple(
                text.as_str().to_owned(),
                font.clone(),
                ui.visuals().text_color(),
                if wrap { wrap_width } else { f32::INFINITY },
            );
            job.halign = egui::Align::LEFT;
            ui.fonts_mut(|f| f.layout_job(job))
        };

        let area = if wrap {
            egui::ScrollArea::vertical()
        } else {
            egui::ScrollArea::both()
        };
        let mut changed = false;
        let mut cursor_range = None;
        area.id_salt(("scroll", doc.id)).auto_shrink([false, false]).show(ui, |ui| {
            let mut te = egui::TextEdit::multiline(&mut doc.text)
                .id(id)
                .font(font.clone())
                .frame(egui::Frame::NONE)
                .lock_focus(true)
                .min_size(size)
                .layouter(&mut layouter);
            if wrap {
                te = te.desired_width(ui.available_width());
            } else {
                te = te.desired_width(f32::INFINITY);
            }
            let out = te.show(ui);
            if out.response.changed() {
                changed = true;
            }
            cursor_range = out.cursor_range;
            if self.scroll_to_cursor {
                if let Some(r) = out.cursor_range {
                    let rect = out.galley.pos_from_cursor(r.primary).translate(out.galley_pos.to_vec2());
                    ui.scroll_to_rect(rect.expand2(egui::vec2(0.0, 20.0)), None);
                }
                self.scroll_to_cursor = false;
            }
        });
        if self.focus_editor {
            ctx.memory_mut(|m| m.request_focus(id));
            self.focus_editor = false;
        }
        let doc = &mut self.docs[self.active];
        if changed {
            doc.refresh_modified();
            self.dirty = true;
        }
        if let Some(r) = cursor_range.or_else(|| {
            get_cursor(ctx, id).map(|(a, b)| CCursorRange::two(CCursor::new(b), CCursor::new(a)))
        }) {
            let (s, e) = sorted(r.primary.index.0, r.secondary.index.0);
            let p = r.primary.index.0;
            let before: String = doc.text.chars().take(p).collect();
            let line = before.matches('\n').count() + 1;
            let col = before.rsplit('\n').next().map(|l| l.chars().count()).unwrap_or(0) + 1;
            self.pos = (line, col);
            self.sel_chars = e - s;
        }
        self.total_chars = doc.text.chars().count();
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(msg) = self.error.clone() {
            egui::Window::new("Erreur").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label(msg);
                if ui.button("OK").clicked() {
                    self.error = None;
                }
            });
        }
        if let Some(i) = self.confirm_close {
            let mut action: Option<u8> = None;
            egui::Window::new("Bloc_note").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label(RichText::new(format!("Voulez-vous enregistrer les modifications apportées à « {} » ?", self.docs[i].title())).strong());
                ui.horizontal(|ui| {
                    if ui.button("Enregistrer").clicked() { action = Some(0); }
                    if ui.button("Ne pas enregistrer").clicked() { action = Some(1); }
                    if ui.button("Annuler").clicked() { action = Some(2); }
                });
            });
            match action {
                Some(0) => {
                    self.confirm_close = None;
                    if self.save(i) {
                        self.close_now(i);
                    }
                }
                Some(1) => {
                    self.confirm_close = None;
                    self.close_now(i);
                }
                Some(_) => self.confirm_close = None,
                None => {}
            }
        }
        if let Some(mut buf) = self.goto.take() {
            let mut keep = true;
            let mut go = false;
            egui::Window::new("Atteindre la ligne").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                let max = self.docs[self.active].text.matches('\n').count() + 1;
                ui.label(format!("Numéro de ligne (1 à {max}) :"));
                let r = ui.text_edit_singleline(&mut buf);
                r.request_focus();
                buf.retain(|c| c.is_ascii_digit());
                ui.horizontal(|ui| {
                    if ui.button("Atteindre").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))) {
                        go = true;
                    }
                    if ui.button("Annuler").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        keep = false;
                    }
                });
            });
            if go {
                if let Ok(n) = buf.parse::<usize>() {
                    self.goto_line(ctx, n.max(1));
                }
            } else if keep {
                self.goto = Some(buf);
            }
        }
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([360.0, 240.0])
            .with_title(APP_NAME)
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(APP_NAME, options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
