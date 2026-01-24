use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use serde_json::{Map, Value};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Clone, Copy)]
enum FieldKind {
    Text,
    Bool { default: bool },
    StringList,
    PairList { left: &'static str, right: &'static str },
    MapList,
    AdminList,
}

#[derive(Clone, Copy)]
struct FieldSpec {
    key: &'static str,
    kind: FieldKind,
    hint: &'static str,
    aliases: &'static [&'static str],
}

const COMMON_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        key: "http-serv-port",
        kind: FieldKind::Text,
        hint: "HTTP service port (legacy)",
        aliases: &[],
    },
    FieldSpec {
        key: "process_waittime_sec",
        kind: FieldKind::Text,
        hint: "Processing wait timeout in seconds",
        aliases: &["process_waittime", "process_waittime_ms"],
    },
    FieldSpec {
        key: "min_interval_ms",
        kind: FieldKind::Text,
        hint: "Minimum send interval in milliseconds",
        aliases: &["min_interval_sec"],
    },
    FieldSpec {
        key: "max_queue",
        kind: FieldKind::Text,
        hint: "Maximum queued posts before flush",
        aliases: &[],
    },
    FieldSpec {
        key: "max_image_number_one_post",
        kind: FieldKind::Text,
        hint: "Max images per post",
        aliases: &["max_images_per_post"],
    },
    FieldSpec {
        key: "send_timeout_ms",
        kind: FieldKind::Text,
        hint: "Send timeout in milliseconds",
        aliases: &["send_timeout", "send_timeout_sec"],
    },
    FieldSpec {
        key: "send_max_attempts",
        kind: FieldKind::Text,
        hint: "Max send attempts",
        aliases: &["max_send_attempts", "max_send_attempt"],
    },
    FieldSpec {
        key: "tz_offset_minutes",
        kind: FieldKind::Text,
        hint: "Timezone offset in minutes",
        aliases: &[],
    },
    FieldSpec {
        key: "max_cache_mb",
        kind: FieldKind::Text,
        hint: "Blob cache size in MB",
        aliases: &[],
    },
    FieldSpec {
        key: "apikey",
        kind: FieldKind::Text,
        hint: "LLM API key",
        aliases: &[],
    },
    FieldSpec {
        key: "text_model",
        kind: FieldKind::Text,
        hint: "Text model name",
        aliases: &[],
    },
    FieldSpec {
        key: "vision_model",
        kind: FieldKind::Text,
        hint: "Vision model name",
        aliases: &[],
    },
    FieldSpec {
        key: "vision_pixel_limit",
        kind: FieldKind::Text,
        hint: "Image pixel limit",
        aliases: &[],
    },
    FieldSpec {
        key: "vision_size_limit_mb",
        kind: FieldKind::Text,
        hint: "Image size limit in MB",
        aliases: &[],
    },
    FieldSpec {
        key: "napcat_base_url",
        kind: FieldKind::Text,
        hint: "NapCat reverse WS base URL",
        aliases: &[],
    },
    FieldSpec {
        key: "napcat_access_token",
        kind: FieldKind::Text,
        hint: "NapCat access token",
        aliases: &[],
    },
    FieldSpec {
        key: "manage_napcat_internal",
        kind: FieldKind::Bool { default: false },
        hint: "Manage NapCat internally",
        aliases: &[],
    },
    FieldSpec {
        key: "renewcookies_use_napcat",
        kind: FieldKind::Bool { default: true },
        hint: "Use NapCat for cookie renewal",
        aliases: &[],
    },
    FieldSpec {
        key: "max_attempts_qzone_autologin",
        kind: FieldKind::Text,
        hint: "Max Qzone auto-login attempts",
        aliases: &[],
    },
    FieldSpec {
        key: "force_chromium_no_sandbox",
        kind: FieldKind::Bool { default: false },
        hint: "Force Chromium no-sandbox",
        aliases: &["force_chromium_no-sandbox"],
    },
    FieldSpec {
        key: "at_unprived_sender",
        kind: FieldKind::Bool { default: false },
        hint: "Mention sender when their space is private",
        aliases: &[],
    },
    FieldSpec {
        key: "friend_request_window_sec",
        kind: FieldKind::Text,
        hint: "Friend request window in seconds",
        aliases: &[],
    },
    FieldSpec {
        key: "friend_add_message",
        kind: FieldKind::Text,
        hint: "Friend add message override",
        aliases: &[],
    },
    FieldSpec {
        key: "use_web_review",
        kind: FieldKind::Bool { default: false },
        hint: "Enable web review panel",
        aliases: &[],
    },
    FieldSpec {
        key: "web_review_port",
        kind: FieldKind::Text,
        hint: "Web review port",
        aliases: &[],
    },
];

const GROUP_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        key: "mangroupid",
        kind: FieldKind::Text,
        hint: "Audit group id (mangroupid)",
        aliases: &[],
    },
    FieldSpec {
        key: "mainqqid",
        kind: FieldKind::Text,
        hint: "Primary account id (mainqqid)",
        aliases: &[],
    },
    FieldSpec {
        key: "mainqq_http_port",
        kind: FieldKind::Text,
        hint: "Primary account HTTP port",
        aliases: &[],
    },
    FieldSpec {
        key: "minorqqid",
        kind: FieldKind::PairList {
            left: "minorqqid",
            right: "minorqq_http_port",
        },
        hint: "Secondary account ids (paired with minorqq_http_port)",
        aliases: &[],
    },
    FieldSpec {
        key: "accounts",
        kind: FieldKind::StringList,
        hint: "Account list (override main/minor)",
        aliases: &[],
    },
    FieldSpec {
        key: "napcat_base_url",
        kind: FieldKind::Text,
        hint: "NapCat reverse WS base URL override",
        aliases: &[],
    },
    FieldSpec {
        key: "napcat_access_token",
        kind: FieldKind::Text,
        hint: "NapCat access token override",
        aliases: &[],
    },
    FieldSpec {
        key: "process_waittime_sec",
        kind: FieldKind::Text,
        hint: "Processing wait timeout in seconds",
        aliases: &["process_waittime", "process_waittime_ms"],
    },
    FieldSpec {
        key: "min_interval_ms",
        kind: FieldKind::Text,
        hint: "Minimum send interval in milliseconds",
        aliases: &["min_interval_sec"],
    },
    FieldSpec {
        key: "max_post_stack",
        kind: FieldKind::Text,
        hint: "Maximum queued posts before flush",
        aliases: &[],
    },
    FieldSpec {
        key: "max_image_number_one_post",
        kind: FieldKind::Text,
        hint: "Max images per post",
        aliases: &["max_images_per_post"],
    },
    FieldSpec {
        key: "send_timeout_ms",
        kind: FieldKind::Text,
        hint: "Send timeout in milliseconds",
        aliases: &["send_timeout", "send_timeout_sec"],
    },
    FieldSpec {
        key: "send_max_attempts",
        kind: FieldKind::Text,
        hint: "Max send attempts",
        aliases: &["max_send_attempts", "max_send_attempt"],
    },
    FieldSpec {
        key: "send_schedule",
        kind: FieldKind::StringList,
        hint: "Send schedule list (HH:MM)",
        aliases: &[],
    },
    FieldSpec {
        key: "individual_image_in_posts",
        kind: FieldKind::Bool { default: true },
        hint: "Send original images alongside rendered post",
        aliases: &[],
    },
    FieldSpec {
        key: "watermark_text",
        kind: FieldKind::Text,
        hint: "Watermark text",
        aliases: &[],
    },
    FieldSpec {
        key: "friend_add_message",
        kind: FieldKind::Text,
        hint: "Friend add message override",
        aliases: &[],
    },
    FieldSpec {
        key: "friend_request_window_sec",
        kind: FieldKind::Text,
        hint: "Friend request window in seconds",
        aliases: &[],
    },
    FieldSpec {
        key: "quick_replies",
        kind: FieldKind::MapList,
        hint: "Quick replies map (command -> reply)",
        aliases: &[],
    },
    FieldSpec {
        key: "admins",
        kind: FieldKind::AdminList,
        hint: "Web review admins (username/password)",
        aliases: &[],
    },
];

#[derive(Clone)]
struct FieldEntry {
    key: String,
    spec: Option<FieldSpec>,
}

#[derive(Clone, Copy)]
struct TabBounds {
    start: u16,
    end: u16,
}

impl TabBounds {
    fn contains(&self, x: u16) -> bool {
        x >= self.start && x < self.end
    }
}

struct GroupTab {
    name: String,
    bounds: TabBounds,
}

struct GroupActionTab {
    action: GroupAction,
    bounds: TabBounds,
}

#[derive(Clone, Copy)]
enum GroupAction {
    Add,
}

#[derive(Clone)]
enum SectionKind {
    Common,
    Group(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConfigFocus {
    Sections,
    Fields,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupStorage {
    Inline,
    Nested,
}

enum EditMode {
    Value {
        section: SectionKind,
        key: String,
        spec: Option<FieldSpec>,
    },
    NewKeyName { section: SectionKind },
    NewKeyValue { section: SectionKind, key: String },
    NewGroupName,
}

struct EditState {
    mode: EditMode,
    buffer: String,
}

#[derive(Clone, Copy)]
enum StatusLevel {
    Info,
    Warn,
    Error,
}

struct StatusMessage {
    text: String,
    level: StatusLevel,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ListFocus {
    Left,
    Right,
}

enum ListKind {
    StringList,
    PairList { left: String, right: String },
    MapList,
    AdminList,
}

enum ListItem {
    Single(String),
    Pair(String, String),
    MapEntry(String, String),
    Admin(String, String),
}

struct ListInput {
    index: usize,
    focus: ListFocus,
    buffer: String,
}

struct ListEditor {
    section: SectionKind,
    key: String,
    kind: ListKind,
    items: Vec<ListItem>,
    selected: usize,
    offset: usize,
    input: Option<ListInput>,
    focus: ListFocus,
    dirty: bool,
    aliases: Vec<&'static str>,
}

pub struct ConfigEditor {
    path: PathBuf,
    common: Map<String, Value>,
    groups: BTreeMap<String, Map<String, Value>>,
    storage: GroupStorage,
    other_root: Map<String, Value>,
    focus: ConfigFocus,
    selected_section: usize,
    selected_field: usize,
    section_offset: usize,
    field_offset: usize,
    section_height: usize,
    field_height: usize,
    layout_sections: Rect,
    layout_fields: Rect,
    layout_groupbar: Rect,
    layout_list: Rect,
    editing: Option<EditState>,
    list_editor: Option<ListEditor>,
    pending_group_delete: Option<String>,
    current_group: Option<String>,
    group_tabs: Vec<GroupTab>,
    group_actions: Vec<GroupActionTab>,
    dirty: bool,
    status: Option<StatusMessage>,
    cursor_on: bool,
    last_blink: Instant,
}

impl ConfigEditor {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, String> {
        let path = path.into();
        match fs::read_to_string(&path) {
            Ok(data) => {
                let root: Value =
                    serde_json::from_str(&data).map_err(|err| format!("invalid json: {err}"))?;
                Self::from_value(path, root)
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                Ok(Self::empty(path, Some("config not found, starting empty")))
            }
            Err(err) => Err(format!("failed to read {}: {err}", path.display())),
        }
    }

    pub fn is_editing(&self) -> bool {
        self.editing.is_some() || self.list_editor.is_some()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.list_editor.is_some() {
            self.handle_list_key(key);
            return;
        }
        if self.editing.is_some() {
            self.handle_edit_key(key);
            return;
        }

        if self.pending_group_delete.is_some()
            && !matches!(key.code, KeyCode::Char('x') | KeyCode::Delete)
        {
            self.pending_group_delete = None;
        }

        match key.code {
            KeyCode::Tab => self.toggle_focus(),
            KeyCode::Left => self.focus_sections(),
            KeyCode::Right => self.focus_fields(),
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::PageUp => self.page_selection(-1),
            KeyCode::PageDown => self.page_selection(1),
            KeyCode::Home => self.jump_selection_top(),
            KeyCode::End => self.jump_selection_bottom(),
            KeyCode::Enter | KeyCode::Char('e') => self.begin_edit_value(),
            KeyCode::Char(' ') | KeyCode::Char('t') => self.toggle_selected_bool(),
            KeyCode::Char('a') => self.begin_new_key(),
            KeyCode::Char('g') => self.begin_new_group(),
            KeyCode::Char('x') | KeyCode::Delete => self.request_delete_group(),
            KeyCode::Char('[') => self.select_prev_group(),
            KeyCode::Char(']') => self.select_next_group(),
            KeyCode::Char('s') => self.save_current(),
            KeyCode::Char('r') => self.reload_current(),
            _ => {}
        }
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.handle_mouse_click(mouse.column, mouse.row);
            }
            MouseEventKind::ScrollUp => {
                self.handle_mouse_scroll(-1, mouse.column, mouse.row);
            }
            MouseEventKind::ScrollDown => {
                self.handle_mouse_scroll(1, mouse.column, mouse.row);
            }
            _ => {}
        }
    }

    pub fn tick(&mut self) {
        let active_cursor = self.editing.is_some()
            || self
                .list_editor
                .as_ref()
                .map(|editor| editor.input.is_some())
                .unwrap_or(false);
        if !active_cursor {
            self.cursor_on = false;
            return;
        }
        if self.last_blink.elapsed() >= Duration::from_millis(500) {
            self.cursor_on = !self.cursor_on;
            self.last_blink = Instant::now();
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        if self.list_editor.is_some() {
            self.render_list_editor(f, area);
            return;
        }

        let sections = self.section_labels();
        self.clamp_section(sections.len());
        self.ensure_current_group();
        let fields = self.current_fields();
        self.clamp_field(fields.len());

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);

        let header_line = self.header_line(layout[0].width as usize);
        let header = Paragraph::new(header_line)
            .style(Style::default().add_modifier(Modifier::REVERSED));
        f.render_widget(header, layout[0]);

        let groupbar_line = self.group_bar_line(layout[1].width as usize);
        let groupbar = Paragraph::new(groupbar_line)
            .style(Style::default().add_modifier(Modifier::REVERSED));
        f.render_widget(groupbar, layout[1]);
        self.layout_groupbar = layout[1];

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(28), Constraint::Percentage(72)])
            .split(layout[2]);

        self.layout_sections = body[0];
        self.layout_fields = body[1];
        self.section_height = body[0].height.saturating_sub(2) as usize;
        self.field_height = body[1].height.saturating_sub(2) as usize;
        self.ensure_visible();

        let section_lines = self.section_lines(&sections, body[0].width as usize);
        let sections_widget = Paragraph::new(Text::from(section_lines)).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Sections"),
        );
        f.render_widget(sections_widget, body[0]);

        let field_lines = self.field_lines(&fields, body[1].width as usize);
        let fields_title = match self.current_section() {
            SectionKind::Common => "Fields".to_string(),
            SectionKind::Group(name) => {
                if name.is_empty() {
                    "Fields (group)".to_string()
                } else {
                    format!("Fields ({name})")
                }
            }
        };
        let fields_widget = Paragraph::new(Text::from(field_lines)).block(
            Block::default()
                .borders(Borders::ALL)
                .title(fields_title),
        );
        f.render_widget(fields_widget, body[1]);

        let hint_line = self.hint_line(layout[3].width as usize);
        let hint_widget = Paragraph::new(hint_line)
            .style(Style::default().add_modifier(Modifier::REVERSED));
        f.render_widget(hint_widget, layout[3]);

        let footer = self.footer_line(layout[4].width as usize);
        let footer_widget = Paragraph::new(footer)
            .style(Style::default().add_modifier(Modifier::REVERSED));
        f.render_widget(footer_widget, layout[4]);
    }

    fn empty(path: PathBuf, status: Option<&str>) -> Self {
        let mut other_root = Map::new();
        other_root.insert(
            "schema_version".to_string(),
            Value::Number(serde_json::Number::from(1)),
        );
        let mut editor = Self {
            path,
            common: Map::new(),
            groups: BTreeMap::new(),
            storage: GroupStorage::Nested,
            other_root,
            focus: ConfigFocus::Fields,
            selected_section: 0,
            selected_field: 0,
            section_offset: 0,
            field_offset: 0,
            section_height: 0,
            field_height: 0,
            layout_sections: Rect::default(),
            layout_fields: Rect::default(),
            layout_groupbar: Rect::default(),
            layout_list: Rect::default(),
            editing: None,
            list_editor: None,
            pending_group_delete: None,
            current_group: None,
            group_tabs: Vec::new(),
            group_actions: Vec::new(),
            dirty: false,
            status: None,
            cursor_on: false,
            last_blink: Instant::now(),
        };
        if let Some(message) = status {
            editor.set_status(message, StatusLevel::Warn);
        }
        editor
    }

    fn from_value(path: PathBuf, root: Value) -> Result<Self, String> {
        let obj = root
            .as_object()
            .ok_or_else(|| "config must be a json object".to_string())?;
        let mut warnings = Vec::new();
        let (common, common_warn) = map_from_value(obj.get("common"));
        if let Some(message) = common_warn {
            warnings.push(format!("common: {message}"));
        }

        let mut groups = BTreeMap::new();
        let mut other_root = Map::new();
        let storage = if let Some(groups_obj) = obj.get("groups").and_then(|v| v.as_object()) {
            for (name, value) in groups_obj {
                let (map, warn) = map_from_value(Some(value));
                if let Some(message) = warn {
                    warnings.push(format!("group {name}: {message}"));
                }
                groups.insert(name.clone(), map);
            }
            for (key, value) in obj {
                if key == "common" || key == "groups" {
                    continue;
                }
                other_root.insert(key.clone(), value.clone());
            }
            GroupStorage::Nested
        } else {
            for (key, value) in obj {
                if key == "common" {
                    continue;
                }
                if key == "schema_version" {
                    other_root.insert(key.clone(), value.clone());
                    continue;
                }
                let (map, warn) = map_from_value(Some(value));
                if let Some(message) = warn {
                    warnings.push(format!("group {key}: {message}"));
                }
                groups.insert(key.clone(), map);
            }
            GroupStorage::Inline
        };

        let mut editor = Self {
            path,
            common,
            groups,
            storage,
            other_root,
            focus: ConfigFocus::Fields,
            selected_section: 0,
            selected_field: 0,
            section_offset: 0,
            field_offset: 0,
            section_height: 0,
            field_height: 0,
            layout_sections: Rect::default(),
            layout_fields: Rect::default(),
            layout_groupbar: Rect::default(),
            layout_list: Rect::default(),
            editing: None,
            list_editor: None,
            pending_group_delete: None,
            current_group: None,
            group_tabs: Vec::new(),
            group_actions: Vec::new(),
            dirty: false,
            status: None,
            cursor_on: false,
            last_blink: Instant::now(),
        };

        if let Some(warn) = warnings.first() {
            editor.set_status(warn, StatusLevel::Warn);
        }
        editor.ensure_current_group();

        Ok(editor)
    }

    fn save_current(&mut self) {
        let (errors, warnings) = self.validate();
        if !errors.is_empty() {
            let msg = summarize_messages(&errors, 1, "errors");
            self.set_status(format!("save blocked: {msg}"), StatusLevel::Error);
            return;
        }
        match self.save() {
            Ok(()) => {
                if warnings.is_empty() {
                    self.set_status("saved config", StatusLevel::Info);
                } else {
                    let msg = summarize_messages(&warnings, 1, "warnings");
                    self.set_status(format!("saved with warning: {msg}"), StatusLevel::Warn);
                }
            }
            Err(err) => self.set_status(err, StatusLevel::Error),
        }
    }

    fn reload_current(&mut self) {
        let path = self.path.clone();
        match Self::load(path) {
            Ok(mut fresh) => {
                let had_status = fresh.status.take();
                *self = fresh;
                if let Some(status) = had_status {
                    self.status = Some(status);
                } else {
                    self.set_status("reloaded config", StatusLevel::Info);
                }
            }
            Err(err) => self.set_status(format!("reload failed: {err}"), StatusLevel::Error),
        }
    }

    fn save(&mut self) -> Result<(), String> {
        let mut root = Map::new();
        root.insert("common".to_string(), Value::Object(sanitize_map(&self.common)));
        match self.storage {
            GroupStorage::Nested => {
                let mut groups_map = Map::new();
                for (name, map) in &self.groups {
                    groups_map.insert(name.clone(), Value::Object(sanitize_map(map)));
                }
                root.insert("groups".to_string(), Value::Object(groups_map));
                for (key, value) in sanitize_map(&self.other_root) {
                    if key == "common" || key == "groups" {
                        continue;
                    }
                    root.insert(key, value);
                }
            }
            GroupStorage::Inline => {
                for (key, value) in sanitize_map(&self.other_root) {
                    if key == "common" {
                        continue;
                    }
                    root.insert(key, value);
                }
                for (name, map) in &self.groups {
                    root.insert(name.clone(), Value::Object(sanitize_map(map)));
                }
            }
        }

        let output = serde_json::to_string_pretty(&Value::Object(root))
            .map_err(|err| format!("serialize failed: {err}"))?;
        fs::write(&self.path, format!("{output}\n"))
            .map_err(|err| format!("write failed: {err}"))?;
        self.dirty = false;
        Ok(())
    }

    fn section_labels(&self) -> Vec<String> {
        vec!["Common".to_string(), "Group".to_string()]
    }

    fn current_section(&self) -> SectionKind {
        if self.selected_section == 0 {
            SectionKind::Common
        } else {
            let name = self.current_group.clone().unwrap_or_default();
            SectionKind::Group(name)
        }
    }

    fn current_fields(&self) -> Vec<FieldEntry> {
        let section = self.current_section();
        if let SectionKind::Group(name) = &section {
            if name.is_empty() || !self.groups.contains_key(name) {
                return Vec::new();
            }
        }
        let specs = match &section {
            SectionKind::Common => COMMON_FIELDS,
            SectionKind::Group(_) => GROUP_FIELDS,
        };

        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for spec in specs {
            out.push(FieldEntry {
                key: spec.key.to_string(),
                spec: Some(*spec),
            });
            seen.insert(spec.key);
            for alias in spec.aliases {
                seen.insert(*alias);
            }
        }

        if let Some(map) = self.section_map(&section) {
            let mut extras: Vec<String> = map
                .keys()
                .filter(|k| !seen.contains(k.as_str()))
                .cloned()
                .collect();
            extras.sort();
            for key in extras {
                out.push(FieldEntry { key, spec: None });
            }
        }

        out
    }

    fn group_names(&self) -> Vec<String> {
        self.groups.keys().cloned().collect()
    }

    fn ensure_current_group(&mut self) {
        let current_valid = self
            .current_group
            .as_ref()
            .map(|name| self.groups.contains_key(name))
            .unwrap_or(false);
        if current_valid {
            return;
        }
        self.current_group = self.groups.keys().next().cloned();
    }

    fn group_bar_line(&mut self, width: usize) -> Line<'static> {
        self.group_tabs.clear();
        self.group_actions.clear();
        let mut spans = Vec::new();
        let prefix = "Group: ";
        spans.push(Span::raw(prefix.to_string()));
        let mut col = UnicodeWidthStr::width(prefix) as u16;

        let names = self.group_names();
        if names.is_empty() {
            let label = "<none>";
            spans.push(Span::styled(label, Style::default().fg(Color::Yellow)));
            col = col.saturating_add(UnicodeWidthStr::width(label) as u16);
        } else {
            for (idx, name) in names.iter().enumerate() {
                if idx > 0 {
                    spans.push(Span::raw(" "));
                    col = col.saturating_add(1);
                }
                let active = self.current_group.as_deref() == Some(name.as_str());
                let label = if active {
                    format!("[{name}]")
                } else {
                    name.to_string()
                };
                let width_label = UnicodeWidthStr::width(label.as_str()) as u16;
                let bounds = TabBounds {
                    start: col,
                    end: col.saturating_add(width_label),
                };
                let style = if active {
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                spans.push(Span::styled(label.clone(), style));
                self.group_tabs.push(GroupTab {
                    name: name.clone(),
                    bounds,
                });
                col = col.saturating_add(width_label);
            }
        }

        spans.push(Span::raw(" "));
        col = col.saturating_add(1);
        let add_label = "[+]";
        let add_width = UnicodeWidthStr::width(add_label) as u16;
        let add_bounds = TabBounds {
            start: col,
            end: col.saturating_add(add_width),
        };
        spans.push(Span::styled(
            add_label,
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
        self.group_actions.push(GroupActionTab {
            action: GroupAction::Add,
            bounds: add_bounds,
        });

        let line = Line::from(spans);
        if width == 0 {
            Line::raw("")
        } else {
            line
        }
    }

    fn select_prev_group(&mut self) {
        let names = self.group_names();
        if names.is_empty() {
            return;
        }
        let idx = self
            .current_group
            .as_ref()
            .and_then(|name| names.iter().position(|item| item == name))
            .unwrap_or(0);
        let next = if idx == 0 { names.len() - 1 } else { idx - 1 };
        self.current_group = Some(names[next].clone());
        self.selected_section = 1;
        self.selected_field = 0;
        self.field_offset = 0;
        self.pending_group_delete = None;
    }

    fn select_next_group(&mut self) {
        let names = self.group_names();
        if names.is_empty() {
            return;
        }
        let idx = self
            .current_group
            .as_ref()
            .and_then(|name| names.iter().position(|item| item == name))
            .unwrap_or(0);
        let next = (idx + 1) % names.len();
        self.current_group = Some(names[next].clone());
        self.selected_section = 1;
        self.selected_field = 0;
        self.field_offset = 0;
        self.pending_group_delete = None;
    }

    fn selected_field_entry<'a>(&self, fields: &'a [FieldEntry]) -> Option<&'a FieldEntry> {
        fields.get(self.selected_field)
    }

    fn section_map(&self, section: &SectionKind) -> Option<&Map<String, Value>> {
        match section {
            SectionKind::Common => Some(&self.common),
            SectionKind::Group(name) => self.groups.get(name),
        }
    }

    fn section_map_mut(&mut self, section: &SectionKind) -> Option<&mut Map<String, Value>> {
        match section {
            SectionKind::Common => Some(&mut self.common),
            SectionKind::Group(name) => self.groups.get_mut(name),
        }
    }

    fn clamp_section(&mut self, len: usize) {
        if len == 0 {
            self.selected_section = 0;
            return;
        }
        if self.selected_section >= len {
            self.selected_section = len - 1;
        }
    }

    fn clamp_field(&mut self, len: usize) {
        if len == 0 {
            self.selected_field = 0;
            return;
        }
        if self.selected_field >= len {
            self.selected_field = len - 1;
        }
    }

    fn ensure_visible(&mut self) {
        let section_count = self.section_labels().len();
        self.section_offset = ensure_visible_offset(
            self.section_offset,
            self.selected_section,
            self.section_height,
            section_count,
        );

        let field_count = self.current_fields().len();
        self.field_offset = ensure_visible_offset(
            self.field_offset,
            self.selected_field,
            self.field_height,
            field_count,
        );
    }

    fn selection_style(&self, selected: bool, focused: bool) -> Style {
        let mut style = Style::default();
        if selected {
            style = style.add_modifier(Modifier::REVERSED);
        }
        if focused {
            style = style.add_modifier(Modifier::BOLD);
        }
        style
    }

    fn section_lines(&self, sections: &[String], width: usize) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if sections.is_empty() {
            lines.push(Line::raw("No sections"));
            return lines;
        }
        let start = self.section_offset.min(sections.len());
        let end = (start + self.section_height.max(1)).min(sections.len());
        for (idx, label) in sections[start..end].iter().enumerate() {
            let absolute = start + idx;
            let style = self.selection_style(
                absolute == self.selected_section,
                self.focus == ConfigFocus::Sections,
            );
            let text = truncate_to_width(label, width.saturating_sub(2));
            lines.push(Line::styled(text, style));
        }
        lines
    }

    fn field_lines(&self, fields: &[FieldEntry], width: usize) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if fields.is_empty() {
            if self.selected_section == 1 && self.current_group.is_none() {
                lines.push(Line::raw("No groups. Press 'g' to add."));
            } else {
                lines.push(Line::raw("No fields. Press 'a' to add."));
            }
            return lines;
        }
        let section = self.current_section();
        let mut inline_key: Option<&str> = None;
        let mut inline_value: Option<&str> = None;
        let mut inline_cursor = false;
        if let Some(edit) = &self.editing {
            if let EditMode::Value { key, .. } = &edit.mode {
                inline_key = Some(key.as_str());
                inline_value = Some(edit.buffer.as_str());
                inline_cursor = self.cursor_on;
            }
        }
        let start = self.field_offset.min(fields.len());
        let end = (start + self.field_height.max(1)).min(fields.len());
        for (idx, entry) in fields[start..end].iter().enumerate() {
            let absolute = start + idx;
            let style = self.selection_style(
                absolute == self.selected_field,
                self.focus == ConfigFocus::Fields,
            );
            let preview = if Some(entry.key.as_str()) == inline_key {
                let buffer = inline_value.unwrap_or("");
                if inline_cursor {
                    format!("{buffer}|")
                } else {
                    buffer.to_string()
                }
            } else {
                self.field_preview(&section, entry)
            };
            let line = format!("{} = {}", entry.key, preview);
            let text = truncate_to_width(&line, width.saturating_sub(2));
            lines.push(Line::styled(text, style));
        }
        lines
    }

    fn field_preview(&self, section: &SectionKind, entry: &FieldEntry) -> String {
        let map = match self.section_map(section) {
            Some(map) => map,
            None => return "<unset>".to_string(),
        };
        let Some(spec) = entry.spec else {
            return map
                .get(&entry.key)
                .map(value_display)
                .unwrap_or_else(|| "<unset>".to_string());
        };

        let value = resolve_value(map, &spec).or_else(|| map.get(spec.key));
        match spec.kind {
            FieldKind::Bool { default } => {
                let (val, missing, invalid) = bool_value(value, default);
                if invalid {
                    value
                        .map(value_display)
                        .unwrap_or_else(|| "<invalid>".to_string())
                } else {
                    let display = if val { "[x]" } else { "[ ]" };
                    if missing {
                        format!("{display} (default)")
                    } else {
                        display.to_string()
                    }
                }
            }
            FieldKind::StringList => {
                let items = extract_string_list(value);
                if is_unset_value(value) {
                    "<unset>".to_string()
                } else {
                    format!("list({})", items.len())
                }
            }
            FieldKind::PairList { left, right } => {
                let items = extract_pair_list(map, left, right);
                if map.get(left).is_none() && map.get(right).is_none() {
                    "<unset>".to_string()
                } else {
                    format!("pairs({})", items.len())
                }
            }
            FieldKind::MapList => {
                let items = extract_map_list(value);
                if is_unset_value(value) {
                    "<unset>".to_string()
                } else {
                    format!("map({})", items.len())
                }
            }
            FieldKind::AdminList => {
                let items = extract_admin_list(value);
                if is_unset_value(value) {
                    "<unset>".to_string()
                } else {
                    format!("admins({})", items.len())
                }
            }
            FieldKind::Text => {
                if is_unset_value(value) {
                    "<unset>".to_string()
                } else {
                    value.map(value_display).unwrap_or_else(|| "<unset>".to_string())
                }
            }
        }
    }

    fn header_line(&self, width: usize) -> Line<'static> {
        let dirty = if self.dirty { " *" } else { "" };
        let text = format!("Config: {}{}", self.path.display(), dirty);
        Line::raw(truncate_to_width(&text, width))
    }

    fn hint_line(&self, width: usize) -> Line<'static> {
        let hint = self.current_hint();
        if hint.is_empty() {
            return Line::raw(truncate_to_width("Hint: -", width));
        }
        let text = format!("Hint: {hint}");
        Line::raw(truncate_to_width(&text, width))
    }

    fn current_hint(&self) -> String {
        let fields = self.current_fields();
        let Some(entry) = self.selected_field_entry(&fields) else {
            return String::new();
        };
        entry
            .spec
            .map(|spec| spec.hint.to_string())
            .unwrap_or_else(|| "Custom field".to_string())
    }

    fn footer_line(&self, width: usize) -> Line<'static> {
        if let Some(edit) = &self.editing {
            let label = match &edit.mode {
                EditMode::Value { key, .. } => format!("Edit {key}: "),
                EditMode::NewKeyName { .. } => "New key name: ".to_string(),
                EditMode::NewKeyValue { key, .. } => format!("New value for {key}: "),
                EditMode::NewGroupName => "New group id: ".to_string(),
            };
            let text = format!("{label}{}", edit.buffer);
            return Line::raw(truncate_to_width(&text, width));
        }

        let help = "mouse click | Tab focus | arrows move | enter/e edit | space toggle | a add key | g add group | x delete group | [ ] switch group | s save | r reload";
        self.status_with_help(help, width)
    }

    fn status_with_help(&self, help: &str, width: usize) -> Line<'static> {
        if let Some(status) = &self.status {
            let status_text = truncate_to_width(&status.text, width.saturating_sub(help.len() + 3));
            let style = match status.level {
                StatusLevel::Info => Style::default().fg(Color::Green),
                StatusLevel::Warn => Style::default().fg(Color::Yellow),
                StatusLevel::Error => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            };
            let mut spans = Vec::new();
            spans.push(Span::styled(status_text, style));
            spans.push(Span::raw(" | "));
            spans.push(Span::raw(help.to_string()));
            Line::from(spans)
        } else {
            Line::raw(truncate_to_width(help, width))
        }
    }

    fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            ConfigFocus::Sections => ConfigFocus::Fields,
            ConfigFocus::Fields => ConfigFocus::Sections,
        };
    }

    fn focus_sections(&mut self) {
        self.focus = ConfigFocus::Sections;
    }

    fn focus_fields(&mut self) {
        self.focus = ConfigFocus::Fields;
    }

    fn move_selection(&mut self, delta: isize) {
        match self.focus {
            ConfigFocus::Sections => {
                let len = self.section_labels().len();
                if len == 0 {
                    return;
                }
                let next = clamp_index(self.selected_section, delta, len);
                if next != self.selected_section {
                    self.selected_section = next;
                    self.selected_field = 0;
                    self.field_offset = 0;
                    self.pending_group_delete = None;
                }
            }
            ConfigFocus::Fields => {
                let len = self.current_fields().len();
                if len == 0 {
                    return;
                }
                self.selected_field = clamp_index(self.selected_field, delta, len);
            }
        }
    }

    fn page_selection(&mut self, direction: isize) {
        let step = match self.focus {
            ConfigFocus::Sections => self.section_height.max(1) as isize,
            ConfigFocus::Fields => self.field_height.max(1) as isize,
        };
        self.move_selection(step * direction);
    }

    fn jump_selection_top(&mut self) {
        match self.focus {
            ConfigFocus::Sections => {
                self.selected_section = 0;
                self.selected_field = 0;
                self.field_offset = 0;
                self.pending_group_delete = None;
            }
            ConfigFocus::Fields => {
                self.selected_field = 0;
            }
        }
    }

    fn jump_selection_bottom(&mut self) {
        match self.focus {
            ConfigFocus::Sections => {
                let len = self.section_labels().len();
                if len > 0 {
                    self.selected_section = len - 1;
                    self.selected_field = 0;
                    self.field_offset = 0;
                    self.pending_group_delete = None;
                }
            }
            ConfigFocus::Fields => {
                let len = self.current_fields().len();
                if len > 0 {
                    self.selected_field = len - 1;
                }
            }
        }
    }

    fn begin_edit_value(&mut self) {
        if self.focus != ConfigFocus::Fields {
            return;
        }
        let fields = self.current_fields();
        if fields.is_empty() {
            self.set_status("no field selected", StatusLevel::Warn);
            return;
        }
        let entry = fields[self.selected_field].clone();
        let section = self.current_section();
        if matches!(section, SectionKind::Group(ref name) if name.is_empty()) {
            self.set_status("no group selected", StatusLevel::Warn);
            return;
        }
        if let Some(spec) = entry.spec {
            match spec.kind {
                FieldKind::StringList | FieldKind::PairList { .. } | FieldKind::MapList | FieldKind::AdminList => {
                    self.open_list_editor(section, entry, spec);
                    return;
                }
                _ => {}
            }
        }

        let map = match self.section_map(&section) {
            Some(map) => map,
            None => return,
        };
        let value = entry
            .spec
            .and_then(|spec| resolve_value(map, &spec))
            .or_else(|| map.get(&entry.key))
            .unwrap_or(&Value::Null);
        let buffer = value_to_input(value);
        self.editing = Some(EditState {
            mode: EditMode::Value {
                section,
                key: entry.key,
                spec: entry.spec,
            },
            buffer,
        });
        self.cursor_on = true;
        self.last_blink = Instant::now();
    }

    fn begin_new_key(&mut self) {
        let section = self.current_section();
        if matches!(section, SectionKind::Group(ref name) if name.is_empty()) {
            self.set_status("no group selected", StatusLevel::Warn);
            return;
        }
        self.editing = Some(EditState {
            mode: EditMode::NewKeyName { section },
            buffer: String::new(),
        });
    }

    fn begin_new_group(&mut self) {
        self.editing = Some(EditState {
            mode: EditMode::NewGroupName,
            buffer: String::new(),
        });
    }

    fn request_delete_group(&mut self) {
        if self.focus != ConfigFocus::Sections {
            return;
        }
        let section = self.current_section();
        let SectionKind::Group(name) = section else {
            self.set_status("cannot delete common section", StatusLevel::Warn);
            return;
        };
        if name.is_empty() {
            self.set_status("no group selected", StatusLevel::Warn);
            return;
        }
        if self.pending_group_delete.as_deref() == Some(&name) {
            self.groups.remove(&name);
            self.pending_group_delete = None;
            if self.current_group.as_deref() == Some(&name) {
                self.current_group = self.groups.keys().next().cloned();
            }
            self.selected_section = 1;
            self.selected_field = 0;
            self.field_offset = 0;
            self.dirty = true;
            self.set_status(format!("deleted group {name}"), StatusLevel::Info);
        } else {
            self.pending_group_delete = Some(name.clone());
            self.set_status(
                format!("press x again to delete group {name}"),
                StatusLevel::Warn,
            );
        }
    }

    fn toggle_selected_bool(&mut self) {
        if self.focus != ConfigFocus::Fields {
            return;
        }
        let fields = self.current_fields();
        let Some(entry) = self.selected_field_entry(&fields) else {
            return;
        };
        let section = self.current_section();
        let map = match self.section_map(&section) {
            Some(map) => map,
            None => return,
        };
        let mut default = false;
        let mut bool_field = false;
        let mut spec = None;
        if let Some(entry_spec) = entry.spec {
            if let FieldKind::Bool { default: spec_default } = entry_spec.kind {
                default = spec_default;
                bool_field = true;
                spec = Some(entry_spec);
            }
        }
        let value = spec
            .and_then(|spec| resolve_value(map, &spec))
            .or_else(|| map.get(&entry.key));
        let (current, _, invalid) = bool_value(value, default);
        if !bool_field && invalid {
            self.set_status("selected field is not a bool", StatusLevel::Warn);
            return;
        }
        let next = !current;
        let key = entry.key.clone();
        if let Some(map) = self.section_map_mut(&section) {
            if let Some(spec) = spec {
                clear_aliases(map, &spec);
            }
            map.insert(key.clone(), Value::Bool(next));
            self.dirty = true;
            self.set_status(format!("set {key} = {next}"), StatusLevel::Info);
        }
    }

    fn handle_edit_key(&mut self, key: KeyEvent) {
        let Some(edit) = &mut self.editing else {
            return;
        };

        match key.code {
            KeyCode::Esc => {
                self.editing = None;
                self.cursor_on = false;
            }
            KeyCode::Enter => {
                let edit = self.editing.take().unwrap();
                self.commit_edit(edit);
                self.cursor_on = false;
            }
            KeyCode::Backspace => {
                edit.buffer.pop();
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                edit.buffer.push(ch);
            }
            _ => {}
        }
    }

    fn handle_mouse_click(&mut self, x: u16, y: u16) {
        if let Some(editor) = self.list_editor.as_mut() {
            if rect_contains(self.layout_list, x, y) {
                if let Some(index) = list_click_index(
                    self.layout_list,
                    y,
                    editor.offset,
                    editor.items.len(),
                ) {
                    let was_selected = index == editor.selected;
                    editor.selected = index;
                    if was_selected {
                        editor.start_edit();
                        self.cursor_on = true;
                        self.last_blink = Instant::now();
                    }
                }
            }
            return;
        }

        if rect_contains(self.layout_groupbar, x, y) {
            let local_x = x.saturating_sub(self.layout_groupbar.x);
            for tab in &self.group_tabs {
                if tab.bounds.contains(local_x) {
                    self.current_group = Some(tab.name.clone());
                    self.selected_section = 1;
                    self.selected_field = 0;
                    self.field_offset = 0;
                    self.pending_group_delete = None;
                    return;
                }
            }
            for action in &self.group_actions {
                if action.bounds.contains(local_x) {
                    match action.action {
                        GroupAction::Add => self.begin_new_group(),
                    }
                    return;
                }
            }
        }

        if rect_contains(self.layout_sections, x, y) {
            if let Some(index) = list_click_index(
                self.layout_sections,
                y,
                self.section_offset,
                self.section_labels().len(),
            ) {
                self.focus_sections();
                if index != self.selected_section {
                    self.selected_section = index;
                    self.selected_field = 0;
                    self.field_offset = 0;
                    self.pending_group_delete = None;
                }
            }
            return;
        }

        if rect_contains(self.layout_fields, x, y) {
            let fields = self.current_fields();
            if let Some(index) = list_click_index(
                self.layout_fields,
                y,
                self.field_offset,
                fields.len(),
            ) {
                let was_selected = index == self.selected_field;
                self.focus_fields();
                self.selected_field = index;
                if was_selected {
                    if let Some(spec) = fields[index].spec {
                        if matches!(spec.kind, FieldKind::Bool { .. }) {
                            self.toggle_selected_bool();
                            return;
                        }
                    }
                    self.begin_edit_value();
                }
            }
        }
    }

    fn handle_mouse_scroll(&mut self, direction: isize, x: u16, y: u16) {
        if let Some(editor) = self.list_editor.as_mut() {
            if rect_contains(self.layout_list, x, y) {
                editor.move_selection(direction);
            }
            return;
        }

        if rect_contains(self.layout_groupbar, x, y) {
            if direction < 0 {
                self.select_prev_group();
            } else {
                self.select_next_group();
            }
            return;
        }

        if rect_contains(self.layout_sections, x, y) {
            self.focus_sections();
            self.move_selection(direction);
        } else if rect_contains(self.layout_fields, x, y) {
            self.focus_fields();
            self.move_selection(direction);
        }
    }

    fn commit_edit(&mut self, edit: EditState) {
        match edit.mode {
            EditMode::Value {
                section,
                key,
                spec,
            } => {
                if let Some(map) = self.section_map_mut(&section) {
                    if let Some(spec) = spec {
                        clear_aliases(map, &spec);
                    }
                    if edit.buffer.trim().is_empty() {
                        map.remove(&key);
                        self.dirty = true;
                        self.set_status(format!("unset {key}"), StatusLevel::Info);
                        return;
                    }
                    let value = parse_input_value(&edit.buffer);
                    map.insert(key.clone(), value);
                    self.dirty = true;
                    self.set_status(format!("updated {key}"), StatusLevel::Info);
                }
            }
            EditMode::NewKeyName { section } => {
                let key = edit.buffer.trim();
                if key.is_empty() {
                    self.set_status("key name is empty", StatusLevel::Warn);
                    return;
                }
                let exists = self
                    .section_map(&section)
                    .map(|map| map.contains_key(key))
                    .unwrap_or(false);
                if exists {
                    self.set_status("key already exists", StatusLevel::Warn);
                    self.editing = Some(EditState {
                        mode: EditMode::NewKeyName { section },
                        buffer: key.to_string(),
                    });
                    return;
                }
                self.editing = Some(EditState {
                    mode: EditMode::NewKeyValue {
                        section,
                        key: key.to_string(),
                    },
                    buffer: String::new(),
                });
            }
            EditMode::NewKeyValue { section, key } => {
                if let Some(map) = self.section_map_mut(&section) {
                    if edit.buffer.trim().is_empty() {
                        self.dirty = true;
                        self.select_key(&key);
                        self.set_status(format!("unset {key}"), StatusLevel::Info);
                        return;
                    }
                    let value = parse_input_value(&edit.buffer);
                    map.insert(key.clone(), value);
                    self.dirty = true;
                    self.select_key(&key);
                    self.set_status(format!("added {key}"), StatusLevel::Info);
                }
            }
            EditMode::NewGroupName => {
                let name = edit.buffer.trim();
                if name.is_empty() {
                    self.set_status("group name is empty", StatusLevel::Warn);
                    return;
                }
                if name.eq_ignore_ascii_case("common") || name.eq_ignore_ascii_case("groups") {
                    self.set_status("group name is reserved", StatusLevel::Warn);
                    return;
                }
                if !is_valid_group_name(name) {
                    self.set_status("group name must be alnum or underscore", StatusLevel::Warn);
                    return;
                }
                if self.groups.contains_key(name) {
                    self.set_status("group already exists", StatusLevel::Warn);
                    return;
                }
                self.groups.insert(name.to_string(), Map::new());
                self.current_group = Some(name.to_string());
                self.selected_section = 1;
                self.selected_field = 0;
                self.field_offset = 0;
                self.dirty = true;
                self.set_status(format!("added group {name}"), StatusLevel::Info);
            }
        }
    }

    fn select_key(&mut self, key: &str) {
        let fields = self.current_fields();
        if let Some(idx) = fields.iter().position(|entry| entry.key == key) {
            self.selected_field = idx;
        }
    }

    fn open_list_editor(&mut self, section: SectionKind, entry: FieldEntry, spec: FieldSpec) {
        let kind = match spec.kind {
            FieldKind::StringList => ListKind::StringList,
            FieldKind::PairList { left, right } => ListKind::PairList {
                left: left.to_string(),
                right: right.to_string(),
            },
            FieldKind::MapList => ListKind::MapList,
            FieldKind::AdminList => ListKind::AdminList,
            _ => return,
        };
        let items = self.load_list_items(&section, &entry.key, &kind, Some(spec));
        self.list_editor = Some(ListEditor {
            section,
            key: entry.key,
            kind,
            items,
            selected: 0,
            offset: 0,
            input: None,
            focus: ListFocus::Left,
            dirty: false,
            aliases: spec.aliases.to_vec(),
        });
        self.cursor_on = true;
        self.last_blink = Instant::now();
    }

    fn load_list_items(
        &self,
        section: &SectionKind,
        key: &str,
        kind: &ListKind,
        spec: Option<FieldSpec>,
    ) -> Vec<ListItem> {
        let map = match self.section_map(section) {
            Some(map) => map,
            None => return Vec::new(),
        };
        match kind {
            ListKind::StringList => {
                let value = spec.and_then(|spec| resolve_value(map, &spec));
                let value = value.or_else(|| map.get(key));
                extract_string_list(value)
                    .into_iter()
                    .map(ListItem::Single)
                    .collect()
            }
            ListKind::PairList { left, right } => {
                let items = extract_pair_list(map, left, right);
                items
                    .into_iter()
                    .map(|(l, r)| ListItem::Pair(l, r))
                    .collect()
            }
            ListKind::MapList => {
                let value = spec.and_then(|spec| resolve_value(map, &spec));
                let value = value.or_else(|| map.get(key));
                extract_map_list(value)
            }
                .into_iter()
                .map(|(k, v)| ListItem::MapEntry(k, v))
                .collect(),
            ListKind::AdminList => {
                let value = spec.and_then(|spec| resolve_value(map, &spec));
                let value = value.or_else(|| map.get(key));
                extract_admin_list(value)
            }
                .into_iter()
                .map(|(u, p)| ListItem::Admin(u, p))
                .collect(),
        }
    }

    fn handle_list_key(&mut self, key: KeyEvent) {
        let Some(editor) = self.list_editor.as_mut() else {
            return;
        };

        if let Some(input) = editor.input.as_mut() {
            match key.code {
                KeyCode::Esc => {
                    editor.input = None;
                    self.cursor_on = false;
                }
                KeyCode::Enter => {
                    let input = editor.input.take().unwrap();
                    editor.apply_input(input);
                    self.cursor_on = false;
                }
                KeyCode::Backspace => {
                    input.buffer.pop();
                }
                KeyCode::Char(ch)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    input.buffer.push(ch);
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Esc => {
                self.close_list_editor();
            }
            KeyCode::Up => editor.move_selection(-1),
            KeyCode::Down => editor.move_selection(1),
            KeyCode::PageUp => editor.page_selection(-1),
            KeyCode::PageDown => editor.page_selection(1),
            KeyCode::Home => editor.jump_top(),
            KeyCode::End => editor.jump_bottom(),
            KeyCode::Tab => editor.toggle_focus(),
            KeyCode::Enter | KeyCode::Char('e') => {
                editor.start_edit();
                self.cursor_on = true;
                self.last_blink = Instant::now();
            }
            KeyCode::Char('a') => {
                editor.add_item();
                self.cursor_on = true;
                self.last_blink = Instant::now();
            }
            KeyCode::Char('d') | KeyCode::Delete => editor.delete_item(),
            _ => {}
        }
    }

    fn close_list_editor(&mut self) {
        let Some(editor) = self.list_editor.take() else {
            return;
        };
        if editor.dirty {
            if let Some(map) = self.section_map_mut(&editor.section) {
                for alias in &editor.aliases {
                    map.remove(*alias);
                }
                editor.apply_to_map(map);
                self.dirty = true;
            }
        }
        self.cursor_on = false;
    }

    fn render_list_editor(&mut self, f: &mut Frame, area: Rect) {
        let Some(editor) = self.list_editor.as_mut() else {
            return;
        };
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)])
            .split(area);

        self.layout_list = layout[1];
        let header_line = editor.header_line(layout[0].width as usize);
        let header = Paragraph::new(header_line)
            .style(Style::default().add_modifier(Modifier::REVERSED));
        f.render_widget(header, layout[0]);

        let list_height = layout[1].height.saturating_sub(2) as usize;
        editor.ensure_visible(list_height);
        let list_lines = editor.list_lines(layout[1].width as usize, list_height, self.cursor_on);
        let list_widget = Paragraph::new(Text::from(list_lines))
            .block(Block::default().borders(Borders::ALL).title("List"));
        f.render_widget(list_widget, layout[1]);

        let footer = editor.footer_line(self.status.as_ref(), layout[2].width as usize);
        let footer_widget = Paragraph::new(footer)
            .style(Style::default().add_modifier(Modifier::REVERSED));
        f.render_widget(footer_widget, layout[2]);
    }

    fn validate(&self) -> (Vec<String>, Vec<String>) {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        let mut main_ids = HashSet::new();
        let mut minor_ids = HashSet::new();
        let mut ports = HashSet::new();

        let audit_cmds = [
            "\u{662f}",
            "\u{5426}",
            "\u{533f}",
            "\u{7b49}",
            "\u{5220}",
            "\u{62d2}",
            "\u{7acb}\u{5373}",
            "\u{5237}\u{65b0}",
            "\u{91cd}\u{6e32}\u{67d3}",
            "\u{6269}\u{5217}\u{5ba1}\u{67e5}",
            "\u{8bc4}\u{8bba}",
            "\u{56de}\u{590d}",
            "\u{5c55}\u{793a}",
            "\u{62c9}\u{9ed1}",
            "\u{6d88}\u{606f}\u{5168}\u{9009}",
        ];

        for (group, obj) in &self.groups {
            if !is_valid_group_name(group) {
                errors.push(format!(
                    "group '{group}' contains invalid characters (alnum/_ only)"
                ));
                continue;
            }

            let mangroupid = value_to_string(obj.get("mangroupid")).unwrap_or_default();
            let mainqqid = value_to_string(obj.get("mainqqid")).unwrap_or_default();
            let main_port = value_to_string(obj.get("mainqq_http_port")).unwrap_or_default();

            if mangroupid.is_empty() || !is_numeric(&mangroupid) {
                errors.push(format!("{group}: mangroupid must be numeric"));
            }
            if !mainqqid.is_empty() {
                if !is_numeric(&mainqqid) {
                    errors.push(format!("{group}: mainqqid must be numeric"));
                } else if !main_ids.insert(mainqqid.clone()) {
                    errors.push(format!("mainqqid {mainqqid} is duplicated"));
                }
            } else {
                errors.push(format!("{group}: mainqqid is missing"));
            }
            if !main_port.is_empty() {
                if !is_numeric(&main_port) {
                    errors.push(format!("{group}: mainqq_http_port must be numeric"));
                } else if !ports.insert(main_port.clone()) {
                    errors.push(format!("mainqq_http_port {main_port} is duplicated"));
                }
            } else {
                errors.push(format!("{group}: mainqq_http_port is missing"));
            }

            let minor_list = extract_string_list(obj.get("minorqqid"));
            let minor_ports = extract_string_list(obj.get("minorqq_http_port"));

            if minor_list.is_empty() {
                warnings.push(format!("{group}: minorqqid is empty"));
            }
            if minor_ports.is_empty() {
                warnings.push(format!("{group}: minorqq_http_port is empty"));
            }
            for mid in &minor_list {
                if mid.is_empty() {
                    continue;
                }
                if !is_numeric(mid) {
                    errors.push(format!("{group}: minorqqid contains non-numeric {mid}"));
                } else if !minor_ids.insert(mid.clone()) || main_ids.contains(mid) {
                    errors.push(format!("minorqqid {mid} is duplicated"));
                }
            }
            for mp in &minor_ports {
                if mp.is_empty() {
                    continue;
                }
                if !is_numeric(mp) {
                    errors.push(format!(
                        "{group}: minorqq_http_port contains non-numeric {mp}"
                    ));
                } else if !ports.insert(mp.clone()) {
                    errors.push(format!("minorqq_http_port {mp} is duplicated"));
                }
            }
            if minor_list.len() != minor_ports.len() {
                errors.push(format!(
                    "{group}: minorqqid count ({}) != minorqq_http_port count ({})",
                    minor_list.len(),
                    minor_ports.len()
                ));
            }

            for key in ["max_post_stack", "max_image_number_one_post"] {
                let val = value_to_string(obj.get(key)).unwrap_or_default();
                if !val.is_empty() && !is_numeric(&val) {
                    errors.push(format!("{group}: {key} must be numeric"));
                }
            }

            for key in ["friend_add_message", "watermark_text"] {
                if let Some(value) = obj.get(key) {
                    if !matches!(value, Value::String(_)) {
                        errors.push(format!("{group}: {key} must be a string"));
                    }
                }
            }

            if let Some(value) = obj.get("send_schedule") {
                match value {
                    Value::Array(items) => {
                        for item in items {
                            if let Some(s) = value_to_string(Some(item)) {
                                if !s.is_empty() && parse_schedule_str(&s).is_none() {
                                    errors.push(format!(
                                        "{group}: send_schedule invalid time {s}"
                                    ));
                                }
                            }
                        }
                    }
                    Value::Null => {}
                    _ => {
                        errors.push(format!("{group}: send_schedule must be an array"));
                    }
                }
            }

            if let Some(value) = obj.get("quick_replies") {
                match value {
                    Value::Object(map) => {
                        for (cmd, val) in map {
                            if audit_cmds.contains(&cmd.as_str()) {
                                errors.push(format!(
                                    "{group}: quick_replies command {cmd} conflicts with audit"
                                ));
                            }
                            if !matches!(val, Value::String(_)) {
                                errors.push(format!(
                                    "{group}: quick_replies value for {cmd} must be string"
                                ));
                                continue;
                            }
                            if let Some(text) = val.as_str() {
                                if text.trim().is_empty() {
                                    errors.push(format!(
                                        "{group}: quick_replies value for {cmd} is empty"
                                    ));
                                }
                            }
                        }
                    }
                    Value::Null => {}
                    _ => {
                        errors.push(format!("{group}: quick_replies must be an object"));
                    }
                }
            }
        }

        (errors, warnings)
    }

    fn set_status(&mut self, text: impl Into<String>, level: StatusLevel) {
        self.status = Some(StatusMessage {
            text: text.into(),
            level,
        });
    }
}

impl ListEditor {
    fn header_line(&self, width: usize) -> Line<'static> {
        let section = match &self.section {
            SectionKind::Common => "common".to_string(),
            SectionKind::Group(name) => format!("group {name}"),
        };
        let key_label = match &self.kind {
            ListKind::PairList { left, right } => format!("{left}/{right}"),
            _ => self.key.clone(),
        };
        let focus = match self.focus {
            ListFocus::Left => "left",
            ListFocus::Right => "right",
        };
        let text = format!("Edit {section}::{key_label} (col={focus})");
        Line::raw(truncate_to_width(&text, width))
    }

    fn footer_line(&self, status: Option<&StatusMessage>, width: usize) -> Line<'static> {
        if let Some(input) = &self.input {
            let label = match self.kind {
                ListKind::StringList => "Edit item: ".to_string(),
                ListKind::PairList { .. } => match input.focus {
                    ListFocus::Left => "Edit left: ".to_string(),
                    ListFocus::Right => "Edit right: ".to_string(),
                },
                ListKind::MapList => match input.focus {
                    ListFocus::Left => "Edit key: ".to_string(),
                    ListFocus::Right => "Edit value: ".to_string(),
                },
                ListKind::AdminList => match input.focus {
                    ListFocus::Left => "Edit username: ".to_string(),
                    ListFocus::Right => "Edit password: ".to_string(),
                },
            };
            let text = format!("{label}{}", input.buffer);
            return Line::raw(truncate_to_width(&text, width));
        }

        let help = "mouse click | arrows move | enter/e edit | a add | d delete | Tab switch col | Esc back";
        if let Some(status) = status {
            let status_text = truncate_to_width(&status.text, width.saturating_sub(help.len() + 3));
            let style = match status.level {
                StatusLevel::Info => Style::default().fg(Color::Green),
                StatusLevel::Warn => Style::default().fg(Color::Yellow),
                StatusLevel::Error => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            };
            let mut spans = Vec::new();
            spans.push(Span::styled(status_text, style));
            spans.push(Span::raw(" | "));
            spans.push(Span::raw(help));
            Line::from(spans)
        } else {
            Line::raw(truncate_to_width(help, width))
        }
    }

    fn list_lines(&self, width: usize, height: usize, cursor_on: bool) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if self.items.is_empty() {
            lines.push(Line::raw("No items. Press 'a' to add."));
            return lines;
        }
        let start = self.offset.min(self.items.len());
        let end = (start + height.max(1)).min(self.items.len());
        for (idx, item) in self.items[start..end].iter().enumerate() {
            let absolute = start + idx;
            let prefix = format!("{:>3} ", absolute + 1);
            let mut body = match item {
                ListItem::Single(val) => val.clone(),
                ListItem::Pair(left, right) => format!("{left} | {right}"),
                ListItem::MapEntry(key, value) => format!("{key} => {value}"),
                ListItem::Admin(user, pass) => format!("{user} | {pass}"),
            };
            if let Some(input) = &self.input {
                if input.index == absolute {
                    let cursor = if cursor_on { "|" } else { "" };
                    body = match item {
                        ListItem::Single(_) => format!("{}{}", input.buffer, cursor),
                        ListItem::Pair(left, right) => match input.focus {
                            ListFocus::Left => format!("{}{} | {right}", input.buffer, cursor),
                            ListFocus::Right => format!("{left} | {}{}", input.buffer, cursor),
                        },
                        ListItem::MapEntry(key, value) => match input.focus {
                            ListFocus::Left => format!("{}{} => {value}", input.buffer, cursor),
                            ListFocus::Right => format!("{key} => {}{}", input.buffer, cursor),
                        },
                        ListItem::Admin(user, pass) => match input.focus {
                            ListFocus::Left => format!("{}{} | {pass}", input.buffer, cursor),
                            ListFocus::Right => format!("{user} | {}{}", input.buffer, cursor),
                        },
                    };
                }
            }
            let line = format!("{prefix}{body}");
            let style = if absolute == self.selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            lines.push(Line::styled(
                truncate_to_width(&line, width.saturating_sub(2)),
                style,
            ));
        }
        lines
    }

    fn ensure_visible(&mut self, height: usize) {
        self.offset = ensure_visible_offset(self.offset, self.selected, height, self.items.len());
    }

    fn move_selection(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        self.selected = clamp_index(self.selected, delta, self.items.len());
    }

    fn page_selection(&mut self, direction: isize) {
        let step = 5isize;
        self.move_selection(step * direction);
    }

    fn jump_top(&mut self) {
        if !self.items.is_empty() {
            self.selected = 0;
        }
    }

    fn jump_bottom(&mut self) {
        if !self.items.is_empty() {
            self.selected = self.items.len() - 1;
        }
    }

    fn toggle_focus(&mut self) {
        if matches!(self.kind, ListKind::StringList) {
            return;
        }
        self.focus = match self.focus {
            ListFocus::Left => ListFocus::Right,
            ListFocus::Right => ListFocus::Left,
        };
    }

    fn start_edit(&mut self) {
        if self.items.is_empty() {
            self.add_item();
            return;
        }
        let idx = self.selected;
        let buffer = match self.items.get(idx) {
            Some(ListItem::Single(val)) => val.clone(),
            Some(ListItem::Pair(left, right)) => match self.focus {
                ListFocus::Left => left.clone(),
                ListFocus::Right => right.clone(),
            },
            Some(ListItem::MapEntry(key, value)) => match self.focus {
                ListFocus::Left => key.clone(),
                ListFocus::Right => value.clone(),
            },
            Some(ListItem::Admin(user, pass)) => match self.focus {
                ListFocus::Left => user.clone(),
                ListFocus::Right => pass.clone(),
            },
            None => String::new(),
        };
        self.input = Some(ListInput {
            index: idx,
            focus: self.focus,
            buffer,
        });
    }

    fn add_item(&mut self) {
        match self.kind {
            ListKind::StringList => self.items.push(ListItem::Single(String::new())),
            ListKind::PairList { .. } => {
                self.items
                    .push(ListItem::Pair(String::new(), String::new()));
            }
            ListKind::MapList => self
                .items
                .push(ListItem::MapEntry("new_key".to_string(), "".to_string())),
            ListKind::AdminList => self
                .items
                .push(ListItem::Admin(String::new(), String::new())),
        }
        self.selected = self.items.len().saturating_sub(1);
        self.dirty = true;
        self.start_edit();
    }

    fn delete_item(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.items.remove(self.selected);
        if self.selected >= self.items.len() && !self.items.is_empty() {
            self.selected = self.items.len() - 1;
        }
        self.dirty = true;
    }

    fn apply_input(&mut self, input: ListInput) {
        if let Some(item) = self.items.get_mut(input.index) {
            match item {
                ListItem::Single(val) => {
                    *val = input.buffer;
                }
                ListItem::Pair(left, right) => match input.focus {
                    ListFocus::Left => *left = input.buffer,
                    ListFocus::Right => *right = input.buffer,
                },
                ListItem::MapEntry(key, value) => match input.focus {
                    ListFocus::Left => *key = input.buffer,
                    ListFocus::Right => *value = input.buffer,
                },
                ListItem::Admin(user, pass) => match input.focus {
                    ListFocus::Left => *user = input.buffer,
                    ListFocus::Right => *pass = input.buffer,
                },
            }
            self.dirty = true;
        }
    }

    fn apply_to_map(self, map: &mut Map<String, Value>) {
        match self.kind {
            ListKind::StringList => {
                let mut values: Vec<Value> = Vec::new();
                for item in self.items {
                    if let ListItem::Single(val) = item {
                        if val.trim().is_empty() {
                            continue;
                        }
                        values.push(Value::String(val));
                    }
                }
                if values.is_empty() {
                    map.remove(&self.key);
                } else {
                    map.insert(self.key, Value::Array(values));
                }
            }
            ListKind::PairList { left, right } => {
                let mut left_values = Vec::new();
                let mut right_values = Vec::new();
                for item in self.items {
                    if let ListItem::Pair(l, r) = item {
                        if l.trim().is_empty() && r.trim().is_empty() {
                            continue;
                        }
                        left_values.push(Value::String(l));
                        right_values.push(Value::String(r));
                    }
                }
                if left_values.is_empty() && right_values.is_empty() {
                    map.remove(&left);
                    map.remove(&right);
                } else {
                    map.insert(left, Value::Array(left_values));
                    map.insert(right, Value::Array(right_values));
                }
            }
            ListKind::MapList => {
                let mut map_val = Map::new();
                for item in self.items {
                    if let ListItem::MapEntry(key, value) = item {
                        if key.trim().is_empty() {
                            continue;
                        }
                        map_val.insert(key, Value::String(value));
                    }
                }
                if map_val.is_empty() {
                    map.remove(&self.key);
                } else {
                    map.insert(self.key, Value::Object(map_val));
                }
            }
            ListKind::AdminList => {
                let mut list = Vec::new();
                for item in self.items {
                    if let ListItem::Admin(user, pass) = item {
                        if user.trim().is_empty() {
                            continue;
                        }
                        let mut obj = Map::new();
                        obj.insert("username".to_string(), Value::String(user));
                        obj.insert("password".to_string(), Value::String(pass));
                        list.push(Value::Object(obj));
                    }
                }
                if list.is_empty() {
                    map.remove(&self.key);
                } else {
                    map.insert(self.key, Value::Array(list));
                }
            }
        }
    }
}

fn sanitize_map(map: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, value) in map {
        if let Some(value) = sanitize_value(value) {
            out.insert(key.clone(), value);
        }
    }
    out
}

fn sanitize_value(value: &Value) -> Option<Value> {
    match value {
        Value::Null => None,
        Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                if let Some(value) = sanitize_value(item) {
                    out.push(value);
                }
            }
            Some(Value::Array(out))
        }
        Value::Object(map) => Some(Value::Object(sanitize_map(map))),
        _ => Some(value.clone()),
    }
}

fn resolve_value<'a>(map: &'a Map<String, Value>, spec: &FieldSpec) -> Option<&'a Value> {
    if let Some(value) = map.get(spec.key) {
        return Some(value);
    }
    for alias in spec.aliases {
        if let Some(value) = map.get(*alias) {
            return Some(value);
        }
    }
    None
}

fn clear_aliases(map: &mut Map<String, Value>, spec: &FieldSpec) {
    for alias in spec.aliases {
        map.remove(*alias);
    }
}

fn rect_contains(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}

fn list_click_index(rect: Rect, y: u16, offset: usize, len: usize) -> Option<usize> {
    if rect.height < 2 {
        return None;
    }
    let start_y = rect.y.saturating_add(1);
    let end_y = rect
        .y
        .saturating_add(rect.height)
        .saturating_sub(1);
    if y < start_y || y >= end_y {
        return None;
    }
    let row = y.saturating_sub(start_y) as usize;
    let index = offset.saturating_add(row);
    if index < len {
        Some(index)
    } else {
        None
    }
}


fn map_from_value(value: Option<&Value>) -> (Map<String, Value>, Option<String>) {
    match value {
        Some(Value::Object(map)) => (map.clone(), None),
        Some(_) => (Map::new(), Some("expected object".to_string())),
        None => (Map::new(), None),
    }
}

fn summarize_messages(items: &[String], max: usize, label: &str) -> String {
    let mut out = String::new();
    if let Some(first) = items.first() {
        out.push_str(first);
    }
    if items.len() > max {
        out.push_str(&format!(" (and {} more {label})", items.len() - max));
    }
    out
}

fn clamp_index(current: usize, delta: isize, len: usize) -> usize {
    let next = current.saturating_add_signed(delta);
    next.min(len.saturating_sub(1))
}

fn ensure_visible_offset(offset: usize, selected: usize, height: usize, len: usize) -> usize {
    if len == 0 || height == 0 {
        return 0;
    }
    let mut out = offset.min(len.saturating_sub(1));
    if selected < out {
        out = selected;
    } else if selected >= out.saturating_add(height) {
        out = selected.saturating_add(1).saturating_sub(height);
    }
    if out + height > len {
        out = len.saturating_sub(height);
    }
    out
}

fn parse_input_value(input: &str) -> Value {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Value::String(String::new());
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if matches!(value, Value::Null) {
            return Value::String(String::new());
        }
        return value;
    }
    Value::String(input.to_string())
}

fn value_display(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "<unset>".to_string(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| "<unset>".to_string()),
    }
}

fn value_to_input(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| "".to_string()),
    }
}

fn value_to_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn is_unset_value(value: Option<&Value>) -> bool {
    matches!(value, None | Some(Value::Null))
}

fn extract_string_list(value: Option<&Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(|v| value_to_string(Some(v)))
            .collect(),
        Value::String(s) => s
            .split(',')
            .map(|item| item.trim())
            .filter(|item| !item.is_empty())
            .map(|item| item.to_string())
            .collect(),
        Value::Number(n) => vec![n.to_string()],
        Value::Bool(b) => vec![b.to_string()],
        _ => Vec::new(),
    }
}

fn extract_pair_list(map: &Map<String, Value>, left: &str, right: &str) -> Vec<(String, String)> {
    let left_list = extract_string_list(map.get(left));
    let right_list = extract_string_list(map.get(right));
    let len = left_list.len().max(right_list.len());
    let mut out = Vec::new();
    for idx in 0..len {
        let l = left_list.get(idx).cloned().unwrap_or_default();
        let r = right_list.get(idx).cloned().unwrap_or_default();
        out.push((l, r));
    }
    out
}

fn extract_map_list(value: Option<&Value>) -> Vec<(String, String)> {
    let Some(Value::Object(map)) = value else {
        return Vec::new();
    };
    let mut items: Vec<(String, String)> = map
        .iter()
        .map(|(k, v)| (k.clone(), value_display(v)))
        .collect();
    items.sort_by(|a, b| a.0.cmp(&b.0));
    items
}

fn extract_admin_list(value: Option<&Value>) -> Vec<(String, String)> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        match item {
            Value::Object(obj) => {
                let user = value_to_string(obj.get("username")).unwrap_or_default();
                let pass = value_to_string(obj.get("password")).unwrap_or_default();
                out.push((user, pass));
            }
            Value::String(s) => {
                out.push((s.clone(), String::new()));
            }
            _ => {}
        }
    }
    out
}

fn bool_value(value: Option<&Value>, default: bool) -> (bool, bool, bool) {
    match value {
        None => (default, true, false),
        Some(Value::Null) => (default, true, false),
        Some(Value::Bool(b)) => (*b, false, false),
        Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => (true, false, false),
            "false" | "0" | "no" => (false, false, false),
            _ => (default, false, true),
        },
        Some(Value::Number(n)) => (n.as_i64().unwrap_or(0) != 0, false, false),
        _ => (default, false, true),
    }
}

fn is_valid_group_name(name: &str) -> bool {
    name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_numeric(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_digit())
}

fn parse_schedule_str(value: &str) -> Option<u16> {
    let mut parts = value.split(':');
    let hour = parts.next()?.trim().parse::<u16>().ok()?;
    let minute = parts.next()?.trim().parse::<u16>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(hour.saturating_mul(60).saturating_add(minute))
}

fn truncate_to_width(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    if max_width <= 3 {
        return text.chars().take(max_width).collect();
    }
    let limit = max_width.saturating_sub(3);
    let mut out = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + w > limit {
            break;
        }
        out.push(ch);
        width += w;
    }
    out.push_str("...");
    out
}
