use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::domain::bundled;
use crate::domain::filter::{FilterOutcome, evaluate_filter, pretty_json};
use crate::domain::form::{
    FormField, FormFieldKind, SchemaType, append_array_item, build_form_fields_with,
    default_value_at_path, default_value_for_schema, form_path_key, json_scalar_display_at_path,
    remove_array_item, replace_json_at_path, resolve_schema_at_path_with, set_scalar_value,
    ResolveCtx,
};
use crate::domain::validation::{ValidationSummary, validate_document};
use crate::infra::clipboard;
use crate::infra::fs;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenMode {
    Edit,
    Help,
    ConfirmOverwrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Standard,
    Editor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Insert,
    Visual,
}

/// Single main-area layout: one column uses the full width below Schema Path (Log/Footer unchanged).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainFullwidthPane {
    Schema,
    Form,
    OutputColumn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaneId {
    SchemaPath,
    Schema,
    Form,
    OutputPath,
    Filter,
    Output,
    Log,
}

impl PaneId {
    pub const ALL: [Self; 7] = [
        Self::SchemaPath,
        Self::Schema,
        Self::Form,
        Self::OutputPath,
        Self::Filter,
        Self::Output,
        Self::Log,
    ];

    pub const STANDARD: [Self; 6] = [
        Self::SchemaPath,
        Self::Form,
        Self::OutputPath,
        Self::Filter,
        Self::Output,
        Self::Log,
    ];

    pub fn base_title(self) -> &'static str {
        match self {
            Self::SchemaPath => "Schema Path",
            Self::Schema => "Schema",
            Self::Form => "Form",
            Self::OutputPath => "Output Path",
            Self::Filter => "Filter",
            Self::Output => "Output",
            Self::Log => "Log",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
}

#[derive(Debug, Clone, Default)]
pub struct PaneHistory {
    pub undo: Vec<PaneEditState>,
    pub redo: Vec<PaneEditState>,
}

#[derive(Debug, Clone)]
pub enum PaneEditState {
    SchemaPath { schema_source: String },
    OutputPath { output_path: String },
    Schema { schema_text: String },
    Form { buffers: Vec<String> },
    Filter { filter_text: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionAnchor {
    pub pane: PaneId,
    pub row: usize,
    pub col: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormArrayButtonKind {
    Add,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormArrayButtonFocus {
    pub array_path: Vec<String>,
    pub kind: FormArrayButtonKind,
}

#[derive(Debug, Clone)]
pub struct SchemaError {
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct SchemaPathState {
    pub schema_source: String,
    pub output_path: String,
}

impl Default for SchemaPathState {
    fn default() -> Self {
        Self {
            schema_source: "./schema/basic.json".to_owned(),
            output_path: "./output.json".to_owned(),
        }
    }
}

#[derive(Debug, Clone)]
struct CompletionCycle {
    seed: String,
    candidates: Vec<String>,
    index: usize,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub app_mode: AppMode,
    pub screen_mode: ScreenMode,
    pub input_mode: InputMode,
    pub active_pane: PaneId,
    pub schema_path: SchemaPathState,
    pub schema_text: String,
    pub schema_json: Value,
    pub output_json: Value,
    pub filter_text: String,
    pub filter_outcome: FilterOutcome,
    pub validation: ValidationSummary,
    pub form_fields: Vec<FormField>,
    pub field_errors: HashMap<String, String>,
    pub schema_error: Option<SchemaError>,
    pub pane_cursors: HashMap<PaneId, Cursor>,
    pub pane_histories: HashMap<PaneId, PaneHistory>,
    pub visual_anchor: Option<SelectionAnchor>,
    schema_completion: Option<CompletionCycle>,
    pub pending_g: bool,
    pub pending_d: bool,
    pub pending_z: bool,
    pub logs: Vec<String>,
    pub next_log_line: usize,
    pub overwrite_path: Option<String>,
    pub form_button_focus: Option<FormArrayButtonFocus>,
    pub schema_collapsed: bool,
    pub form_collapsed: bool,
    /// One main column (Schema, Form, or output stack) spans the full width under Schema Path.
    pub main_fullwidth: Option<MainFullwidthPane>,
    /// JSON path key ([`form_path_key`]) → selected `oneOf` branch index for form generation.
    pub one_of_choices: HashMap<String, usize>,
    /// High-resolution mouse wheel: move form cursor one row per this many ticks.
    pub(crate) form_mouse_scroll_accum: i8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FormFocusTarget {
    Field(usize),
    Button(FormArrayButtonFocus),
}

impl AppState {
    pub fn new() -> Self {
        Self::new_with_mode(AppMode::Standard)
    }

    pub fn new_with_mode(app_mode: AppMode) -> Self {
        let mut state = Self {
            app_mode,
            screen_mode: ScreenMode::Edit,
            input_mode: InputMode::Normal,
            active_pane: PaneId::Form,
            schema_path: SchemaPathState::default(),
            schema_text: String::new(),
            schema_json: Value::Null,
            output_json: Value::Null,
            filter_text: ".".to_owned(),
            filter_outcome: FilterOutcome::default(),
            validation: ValidationSummary::default(),
            form_fields: Vec::new(),
            field_errors: HashMap::new(),
            schema_error: None,
            pane_cursors: PaneId::ALL
                .into_iter()
                .map(|pane| (pane, Cursor::default()))
                .collect(),
            pane_histories: PaneId::ALL
                .into_iter()
                .map(|pane| (pane, PaneHistory::default()))
                .collect(),
            visual_anchor: None,
            schema_completion: None,
            pending_g: false,
            pending_d: false,
            pending_z: false,
            logs: Vec::new(),
            next_log_line: 1,
            overwrite_path: None,
            form_button_focus: None,
            schema_collapsed: false,
            form_collapsed: false,
            main_fullwidth: None,
            one_of_choices: HashMap::new(),
            form_mouse_scroll_accum: 0,
        };
        if let Err(err) = state.set_schema_source(state.schema_path.schema_source.clone()) {
            state.schema_error = Some(SchemaError {
                message: err.to_string(),
                line: None,
                column: None,
            });
            state.log(format!("schema load error: {err}"));
        }
        state
    }

    pub fn visible_panes(&self) -> &'static [PaneId] {
        match self.app_mode {
            AppMode::Standard => &PaneId::STANDARD,
            AppMode::Editor => &PaneId::ALL,
        }
    }

    pub fn is_pane_visible(&self, pane: PaneId) -> bool {
        self.visible_panes().contains(&pane)
    }

    pub fn pane_title(&self, pane: PaneId) -> String {
        let index = self
            .visible_panes()
            .iter()
            .position(|candidate| *candidate == pane)
            .map(|idx| idx + 1);
        match index {
            Some(index) => format!("[{index}] {}", pane.base_title()),
            None => pane.base_title().to_owned(),
        }
    }

    pub fn pane_line_progress(&self, pane: PaneId) -> Option<(usize, usize)> {
        match pane {
            PaneId::Schema | PaneId::Output | PaneId::Log => {
                let total = self.pane_line_count(pane).max(1);
                let current = self.cursor_row(pane).min(total.saturating_sub(1)) + 1;
                Some((current, total))
            }
            _ => None,
        }
    }

    pub fn is_pane_collapsed(&self, pane: PaneId) -> bool {
        match pane {
            PaneId::Schema => self.schema_collapsed,
            PaneId::Form => self.form_collapsed,
            _ => false,
        }
    }

    pub fn toggle_active_pane_collapse(&mut self) {
        match self.active_pane {
            PaneId::Schema => self.schema_collapsed = !self.schema_collapsed,
            PaneId::Form => self.form_collapsed = !self.form_collapsed,
            _ => {}
        }
    }

    pub fn collapse_active_pane(&mut self) {
        match self.active_pane {
            PaneId::Schema => self.schema_collapsed = true,
            PaneId::Form => self.form_collapsed = true,
            _ => {}
        }
    }

    pub fn expand_active_pane(&mut self) {
        match self.active_pane {
            PaneId::Schema => self.schema_collapsed = false,
            PaneId::Form => self.form_collapsed = false,
            _ => {}
        }
    }

    pub fn toggle_main_fullwidth_for_active_pane(&mut self) {
        let target = match self.active_pane {
            PaneId::Schema if self.is_pane_visible(PaneId::Schema) => Some(MainFullwidthPane::Schema),
            PaneId::Form => Some(MainFullwidthPane::Form),
            PaneId::Output | PaneId::Filter | PaneId::OutputPath => Some(MainFullwidthPane::OutputColumn),
            _ => None,
        };
        let Some(target) = target else {
            self.log(
                "fullwidth: focus Schema, Form, Output Path, Filter, or Output, then type z w"
                    .to_owned(),
            );
            return;
        };
        if self.main_fullwidth == Some(target) {
            self.main_fullwidth = None;
            self.log("main area layout reset".to_owned());
        } else {
            self.main_fullwidth = Some(target);
            self.log(format!(
                "main area: {} full width (z w again to reset)",
                match target {
                    MainFullwidthPane::Schema => "Schema",
                    MainFullwidthPane::Form => "Form",
                    MainFullwidthPane::OutputColumn => "Output path / Filter / Output",
                }
            ));
        }
    }

    pub fn set_schema_source(&mut self, source: String) -> Result<()> {
        let schema_text = match bundled::get_schema(&source) {
            Some(text) => text.to_owned(),
            None => fs::read_to_string(&source)?,
        };
        self.schema_completion = None;
        self.schema_path.schema_source = source;
        self.schema_text = schema_text;
        self.rebuild_from_schema_text()
    }

    pub fn commit_active_editor(&mut self) -> Result<()> {
        match self.active_pane {
            PaneId::SchemaPath => {
                let source = self.schema_path.schema_source.clone();
                self.set_schema_source(source)?;
            }
            PaneId::OutputPath => {
                self.log(format!(
                    "output path set to {}",
                    self.schema_path.output_path
                ));
            }
            PaneId::Schema => {
                self.rebuild_from_schema_text()?;
            }
            PaneId::Form => {
                self.commit_form_field()?;
            }
            PaneId::Filter => {
                self.refresh_filter();
            }
            PaneId::Output | PaneId::Log => {}
        }
        Ok(())
    }

    pub fn save_output(&mut self) -> Result<()> {
        let path = self.schema_path.output_path.trim();
        if path.is_empty() {
            self.log("save error: output path is empty");
            return Ok(());
        }
        if self.overwrite_path.as_deref() != Some(path) && Path::new(path).exists() {
            self.overwrite_path = Some(path.to_owned());
            self.screen_mode = ScreenMode::ConfirmOverwrite;
            self.log(format!("overwrite confirmation required for {path}"));
            return Ok(());
        }
        let contents = pretty_json(&self.output_json)?;
        fs::write_string(path, &contents)?;
        self.overwrite_path = None;
        self.screen_mode = ScreenMode::Edit;
        self.log(format!("wrote output json to {path}"));
        Ok(())
    }

    pub fn confirm_overwrite(&mut self) -> Result<()> {
        if let Some(path) = self.overwrite_path.clone() {
            self.schema_path.output_path = path;
        }
        self.save_output()
    }

    pub fn cancel_overwrite(&mut self) {
        if let Some(path) = self.overwrite_path.take() {
            self.log(format!("cancelled overwrite for {path}"));
        }
        self.screen_mode = ScreenMode::Edit;
    }

    pub fn rebuild_from_schema_text(&mut self) -> Result<()> {
        let parsed: Value = match serde_json::from_str(&self.schema_text) {
            Ok(parsed) => parsed,
            Err(err) => {
                self.set_schema_error(
                    format!("invalid schema json: {err}"),
                    Some(err.line()),
                    Some(err.column()),
                );
                return Ok(());
            }
        };
        self.one_of_choices.clear();
        let default_output = match default_value_for_schema(&parsed, &parsed) {
            Ok(default_output) => default_output,
            Err(err) => {
                self.set_schema_error(format!("invalid schema: {err}"), None, None);
                return Ok(());
            }
        };
        let fields = build_form_fields_with(&parsed, &parsed, &default_output, Some(&self.one_of_choices));
        let validation = match validate_document(&parsed, &default_output) {
            Ok(validation) => validation,
            Err(err) => {
                self.set_schema_error(format!("invalid schema: {err}"), None, None);
                return Ok(());
            }
        };

        self.schema_json = parsed;
        self.output_json = default_output;
        self.form_fields = fields;
        self.field_errors.clear();
        self.schema_error = None;
        self.validation = validation;
        self.refresh_filter();
        self.log("schema rebuilt");
        self.reset_pane_history(PaneId::Schema);
        self.reset_pane_history(PaneId::Form);
        self.clamp_cursor_all();
        Ok(())
    }

    pub fn refresh_filter(&mut self) {
        self.filter_outcome = evaluate_filter(&self.output_json, &self.filter_text);
        self.clamp_cursor(PaneId::Output);
    }

    pub fn schema_candidates(&self) -> Vec<String> {
        self.schema_candidate_values()
            .into_iter()
            .map(|candidate| {
                let marker = if self.schema_path.schema_source == candidate {
                    ">"
                } else {
                    " "
                };
                if bundled::get_schema(&candidate).is_some() {
                    format!("{marker} bundle  {candidate}")
                } else {
                    format!("{marker} file    {candidate}")
                }
            })
            .take(8)
            .collect()
    }

    pub fn complete_schema_path(&mut self) {
        self.advance_schema_completion(1);
    }

    pub fn complete_schema_path_prev(&mut self) {
        self.advance_schema_completion(-1);
    }

    fn schema_candidate_values(&self) -> Vec<String> {
        let mut candidates: Vec<String> = Vec::new();
        let source = self.schema_path.schema_source.trim();

        for name in bundled::names() {
            if source.is_empty() || name.starts_with(source) {
                candidates.push((*name).to_owned());
            }
        }

        let (dir, prefix, base_display) = path_completion_context(source);
        if let Ok(entries) = fs::list_dir(&dir) {
            for entry in entries
                .into_iter()
                .filter(|entry| is_schema_path_entry(entry))
                .filter(|entry| entry.starts_with(&prefix))
                .take(8)
            {
                candidates.push(format!("{}{}", base_display, entry));
            }
        }

        candidates.truncate(8);
        candidates
    }

    pub fn enter_insert_mode(&mut self, append: bool) {
        if !self.is_editable_pane(self.active_pane) {
            self.log(format!(
                "pane {} is read-only",
                self.active_pane.base_title()
            ));
            return;
        }
        if self.active_pane == PaneId::Form && self.form_button_focus.is_some() {
            return;
        }
        if append {
            let pane = self.active_pane;
            let max_col = self.max_insert_col(pane, self.cursor_row(pane));
            if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
                cursor.col = (cursor.col + 1).min(max_col);
            }
        }
        self.input_mode = InputMode::Insert;
        self.visual_anchor = None;
        self.clamp_cursor(self.active_pane);
    }

    pub fn enter_visual_mode(&mut self) {
        self.input_mode = InputMode::Visual;
        self.visual_anchor = Some(SelectionAnchor {
            pane: self.active_pane,
            row: self.cursor_row(self.active_pane),
            col: self.cursor_col(self.active_pane),
        });
    }

    pub fn exit_mode(&mut self) {
        self.input_mode = InputMode::Normal;
        self.visual_anchor = None;
    }

    pub fn focus_next_pane(&mut self) {
        let panes = self.visible_panes();
        let index = panes
            .iter()
            .position(|pane| *pane == self.active_pane)
            .unwrap_or(0);
        self.active_pane = panes[(index + 1) % panes.len()];
        self.exit_mode();
        if self.active_pane != PaneId::SchemaPath {
            self.schema_completion = None;
        }
    }

    pub fn focus_prev_pane(&mut self) {
        let panes = self.visible_panes();
        let index = panes
            .iter()
            .position(|pane| *pane == self.active_pane)
            .unwrap_or(0);
        self.active_pane = panes[(index + panes.len() - 1) % panes.len()];
        self.exit_mode();
        if self.active_pane != PaneId::SchemaPath {
            self.schema_completion = None;
        }
    }

    pub fn focus_next_form_field(&mut self) {
        self.focus_form_target_by_offset(1);
    }

    pub fn focus_prev_form_field(&mut self) {
        self.focus_form_target_by_offset(-1);
    }

    pub fn focus_pane_at(&mut self, pane: PaneId) {
        if !self.is_pane_visible(pane) {
            return;
        }
        if pane != PaneId::Form {
            self.form_mouse_scroll_accum = 0;
        }
        self.active_pane = pane;
        self.exit_mode();
        if pane != PaneId::SchemaPath {
            self.schema_completion = None;
        }
    }

    pub fn is_pane_collapsible(&self, pane: PaneId) -> bool {
        matches!(pane, PaneId::Schema | PaneId::Form)
    }

    pub fn set_pane_cursor(&mut self, pane: PaneId, row: usize, col: usize) {
        if pane == PaneId::Form {
            self.form_button_focus = None;
        }
        if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
            cursor.row = row;
            cursor.col = col;
        }
        self.clamp_cursor(pane);
    }

    fn focus_form_target_by_offset(&mut self, delta: isize) {
        self.active_pane = PaneId::Form;
        let targets = self.form_focus_targets();
        if targets.is_empty() {
            self.exit_mode();
            return;
        }
        let current = self.current_form_target_index(&targets);
        let next = if delta >= 0 {
            (current + delta as usize).min(targets.len().saturating_sub(1))
        } else {
            current.saturating_sub(delta.unsigned_abs())
        };
        self.apply_form_focus_target(targets[next].clone());
        self.input_mode = InputMode::Insert;
        self.visual_anchor = None;
    }

    fn move_form_focus_vertical(&mut self, delta: isize) -> bool {
        if self.active_pane != PaneId::Form || self.input_mode == InputMode::Insert {
            return false;
        }
        let targets = self.form_focus_targets();
        if targets.is_empty() {
            return false;
        }
        let current = self.current_form_target_index(&targets);
        let next = if delta >= 0 {
            (current + delta as usize).min(targets.len().saturating_sub(1))
        } else {
            current.saturating_sub(delta.unsigned_abs())
        };
        if next == current {
            return true;
        }
        self.apply_form_focus_target(targets[next].clone());
        true
    }

    pub fn yank_selection(&mut self) -> Result<()> {
        let Some(anchor) = self.visual_anchor else {
            return Ok(());
        };
        if anchor.pane != self.active_pane {
            self.log("visual selection is limited to a single pane");
            return Ok(());
        }
        let text = self.selection_text(anchor);
        clipboard::set_text(&text)?;
        self.log(format!(
            "yanked {} chars from {}",
            text.chars().count(),
            self.active_pane.base_title()
        ));
        Ok(())
    }

    pub fn move_cursor_up(&mut self) {
        if self.is_pane_collapsed(self.active_pane) {
            return;
        }
        if self.move_form_focus_vertical(-1) {
            return;
        }
        self.form_button_focus = None;
        if self.move_form_cursor_vertical(-1) {
            return;
        }
        let pane = self.active_pane;
        if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
            cursor.row = cursor.row.saturating_sub(1);
        }
        self.clamp_cursor(pane);
    }

    pub fn move_cursor_down(&mut self) {
        if self.is_pane_collapsed(self.active_pane) {
            return;
        }
        if self.move_form_focus_vertical(1) {
            return;
        }
        self.form_button_focus = None;
        if self.move_form_cursor_vertical(1) {
            return;
        }
        let pane = self.active_pane;
        if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
            cursor.row += 1;
        }
        self.clamp_cursor(pane);
    }

    pub fn move_cursor_left(&mut self) {
        if self.is_pane_collapsed(self.active_pane) {
            return;
        }
        self.form_button_focus = None;
        if self.cycle_form_enum(-1) {
            return;
        }
        let pane = self.active_pane;
        if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
            cursor.col = cursor.col.saturating_sub(1);
        }
        self.clamp_cursor(pane);
    }

    pub fn move_cursor_right(&mut self) {
        if self.is_pane_collapsed(self.active_pane) {
            return;
        }
        self.form_button_focus = None;
        if self.cycle_form_enum(1) {
            return;
        }
        let pane = self.active_pane;
        if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
            cursor.col += 1;
        }
        self.clamp_cursor(pane);
    }

    pub fn move_cursor_line_start(&mut self) {
        if self.is_pane_collapsed(self.active_pane) {
            return;
        }
        self.form_button_focus = None;
        if let Some(cursor) = self.pane_cursors.get_mut(&self.active_pane) {
            cursor.col = 0;
        }
        self.clamp_cursor(self.active_pane);
    }

    pub fn move_cursor_line_end(&mut self) {
        if self.is_pane_collapsed(self.active_pane) {
            return;
        }
        self.form_button_focus = None;
        let pane = self.active_pane;
        let end = self.max_cursor_col(pane, self.cursor_row(pane));
        if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
            cursor.col = end;
        }
    }

    pub fn move_cursor_top(&mut self) {
        if self.is_pane_collapsed(self.active_pane) {
            return;
        }
        self.form_button_focus = None;
        let pane = self.active_pane;
        if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
            cursor.row = 0;
            cursor.col = 0;
        }
        self.clamp_cursor(pane);
    }

    pub fn move_cursor_bottom(&mut self) {
        if self.is_pane_collapsed(self.active_pane) {
            return;
        }
        self.form_button_focus = None;
        let pane = self.active_pane;
        let bottom = self.pane_line_count(pane).saturating_sub(1);
        if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
            cursor.row = bottom;
            cursor.col = 0;
        }
        self.clamp_cursor(pane);
    }

    pub fn move_cursor_word_forward(&mut self) {
        if self.is_pane_collapsed(self.active_pane) {
            return;
        }
        self.form_button_focus = None;
        let pane = self.active_pane;
        let (row, col) = next_word_position(
            &self.pane_lines(pane),
            self.cursor_row(pane),
            self.cursor_col(pane),
        );
        if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
            cursor.row = row;
            cursor.col = col;
        }
        self.clamp_cursor(pane);
    }

    pub fn move_cursor_word_end(&mut self) {
        if self.is_pane_collapsed(self.active_pane) {
            return;
        }
        self.form_button_focus = None;
        let pane = self.active_pane;
        let (row, col) = next_word_end_position(
            &self.pane_lines(pane),
            self.cursor_row(pane),
            self.cursor_col(pane),
        );
        if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
            cursor.row = row;
            cursor.col = col;
        }
        self.clamp_cursor(pane);
    }

    pub fn move_cursor_word_backward(&mut self) {
        if self.is_pane_collapsed(self.active_pane) {
            return;
        }
        self.form_button_focus = None;
        let pane = self.active_pane;
        let (row, col) = prev_word_position(
            &self.pane_lines(pane),
            self.cursor_row(pane),
            self.cursor_col(pane),
        );
        if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
            cursor.row = row;
            cursor.col = col;
        }
        self.clamp_cursor(pane);
    }

    pub fn insert_char(&mut self, c: char) {
        self.record_edit_state(self.active_pane);
        match self.active_pane {
            PaneId::SchemaPath => self.insert_char_schema_path(c),
            PaneId::OutputPath => self.insert_char_output_path(c),
            PaneId::Schema => self.insert_char_schema(c),
            PaneId::Form => self.insert_char_form(c),
            PaneId::Filter => self.insert_char_filter(c),
            PaneId::Output | PaneId::Log => {}
        }
    }

    pub fn insert_newline(&mut self) {
        self.record_edit_state(self.active_pane);
        match self.active_pane {
            PaneId::Schema => self.insert_newline_schema(),
            PaneId::Form => self.insert_newline_form(),
            PaneId::Log | PaneId::Output => {}
            PaneId::SchemaPath | PaneId::OutputPath | PaneId::Filter => {}
        }
    }

    pub fn indent_schema_line(&mut self) {
        self.record_edit_state(PaneId::Schema);
        self.indent_text_pane_line(PaneId::Schema);
    }

    pub fn outdent_schema_line(&mut self) {
        self.record_edit_state(PaneId::Schema);
        self.outdent_text_pane_line(PaneId::Schema);
    }

    pub fn backspace(&mut self) {
        self.record_edit_state(self.active_pane);
        match self.active_pane {
            PaneId::SchemaPath => self.backspace_schema_path(),
            PaneId::OutputPath => self.backspace_output_path(),
            PaneId::Schema => self.backspace_schema(),
            PaneId::Form => self.backspace_form(),
            PaneId::Filter => self.backspace_filter(),
            PaneId::Output | PaneId::Log => {}
        }
    }

    pub fn delete_char(&mut self) {
        self.record_edit_state(self.active_pane);
        match self.active_pane {
            PaneId::SchemaPath => self.delete_char_schema_path(),
            PaneId::OutputPath => self.delete_char_output_path(),
            PaneId::Schema => self.delete_char_schema(),
            PaneId::Form => self.delete_char_form(),
            PaneId::Filter => self.delete_char_filter(),
            PaneId::Output | PaneId::Log => {}
        }
    }

    pub fn delete_to_line_end(&mut self) {
        self.record_edit_state(self.active_pane);
        match self.active_pane {
            PaneId::SchemaPath => self.delete_to_line_end_schema_path(),
            PaneId::OutputPath => self.delete_to_line_end_output_path(),
            PaneId::Schema => self.delete_to_line_end_schema(),
            PaneId::Form => self.delete_to_line_end_form(),
            PaneId::Filter => self.delete_to_line_end_filter(),
            PaneId::Output | PaneId::Log => {}
        }
    }

    pub fn delete_to_line_start(&mut self) {
        self.record_edit_state(self.active_pane);
        match self.active_pane {
            PaneId::SchemaPath => self.delete_to_line_start_schema_path(),
            PaneId::OutputPath => self.delete_to_line_start_output_path(),
            PaneId::Schema => self.delete_to_line_start_schema(),
            PaneId::Form => self.delete_to_line_start_form(),
            PaneId::Filter => self.delete_to_line_start_filter(),
            PaneId::Output | PaneId::Log => {}
        }
    }

    pub fn delete_word_forward(&mut self) {
        self.record_edit_state(self.active_pane);
        match self.active_pane {
            PaneId::SchemaPath => self.delete_word_forward_schema_path(),
            PaneId::OutputPath => self.delete_word_forward_output_path(),
            PaneId::Schema => self.delete_word_forward_schema(),
            PaneId::Form => self.delete_word_forward_form(),
            PaneId::Filter => self.delete_word_forward_filter(),
            PaneId::Output | PaneId::Log => {}
        }
    }

    pub fn open_line_below(&mut self) {
        self.record_edit_state(self.active_pane);
        match self.active_pane {
            PaneId::Schema => self.open_line_in_text_pane(PaneId::Schema, false),
            PaneId::Filter => self.open_line_in_text_pane(PaneId::Filter, false),
            _ => {}
        }
    }

    pub fn open_line_above(&mut self) {
        self.record_edit_state(self.active_pane);
        match self.active_pane {
            PaneId::Schema => self.open_line_in_text_pane(PaneId::Schema, true),
            PaneId::Filter => self.open_line_in_text_pane(PaneId::Filter, true),
            _ => {}
        }
    }

    pub fn delete_line(&mut self) {
        self.record_edit_state(self.active_pane);
        match self.active_pane {
            PaneId::Schema => self.delete_line_in_text_pane(PaneId::Schema),
            PaneId::Filter => self.delete_line_in_text_pane(PaneId::Filter),
            _ => {}
        }
    }

    pub fn add_array_item_at_cursor(&mut self) -> Result<()> {
        if self.active_pane != PaneId::Form {
            return Ok(());
        }
        self.record_edit_state(PaneId::Form);
        let array_path = if let Some(focus) = &self.form_button_focus {
            focus.array_path.clone()
        } else {
            let row = self.cursor_row(PaneId::Form);
            let Some(field) = self.form_fields.get(row) else {
                return Ok(());
            };
            let Some(array_path) = array_path_for_form_field(field) else {
                self.log("current field is not inside an array");
                return Ok(());
            };
            array_path
        };
        let next_index = self
            .output_json
            .pointer(&json_pointer(&array_path))
            .and_then(Value::as_array)
            .map(|items| items.len())
            .unwrap_or(0);
        let next_json = append_array_item(
            &self.output_json,
            &self.schema_json,
            &array_path,
            Some(&self.one_of_choices),
        )?;
        let validation = validate_document(&self.schema_json, &next_json)?;
        self.output_json = next_json;
        self.form_fields = build_form_fields_with(
            &self.schema_json,
            &self.schema_json,
            &self.output_json,
            Some(&self.one_of_choices),
        );
        self.validation = validation;
        self.refresh_filter();
        if let Some(new_row) = self.form_fields.iter().position(|candidate| {
            candidate.path.starts_with(&array_path)
                && candidate.path.get(array_path.len()) == Some(&next_index.to_string())
        }) {
            self.set_pane_cursor(PaneId::Form, new_row, 0);
        }
        self.log(format!("added item to {}", array_path.join(".")));
        Ok(())
    }

    pub fn remove_array_item_at_cursor(&mut self) -> Result<()> {
        if self.active_pane != PaneId::Form {
            return Ok(());
        }
        self.record_edit_state(PaneId::Form);
        let row = self.cursor_row(PaneId::Form);
        let Some(field) = self.form_fields.get(row) else {
            return Ok(());
        };
        let Some((array_path, item_index)) = array_location_for_field(&field.path) else {
            self.log("current field is not inside an array");
            return Ok(());
        };

        let next_json = remove_array_item(
            &self.output_json,
            &self.schema_json,
            &array_path,
            item_index,
            Some(&self.one_of_choices),
        )?;
        let validation = validate_document(&self.schema_json, &next_json)?;
        self.output_json = next_json;
        self.form_fields = build_form_fields_with(
            &self.schema_json,
            &self.schema_json,
            &self.output_json,
            Some(&self.one_of_choices),
        );
        self.validation = validation;
        self.refresh_filter();

        let target_index = item_index.min(
            self.output_json
                .pointer(&json_pointer(&array_path))
                .and_then(Value::as_array)
                .map(|items| items.len().saturating_sub(1))
                .unwrap_or(0),
        );
        if let Some(new_row) = self.form_fields.iter().position(|candidate| {
            candidate.path.starts_with(&array_path)
                && candidate.path.get(array_path.len()) == Some(&target_index.to_string())
        }) {
            self.set_pane_cursor(PaneId::Form, new_row, 0);
        } else {
            self.set_pane_cursor(PaneId::Form, row.saturating_sub(1), 0);
        }
        self.log(format!(
            "removed item {} from {}",
            item_index,
            array_path.join(".")
        ));
        Ok(())
    }

    pub fn reset_form_to_defaults(&mut self) -> Result<()> {
        if self.active_pane != PaneId::Form {
            return Ok(());
        }
        self.record_edit_state(PaneId::Form);
        self.one_of_choices.clear();
        let default_output = default_value_for_schema(&self.schema_json, &self.schema_json)?;
        let validation = validate_document(&self.schema_json, &default_output)?;
        self.output_json = default_output;
        self.form_fields = build_form_fields_with(
            &self.schema_json,
            &self.schema_json,
            &self.output_json,
            Some(&self.one_of_choices),
        );
        self.validation = validation;
        self.field_errors.clear();
        self.form_button_focus = None;
        self.refresh_filter();
        self.clamp_cursor(PaneId::Form);
        self.log("reset form to schema defaults");
        Ok(())
    }

    pub fn focus_form_button(&mut self, array_path: Vec<String>, kind: FormArrayButtonKind) {
        self.active_pane = PaneId::Form;
        self.input_mode = InputMode::Normal;
        self.visual_anchor = None;
        self.form_button_focus = Some(FormArrayButtonFocus { array_path, kind });
    }

    pub fn activate_form_button(
        &mut self,
        array_path: Vec<String>,
        kind: FormArrayButtonKind,
    ) -> Result<()> {
        self.focus_form_button(array_path, kind);
        match kind {
            FormArrayButtonKind::Add => self.add_array_item_at_cursor(),
            FormArrayButtonKind::Remove => self.remove_array_item_at_cursor(),
        }
    }

    pub fn is_form_button_focused(&self, array_path: &[String], kind: FormArrayButtonKind) -> bool {
        self.form_button_focus
            .as_ref()
            .map(|focus| focus.kind == kind && focus.array_path == array_path)
            .unwrap_or(false)
    }

    pub fn focused_form_button(&self) -> Option<FormArrayButtonFocus> {
        self.form_button_focus.clone()
    }

    pub fn current_form_breadcrumb(&self) -> Option<String> {
        if self.active_pane != PaneId::Form && self.form_button_focus.is_none() {
            return None;
        }
        if let Some(focus) = &self.form_button_focus {
            let action = match focus.kind {
                FormArrayButtonKind::Add => "Add Item",
                FormArrayButtonKind::Remove => "Remove Item",
            };
            let path = if focus.array_path.is_empty() {
                "root".to_owned()
            } else {
                focus.array_path.join(" > ")
            };
            return Some(format!("{path} > {action}"));
        }

        let row = self.cursor_row(PaneId::Form);
        let field = self.form_fields.get(row)?;
        let path = if field.path.is_empty() {
            field.label.clone()
        } else {
            field.path.join(" > ")
        };
        Some(path)
    }

    pub fn undo(&mut self) {
        let pane = self.active_pane;
        let Some(current) = self.capture_edit_state(pane) else {
            return;
        };
        let Some(previous) = self
            .pane_histories
            .get_mut(&pane)
            .and_then(|history| history.undo.pop())
        else {
            return;
        };
        if let Some(history) = self.pane_histories.get_mut(&pane) {
            history.redo.push(current);
        }
        self.restore_edit_state(pane, previous);
        self.log(format!("undo {}", pane.base_title()));
    }

    pub fn redo(&mut self) {
        let pane = self.active_pane;
        let Some(current) = self.capture_edit_state(pane) else {
            return;
        };
        let Some(next) = self
            .pane_histories
            .get_mut(&pane)
            .and_then(|history| history.redo.pop())
        else {
            return;
        };
        if let Some(history) = self.pane_histories.get_mut(&pane) {
            history.undo.push(current);
        }
        self.restore_edit_state(pane, next);
        self.log(format!("redo {}", pane.base_title()));
    }

    pub fn footer_text(&self) -> String {
        format!(
            "mode={:?}/{:?}/{:?} focus={} validation={}",
            self.app_mode,
            self.screen_mode,
            self.input_mode,
            self.active_pane.base_title(),
            self.validation.status_line()
        )
    }

    fn insert_char_schema_path(&mut self, c: char) {
        self.schema_completion = None;
        let col = self.cursor_col(PaneId::SchemaPath);
        insert_char_at(&mut self.schema_path.schema_source, col, c);
        if let Some(cursor) = self.pane_cursors.get_mut(&PaneId::SchemaPath) {
            cursor.col += 1;
        }
        self.clamp_cursor(PaneId::SchemaPath);
    }

    fn backspace_schema_path(&mut self) {
        self.schema_completion = None;
        let col = self.cursor_col(PaneId::SchemaPath);
        let deleted = delete_char_before(&mut self.schema_path.schema_source, col);
        if deleted {
            self.move_cursor_left();
        }
        self.clamp_cursor(PaneId::SchemaPath);
    }

    fn delete_char_schema_path(&mut self) {
        self.schema_completion = None;
        let col = self.cursor_col(PaneId::SchemaPath);
        delete_char_at(&mut self.schema_path.schema_source, col);
        self.clamp_cursor(PaneId::SchemaPath);
    }

    fn delete_to_line_end_schema_path(&mut self) {
        self.schema_completion = None;
        let col = self.cursor_col(PaneId::SchemaPath);
        delete_to_line_end(&mut self.schema_path.schema_source, col);
        self.clamp_cursor(PaneId::SchemaPath);
    }

    fn delete_to_line_start_schema_path(&mut self) {
        self.schema_completion = None;
        let col = self.cursor_col(PaneId::SchemaPath);
        delete_char_range(&mut self.schema_path.schema_source, 0, col);
        if let Some(cursor) = self.pane_cursors.get_mut(&PaneId::SchemaPath) {
            cursor.col = 0;
        }
        self.clamp_cursor(PaneId::SchemaPath);
    }

    fn delete_word_forward_schema_path(&mut self) {
        self.schema_completion = None;
        let col = self.cursor_col(PaneId::SchemaPath);
        let end = next_word_start(&self.schema_path.schema_source, col);
        delete_char_range(&mut self.schema_path.schema_source, col, end);
        self.clamp_cursor(PaneId::SchemaPath);
    }

    fn insert_char_output_path(&mut self, c: char) {
        let col = self.cursor_col(PaneId::OutputPath);
        insert_char_at(&mut self.schema_path.output_path, col, c);
        if let Some(cursor) = self.pane_cursors.get_mut(&PaneId::OutputPath) {
            cursor.col += 1;
        }
        self.clamp_cursor(PaneId::OutputPath);
    }

    fn backspace_output_path(&mut self) {
        let col = self.cursor_col(PaneId::OutputPath);
        let deleted = delete_char_before(&mut self.schema_path.output_path, col);
        if deleted {
            if let Some(cursor) = self.pane_cursors.get_mut(&PaneId::OutputPath) {
                cursor.col = cursor.col.saturating_sub(1);
            }
        }
        self.clamp_cursor(PaneId::OutputPath);
    }

    fn delete_char_output_path(&mut self) {
        let col = self.cursor_col(PaneId::OutputPath);
        delete_char_at(&mut self.schema_path.output_path, col);
        self.clamp_cursor(PaneId::OutputPath);
    }

    fn delete_to_line_end_output_path(&mut self) {
        let col = self.cursor_col(PaneId::OutputPath);
        delete_to_line_end(&mut self.schema_path.output_path, col);
        self.clamp_cursor(PaneId::OutputPath);
    }

    fn delete_to_line_start_output_path(&mut self) {
        let col = self.cursor_col(PaneId::OutputPath);
        delete_char_range(&mut self.schema_path.output_path, 0, col);
        if let Some(cursor) = self.pane_cursors.get_mut(&PaneId::OutputPath) {
            cursor.col = 0;
        }
        self.clamp_cursor(PaneId::OutputPath);
    }

    fn delete_word_forward_output_path(&mut self) {
        let col = self.cursor_col(PaneId::OutputPath);
        let end = next_word_start(&self.schema_path.output_path, col);
        delete_char_range(&mut self.schema_path.output_path, col, end);
        self.clamp_cursor(PaneId::OutputPath);
    }

    fn insert_char_schema(&mut self, c: char) {
        let pane = PaneId::Schema;
        self.schema_text = insert_char_into_multiline(
            &self.schema_text,
            self.cursor_row(pane),
            self.cursor_col(pane),
            c,
        );
        if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
            cursor.col += 1;
        }
        self.clamp_cursor(pane);
    }

    fn insert_newline_schema(&mut self) {
        let pane = PaneId::Schema;
        self.schema_text = insert_newline_into_multiline(
            &self.schema_text,
            self.cursor_row(pane),
            self.cursor_col(pane),
        );
        if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
            cursor.row += 1;
            cursor.col = 0;
        }
        self.clamp_cursor(pane);
    }

    fn backspace_schema(&mut self) {
        let pane = PaneId::Schema;
        let (updated, moved, row, col) = delete_char_from_multiline(
            &self.schema_text,
            self.cursor_row(pane),
            self.cursor_col(pane),
        );
        if moved {
            self.schema_text = updated;
            if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
                cursor.row = row;
                cursor.col = col;
            }
            self.clamp_cursor(pane);
        }
    }

    fn delete_char_schema(&mut self) {
        let pane = PaneId::Schema;
        let (updated, changed, row, col) = delete_char_at_multiline(
            &self.schema_text,
            self.cursor_row(pane),
            self.cursor_col(pane),
        );
        if changed {
            self.schema_text = updated;
            if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
                cursor.row = row;
                cursor.col = col;
            }
            self.clamp_cursor(pane);
        }
    }

    fn delete_to_line_end_schema(&mut self) {
        let pane = PaneId::Schema;
        self.schema_text = delete_to_line_end_multiline(
            &self.schema_text,
            self.cursor_row(pane),
            self.cursor_col(pane),
        );
        self.clamp_cursor(pane);
    }

    fn delete_to_line_start_schema(&mut self) {
        let pane = PaneId::Schema;
        let row = self.cursor_row(pane);
        let col = self.cursor_col(pane);
        self.schema_text = delete_range_from_multiline(&self.schema_text, row, 0, col);
        if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
            cursor.col = 0;
        }
        self.clamp_cursor(pane);
    }

    fn delete_word_forward_schema(&mut self) {
        let pane = PaneId::Schema;
        let row = self.cursor_row(pane);
        let col = self.cursor_col(pane);
        let line = self.line_text(pane, row);
        let end = next_word_start(&line, col);
        self.schema_text = delete_range_from_multiline(&self.schema_text, row, col, end);
        self.clamp_cursor(pane);
    }

    fn insert_char_form(&mut self, c: char) {
        let row = self.cursor_row(PaneId::Form);
        if self.is_form_select_row(row) {
            if c == 'h' {
                self.move_cursor_left();
                return;
            }
            if c == 'l' {
                self.move_cursor_right();
                return;
            }
            self.select_form_enum_by_char(row, c);
            self.clamp_cursor(PaneId::Form);
            return;
        }
        let offset = self.form_value_offset(row, self.cursor_col(PaneId::Form));
        if let Some(field) = self.active_form_field_mut() {
            if !form_accepts_char(field, c) {
                self.clamp_cursor(PaneId::Form);
                return;
            }
            insert_char_at(&mut field.edit_buffer, offset, c);
        }
        if let Some(cursor) = self.pane_cursors.get_mut(&PaneId::Form) {
            cursor.col += 1;
        }
        self.clamp_cursor(PaneId::Form);
    }

    fn backspace_form(&mut self) {
        let row = self.cursor_row(PaneId::Form);
        if self.is_form_select_row(row) {
            return;
        }
        let offset = self.form_value_offset(row, self.cursor_col(PaneId::Form));
        let deleted = if let Some(field) = self.active_form_field_mut() {
            delete_char_before(&mut field.edit_buffer, offset)
        } else {
            false
        };
        if deleted {
            self.move_cursor_left();
        }
        self.clamp_cursor(PaneId::Form);
    }

    fn delete_char_form(&mut self) {
        let row = self.cursor_row(PaneId::Form);
        if self.is_form_select_row(row) {
            return;
        }
        let offset = self.form_value_offset(row, self.cursor_col(PaneId::Form));
        if let Some(field) = self.active_form_field_mut() {
            delete_char_at(&mut field.edit_buffer, offset);
        }
        self.clamp_cursor(PaneId::Form);
    }

    fn delete_to_line_end_form(&mut self) {
        let row = self.cursor_row(PaneId::Form);
        if self.is_form_select_row(row) {
            return;
        }
        let offset = self.form_value_offset(row, self.cursor_col(PaneId::Form));
        if let Some(field) = self.active_form_field_mut() {
            delete_to_line_end(&mut field.edit_buffer, offset);
        }
        self.clamp_cursor(PaneId::Form);
    }

    fn delete_to_line_start_form(&mut self) {
        let row = self.cursor_row(PaneId::Form);
        if self.is_form_select_row(row) {
            return;
        }
        let offset = self.form_value_offset(row, self.cursor_col(PaneId::Form));
        if let Some(field) = self.active_form_field_mut() {
            delete_char_range(&mut field.edit_buffer, 0, offset);
        }
        if let Some(cursor) = self.pane_cursors.get_mut(&PaneId::Form) {
            cursor.col = 0;
        }
        self.clamp_cursor(PaneId::Form);
    }

    fn delete_word_forward_form(&mut self) {
        let row = self.cursor_row(PaneId::Form);
        if self.is_form_select_row(row) {
            return;
        }
        let offset = self.form_value_offset(row, self.cursor_col(PaneId::Form));
        if let Some(field) = self.active_form_field_mut() {
            let end = next_word_start(&field.edit_buffer, offset);
            delete_char_range(&mut field.edit_buffer, offset, end);
        }
        self.clamp_cursor(PaneId::Form);
    }

    fn insert_newline_form(&mut self) {
        let row = self.cursor_row(PaneId::Form);
        let Some(field) = self.form_fields.get(row) else {
            return;
        };
        if !field.multiline || field.enum_options.is_some() {
            return;
        }
        let offset = self.form_value_offset(row, self.cursor_col(PaneId::Form));
        if let Some(field) = self.active_form_field_mut() {
            insert_char_at(&mut field.edit_buffer, offset, '\n');
        }
        if let Some(cursor) = self.pane_cursors.get_mut(&PaneId::Form) {
            cursor.col += 1;
        }
        self.clamp_cursor(PaneId::Form);
    }

    fn insert_char_filter(&mut self, c: char) {
        let pane = PaneId::Filter;
        self.filter_text = insert_char_into_multiline(
            &self.filter_text,
            self.cursor_row(pane),
            self.cursor_col(pane),
            c,
        );
        if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
            cursor.col += 1;
        }
        self.clamp_cursor(pane);
    }

    fn backspace_filter(&mut self) {
        let pane = PaneId::Filter;
        let (updated, moved, row, col) = delete_char_from_multiline(
            &self.filter_text,
            self.cursor_row(pane),
            self.cursor_col(pane),
        );
        if moved {
            self.filter_text = updated;
            if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
                cursor.row = row;
                cursor.col = col;
            }
            self.clamp_cursor(pane);
        }
    }

    fn delete_char_filter(&mut self) {
        let pane = PaneId::Filter;
        let (updated, changed, row, col) = delete_char_at_multiline(
            &self.filter_text,
            self.cursor_row(pane),
            self.cursor_col(pane),
        );
        if changed {
            self.filter_text = updated;
            if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
                cursor.row = row;
                cursor.col = col;
            }
            self.clamp_cursor(pane);
        }
    }

    fn delete_to_line_end_filter(&mut self) {
        let pane = PaneId::Filter;
        self.filter_text = delete_to_line_end_multiline(
            &self.filter_text,
            self.cursor_row(pane),
            self.cursor_col(pane),
        );
        self.clamp_cursor(pane);
    }

    fn delete_to_line_start_filter(&mut self) {
        let pane = PaneId::Filter;
        let row = self.cursor_row(pane);
        let col = self.cursor_col(pane);
        self.filter_text = delete_range_from_multiline(&self.filter_text, row, 0, col);
        if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
            cursor.col = 0;
        }
        self.clamp_cursor(pane);
    }

    fn delete_word_forward_filter(&mut self) {
        let pane = PaneId::Filter;
        let row = self.cursor_row(pane);
        let col = self.cursor_col(pane);
        let line = self.line_text(pane, row);
        let end = next_word_start(&line, col);
        self.filter_text = delete_range_from_multiline(&self.filter_text, row, col, end);
        self.clamp_cursor(pane);
    }

    fn open_line_in_text_pane(&mut self, pane: PaneId, above: bool) {
        let row = self.cursor_row(pane);
        match pane {
            PaneId::Schema => {
                self.schema_text = insert_empty_line(&self.schema_text, row, above);
            }
            PaneId::Filter => {
                self.filter_text = insert_empty_line(&self.filter_text, row, above);
            }
            _ => return,
        }
        if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
            cursor.row = if above { row } else { row + 1 };
            cursor.col = 0;
        }
        self.input_mode = InputMode::Insert;
        self.clamp_cursor(pane);
    }

    fn delete_line_in_text_pane(&mut self, pane: PaneId) {
        let row = self.cursor_row(pane);
        let (next_text, next_row) = match pane {
            PaneId::Schema => delete_line_from_multiline(&self.schema_text, row),
            PaneId::Filter => delete_line_from_multiline(&self.filter_text, row),
            _ => return,
        };
        match pane {
            PaneId::Schema => self.schema_text = next_text,
            PaneId::Filter => self.filter_text = next_text,
            _ => {}
        }
        if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
            cursor.row = next_row;
            cursor.col = 0;
        }
        self.clamp_cursor(pane);
    }

    fn indent_text_pane_line(&mut self, pane: PaneId) {
        let row = self.cursor_row(pane);
        match pane {
            PaneId::Schema => self.schema_text = indent_line_in_multiline(&self.schema_text, row),
            PaneId::Filter => self.filter_text = indent_line_in_multiline(&self.filter_text, row),
            _ => return,
        }
        if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
            cursor.col += 4;
        }
        self.clamp_cursor(pane);
    }

    fn outdent_text_pane_line(&mut self, pane: PaneId) {
        let row = self.cursor_row(pane);
        let removed = match pane {
            PaneId::Schema => {
                let (text, removed) = outdent_line_in_multiline(&self.schema_text, row);
                self.schema_text = text;
                removed
            }
            PaneId::Filter => {
                let (text, removed) = outdent_line_in_multiline(&self.filter_text, row);
                self.filter_text = text;
                removed
            }
            _ => return,
        };
        if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
            cursor.col = cursor.col.saturating_sub(removed);
        }
        self.clamp_cursor(pane);
    }

    fn active_form_field_mut(&mut self) -> Option<&mut FormField> {
        let row = self.cursor_row(PaneId::Form);
        self.form_fields.get_mut(row)
    }

    pub fn form_textarea_cursor(&self, row: usize) -> (usize, usize) {
        let Some(field) = self.form_fields.get(row) else {
            return (0, 0);
        };
        offset_to_line_col(&field.edit_buffer, self.cursor_col(PaneId::Form))
    }

    fn is_form_select_row(&self, row: usize) -> bool {
        self.form_fields
            .get(row)
            .and_then(|field| field.enum_options.as_ref())
            .is_some()
    }

    fn cycle_form_enum(&mut self, delta: isize) -> bool {
        if self.active_pane != PaneId::Form {
            return false;
        }
        let row = self.cursor_row(PaneId::Form);
        let Some(field) = self.form_fields.get(row) else {
            return false;
        };
        let Some(options) = field.enum_options.as_ref() else {
            return false;
        };
        if options.is_empty() {
            return false;
        }
        if matches!(field.kind, FormFieldKind::OneOfSelector { .. }) {
            let branch_count = options.len();
            let current = options
                .iter()
                .position(|option| option == &field.edit_buffer)
                .unwrap_or(0);
            let next =
                (current as isize + delta).rem_euclid(branch_count as isize) as usize;
            let value_path = field.path.clone();
            if let Err(err) = self.apply_one_of_branch_index(value_path, next) {
                self.log(format!("oneOf branch error: {err}"));
            }
            if let Some(cursor) = self.pane_cursors.get_mut(&PaneId::Form) {
                cursor.col = 0;
            }
            return true;
        }
        let current = options
            .iter()
            .position(|option| option == &field.edit_buffer)
            .unwrap_or(0);
        let next = if delta >= 0 {
            (current + 1).min(options.len().saturating_sub(1))
        } else {
            current.saturating_sub(delta.unsigned_abs())
        };
        let next_value = options[next].clone();
        if let Some(field) = self.form_fields.get_mut(row) {
            field.edit_buffer = next_value;
        }
        if let Some(cursor) = self.pane_cursors.get_mut(&PaneId::Form) {
            cursor.col = 0;
        }
        if let Err(err) = self.commit_form_field() {
            self.log(format!("validation error on form select: {err}"));
        }
        true
    }

    fn select_form_enum_by_char(&mut self, row: usize, c: char) -> bool {
        let needle = c.to_ascii_lowercase().to_string();
        let Some(field) = self.form_fields.get(row) else {
            return false;
        };
        let Some(options) = field.enum_options.as_ref() else {
            return false;
        };
        let Some(next_index) = options
            .iter()
            .position(|option| option.to_ascii_lowercase().starts_with(&needle))
        else {
            return false;
        };
        if matches!(field.kind, FormFieldKind::OneOfSelector { .. }) {
            let value_path = field.path.clone();
            if let Err(err) = self.apply_one_of_branch_index(value_path, next_index) {
                self.log(format!("oneOf branch error: {err}"));
            }
            if let Some(cursor) = self.pane_cursors.get_mut(&PaneId::Form) {
                cursor.col = 0;
            }
            return true;
        }
        let next = options[next_index].clone();
        if let Some(field) = self.form_fields.get_mut(row) {
            field.edit_buffer = next;
        }
        if let Some(cursor) = self.pane_cursors.get_mut(&PaneId::Form) {
            cursor.col = 0;
        }
        if let Err(err) = self.commit_form_field() {
            self.log(format!("validation error on form select: {err}"));
        }
        true
    }

    fn apply_one_of_branch_index(&mut self, value_path: Vec<String>, new_index: usize) -> Result<()> {
        let choice_key = form_path_key(&value_path);
        let old_branch_index = self.one_of_choices.get(&choice_key).copied();
        self.record_edit_state(PaneId::Form);
        self.one_of_choices.insert(choice_key.clone(), new_index);
        let subtree = default_value_at_path(
            &self.schema_json,
            &value_path,
            Some(&self.one_of_choices),
        )?;
        let mut next_json = self.output_json.clone();
        replace_json_at_path(&mut next_json, &value_path, subtree)?;
        let validation = validate_document(&self.schema_json, &next_json)?;
        if !validation.is_valid {
            let message = validation
                .errors
                .first()
                .cloned()
                .unwrap_or_else(|| "unknown validation error".to_owned());
            self.log(format!("oneOf: validation error: {message}"));
            self.one_of_choices.remove(&choice_key);
            return Ok(());
        }
        self.output_json = next_json;
        self.form_fields = build_form_fields_with(
            &self.schema_json,
            &self.schema_json,
            &self.output_json,
            Some(&self.one_of_choices),
        );
        self.validation = validation;
        self.field_errors.remove(&format!("{}.oneOf", value_path.join(".")));
        self.refresh_filter();
        let path_s = value_path.join(".");
        let old_s = old_branch_index
            .map(|i| i.to_string())
            .unwrap_or_else(|| "default".to_owned());
        self.log(format!(
            "form oneOf {path_s}: branch index {old_s} -> {new_index}"
        ));
        self.clamp_cursor(PaneId::Form);
        Ok(())
    }

    fn commit_form_field(&mut self) -> Result<()> {
        let row = self.cursor_row(PaneId::Form);
        let Some(field) = self.form_fields.get(row).cloned() else {
            return Ok(());
        };
        if matches!(
            field.kind,
            FormFieldKind::OneOfSelector { .. } | FormFieldKind::ArrayPlaceholder
        ) {
            return Ok(());
        }
        let field_key = field.path.join(".");
        let old_display = Self::concise_form_log_value(&json_scalar_display_at_path(
            &self.output_json,
            &field.path,
        ));

        let next_json = match set_scalar_value(
            &self.output_json,
            &field.path,
            &field.schema_type,
            &field.edit_buffer,
        ) {
            Ok(next_json) => next_json,
            Err(err) => {
                self.field_errors.insert(field_key.clone(), err.to_string());
                self.log(format!("validation error on {field_key}: {err}"));
                return Ok(());
            }
        };

        let validation = validate_document(&self.schema_json, &next_json)?;
        if !validation.is_valid {
            let message = validation
                .errors
                .first()
                .cloned()
                .unwrap_or_else(|| "unknown validation error".to_owned());
            self.field_errors.insert(field_key.clone(), message.clone());
            self.log(format!("validation error on {field_key}: {message}"));
            return Ok(());
        }

        let new_display = Self::concise_form_log_value(&json_scalar_display_at_path(
            &next_json,
            &field.path,
        ));

        self.output_json = next_json;
        self.form_fields = build_form_fields_with(
            &self.schema_json,
            &self.schema_json,
            &self.output_json,
            Some(&self.one_of_choices),
        );
        self.validation = validation;
        self.field_errors.remove(&field_key);
        self.refresh_filter();
        self.log(format!(
            "form field {field_key}: {old_display} -> {new_display}"
        ));
        self.clamp_cursor(PaneId::Form);
        Ok(())
    }

    /// Shorten values for the log pane (newlines as `\n`, cap length).
    fn concise_form_log_value(raw: &str) -> String {
        const MAX_CHARS: usize = 100;
        let collapsed = raw.replace('\r', "").replace('\n', "\\n");
        let n = collapsed.chars().count();
        if n <= MAX_CHARS {
            collapsed
        } else {
            let prefix: String = collapsed.chars().take(MAX_CHARS).collect();
            format!("{prefix}… (+{} more chars)", n.saturating_sub(MAX_CHARS))
        }
    }

    pub fn log_error(&mut self, message: impl Into<String>) {
        self.log(message);
    }

    fn log(&mut self, message: impl Into<String>) {
        let line = self.next_log_line;
        self.next_log_line += 1;
        self.logs.push(format!("{line:04} | {}", message.into()));
        if self.logs.len() > 100 {
            self.logs.remove(0);
        }
        let last_row = self.logs.len().saturating_sub(1);
        if let Some(cursor) = self.pane_cursors.get_mut(&PaneId::Log) {
            cursor.row = last_row;
            cursor.col = 0;
        }
        self.clamp_cursor(PaneId::Log);
    }

    fn clamp_cursor_all(&mut self) {
        for pane in PaneId::ALL {
            self.clamp_cursor(pane);
        }
    }

    fn clamp_cursor(&mut self, pane: PaneId) {
        let max_row = self.pane_line_count(pane).saturating_sub(1);
        let current_row = self.cursor_row(pane).min(max_row);
        let max_col = self.max_cursor_col(pane, current_row);
        if let Some(cursor) = self.pane_cursors.get_mut(&pane) {
            cursor.row = current_row;
            cursor.col = cursor.col.min(max_col);
        }
    }

    fn cursor_row(&self, pane: PaneId) -> usize {
        self.pane_cursors
            .get(&pane)
            .map(|cursor| cursor.row)
            .unwrap_or(0)
    }

    fn cursor_col(&self, pane: PaneId) -> usize {
        self.pane_cursors
            .get(&pane)
            .map(|cursor| cursor.col)
            .unwrap_or(0)
    }

    fn pane_line_count(&self, pane: PaneId) -> usize {
        self.pane_lines(pane).len().max(1)
    }

    fn line_char_len(&self, pane: PaneId, row: usize) -> usize {
        self.line_text(pane, row).chars().count()
    }

    fn max_cursor_col(&self, pane: PaneId, row: usize) -> usize {
        if self.input_mode == InputMode::Insert && self.is_editable_pane(pane) {
            return self.max_insert_col(pane, row);
        }
        self.content_len_for_cursor(pane, row).saturating_sub(1)
    }

    fn max_insert_col(&self, pane: PaneId, row: usize) -> usize {
        self.content_len_for_cursor(pane, row)
    }

    fn content_len_for_cursor(&self, pane: PaneId, row: usize) -> usize {
        match pane {
            PaneId::Form => self
                .form_fields
                .get(row)
                .map(|field| field.edit_buffer.chars().count())
                .unwrap_or(0),
            _ => self.line_char_len(pane, row),
        }
    }

    fn line_text(&self, pane: PaneId, row: usize) -> String {
        self.pane_lines(pane).get(row).cloned().unwrap_or_default()
    }

    fn pane_lines(&self, pane: PaneId) -> Vec<String> {
        match pane {
            PaneId::SchemaPath => vec![self.schema_path.schema_source.clone()],
            PaneId::OutputPath => vec![self.schema_path.output_path.clone()],
            PaneId::Schema => text_lines(&self.schema_text),
            PaneId::Form => {
                if self.form_fields.is_empty() {
                    vec!["No editable scalar fields".to_owned()]
                } else {
                    self.form_fields
                        .iter()
                        .map(|field| {
                            let prefix = if field.required { "*" } else { " " };
                            let mut line =
                                format!("{prefix} {} = {}", field.label, field.edit_buffer);
                            if let Some(error) = self.field_errors.get(&field.key) {
                                line.push_str(&format!("  !! {error}"));
                            }
                            line
                        })
                        .collect()
                }
            }
            PaneId::Filter => text_lines(&self.filter_text),
            PaneId::Output => {
                let lines = text_lines(&self.filter_outcome.text);
                if lines.is_empty() {
                    vec![String::new()]
                } else {
                    lines
                }
            }
            PaneId::Log => {
                if self.logs.is_empty() {
                    vec![String::new()]
                } else {
                    self.logs.clone()
                }
            }
        }
    }

    fn is_editable_pane(&self, pane: PaneId) -> bool {
        let editable = matches!(
            pane,
            PaneId::SchemaPath
                | PaneId::OutputPath
                | PaneId::Schema
                | PaneId::Form
                | PaneId::Filter
        );
        if !editable {
            return false;
        }
        match self.app_mode {
            AppMode::Standard => pane != PaneId::SchemaPath && pane != PaneId::Schema,
            AppMode::Editor => true,
        }
    }

    fn form_value_offset(&self, row: usize, col: usize) -> usize {
        let Some(field) = self.form_fields.get(row) else {
            return 0;
        };
        if field.enum_options.is_some() {
            return 0;
        }
        col.min(field.edit_buffer.chars().count())
    }

    fn form_focus_targets(&self) -> Vec<FormFocusTarget> {
        let mut targets = Vec::new();
        let mut index = 0;
        while index < self.form_fields.len() {
            let field = &self.form_fields[index];
            if !matches!(field.kind, FormFieldKind::ArrayPlaceholder) {
                targets.push(FormFocusTarget::Field(index));
            }
            if let Some(array_path) = array_path_for_form_field(field) {
                let mut end = index + 1;
                while end < self.form_fields.len()
                    && array_path_for_form_field(&self.form_fields[end]).as_ref()
                        == Some(&array_path)
                {
                    if !matches!(self.form_fields[end].kind, FormFieldKind::ArrayPlaceholder) {
                        targets.push(FormFocusTarget::Field(end));
                    }
                    end += 1;
                }
                if self.can_add_array_item(&array_path) {
                    targets.push(FormFocusTarget::Button(FormArrayButtonFocus {
                        array_path: array_path.clone(),
                        kind: FormArrayButtonKind::Add,
                    }));
                }
                if self.can_remove_array_item(&array_path) {
                    targets.push(FormFocusTarget::Button(FormArrayButtonFocus {
                        array_path,
                        kind: FormArrayButtonKind::Remove,
                    }));
                }
                index = end;
            } else {
                index += 1;
            }
        }
        targets
    }

    fn current_form_target_index(&self, targets: &[FormFocusTarget]) -> usize {
        if let Some(focus) = &self.form_button_focus {
            targets
                .iter()
                .position(|target| target == &FormFocusTarget::Button(focus.clone()))
                .unwrap_or(0)
        } else {
            let row = self.cursor_row(PaneId::Form);
            targets
                .iter()
                .position(|target| target == &FormFocusTarget::Field(row))
                .unwrap_or(0)
        }
    }

    fn apply_form_focus_target(&mut self, target: FormFocusTarget) {
        match target {
            FormFocusTarget::Field(row) => self.set_pane_cursor(PaneId::Form, row, 0),
            FormFocusTarget::Button(focus) => self.focus_form_button(focus.array_path, focus.kind),
        }
    }

    fn can_add_array_item(&self, array_path: &[String]) -> bool {
        let item_count = self
            .output_json
            .pointer(&json_pointer(array_path))
            .and_then(Value::as_array)
            .map(|items| items.len())
            .unwrap_or(0);
        let ctx = ResolveCtx {
            root: &self.schema_json,
            instance: Some(&self.output_json),
            choices: Some(&self.one_of_choices),
        };
        let Ok(schema) = resolve_schema_at_path_with(array_path, ctx) else {
            return false;
        };
        let has_additional_schema = schema.get("items").is_some()
            || schema
                .get("prefixItems")
                .and_then(Value::as_array)
                .map(|items| item_count < items.len())
                .unwrap_or(false);
        if !has_additional_schema {
            return false;
        }
        match schema
            .get("maxItems")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
        {
            Some(max_items) => item_count < max_items,
            None => true,
        }
    }

    fn can_remove_array_item(&self, array_path: &[String]) -> bool {
        let item_count = self
            .output_json
            .pointer(&json_pointer(array_path))
            .and_then(Value::as_array)
            .map(|items| items.len())
            .unwrap_or(0);
        let ctx = ResolveCtx {
            root: &self.schema_json,
            instance: Some(&self.output_json),
            choices: Some(&self.one_of_choices),
        };
        let Ok(schema) = resolve_schema_at_path_with(array_path, ctx) else {
            return false;
        };
        let min_items = schema
            .get("minItems")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(0);
        item_count > min_items
    }

    pub fn form_enter_commits(&self) -> bool {
        if self.active_pane != PaneId::Form {
            return false;
        }
        let row = self.cursor_row(PaneId::Form);
        self.form_fields
            .get(row)
            .map(|field| !field.multiline && !matches!(field.kind, FormFieldKind::ArrayPlaceholder))
            .unwrap_or(false)
    }

    fn move_form_cursor_vertical(&mut self, delta: isize) -> bool {
        if self.active_pane != PaneId::Form || self.input_mode != InputMode::Insert {
            return false;
        }
        let row = self.cursor_row(PaneId::Form);
        let Some(field) = self.form_fields.get(row) else {
            return false;
        };
        if !field.multiline || field.enum_options.is_some() {
            return false;
        }
        let current_offset = self.cursor_col(PaneId::Form);
        let (line, col) = offset_to_line_col(&field.edit_buffer, current_offset);
        let target_line = if delta >= 0 {
            line.saturating_add(delta as usize)
        } else {
            line.saturating_sub(delta.unsigned_abs())
        };
        let next_offset = line_col_to_offset(&field.edit_buffer, target_line, col);
        if let Some(cursor) = self.pane_cursors.get_mut(&PaneId::Form) {
            cursor.col = next_offset;
        }
        self.clamp_cursor(PaneId::Form);
        true
    }

    fn selection_text(&self, anchor: SelectionAnchor) -> String {
        let current = SelectionAnchor {
            pane: self.active_pane,
            row: self.cursor_row(self.active_pane),
            col: self.cursor_col(self.active_pane),
        };
        let (start, end) = normalize_selection(anchor, current);
        collect_selection_text(&self.pane_lines(anchor.pane), start, end)
    }

    fn set_schema_error(&mut self, message: String, line: Option<usize>, column: Option<usize>) {
        self.schema_error = Some(SchemaError {
            message: message.clone(),
            line,
            column,
        });
        let location = match (line, column) {
            (Some(line), Some(column)) => format!("L{line}:C{column} | "),
            (Some(line), None) => format!("L{line} | "),
            _ => String::new(),
        };
        self.log(format!("schema error: {location}{message}"));
    }

    fn capture_edit_state(&self, pane: PaneId) -> Option<PaneEditState> {
        match pane {
            PaneId::SchemaPath => Some(PaneEditState::SchemaPath {
                schema_source: self.schema_path.schema_source.clone(),
            }),
            PaneId::OutputPath => Some(PaneEditState::OutputPath {
                output_path: self.schema_path.output_path.clone(),
            }),
            PaneId::Schema => Some(PaneEditState::Schema {
                schema_text: self.schema_text.clone(),
            }),
            PaneId::Form => Some(PaneEditState::Form {
                buffers: self
                    .form_fields
                    .iter()
                    .map(|field| field.edit_buffer.clone())
                    .collect(),
            }),
            PaneId::Filter => Some(PaneEditState::Filter {
                filter_text: self.filter_text.clone(),
            }),
            PaneId::Output | PaneId::Log => None,
        }
    }

    fn restore_edit_state(&mut self, pane: PaneId, state: PaneEditState) {
        match state {
            PaneEditState::SchemaPath { schema_source } if pane == PaneId::SchemaPath => {
                self.schema_path.schema_source = schema_source;
            }
            PaneEditState::OutputPath { output_path } if pane == PaneId::OutputPath => {
                self.schema_path.output_path = output_path;
            }
            PaneEditState::Schema { schema_text } if pane == PaneId::Schema => {
                self.schema_text = schema_text;
            }
            PaneEditState::Form { buffers } if pane == PaneId::Form => {
                for (field, buffer) in self.form_fields.iter_mut().zip(buffers.into_iter()) {
                    field.edit_buffer = buffer;
                }
            }
            PaneEditState::Filter { filter_text } if pane == PaneId::Filter => {
                self.filter_text = filter_text;
            }
            _ => {}
        }
        self.clamp_cursor(pane);
    }

    fn record_edit_state(&mut self, pane: PaneId) {
        let Some(snapshot) = self.capture_edit_state(pane) else {
            return;
        };
        let Some(history) = self.pane_histories.get_mut(&pane) else {
            return;
        };
        history.undo.push(snapshot);
        history.redo.clear();
        if history.undo.len() > 100 {
            history.undo.remove(0);
        }
    }

    fn reset_pane_history(&mut self, pane: PaneId) {
        if let Some(history) = self.pane_histories.get_mut(&pane) {
            history.undo.clear();
            history.redo.clear();
        }
    }

    fn advance_schema_completion(&mut self, delta: isize) {
        let source = self.schema_path.schema_source.clone();
        let mut cycle = match self.schema_completion.clone() {
            Some(cycle)
                if !cycle.candidates.is_empty()
                    && (cycle.seed == source
                        || cycle
                            .candidates
                            .iter()
                            .any(|candidate| candidate == &source)) =>
            {
                cycle
            }
            _ => {
                let candidates = self.schema_candidate_values();
                if candidates.is_empty() {
                    return;
                }
                CompletionCycle {
                    seed: source,
                    candidates,
                    index: 0,
                }
            }
        };

        if self.schema_completion.is_some() && cycle.candidates.len() > 1 {
            let len = cycle.candidates.len() as isize;
            let next = (cycle.index as isize + delta).rem_euclid(len) as usize;
            cycle.index = next;
        }

        self.schema_path.schema_source = cycle.candidates[cycle.index].clone();
        self.schema_completion = Some(cycle);
        if let Some(cursor) = self.pane_cursors.get_mut(&PaneId::SchemaPath) {
            cursor.col = self.schema_path.schema_source.chars().count();
        }
        self.clamp_cursor(PaneId::SchemaPath);
    }
}

fn array_path_for_field(path: &[String]) -> Option<Vec<String>> {
    path.iter()
        .position(|segment| segment.parse::<usize>().is_ok())
        .map(|index| path[..index].to_vec())
}

fn array_path_for_form_field(field: &FormField) -> Option<Vec<String>> {
    match field.kind {
        FormFieldKind::ArrayPlaceholder => Some(field.path.clone()),
        _ => array_path_for_field(&field.path),
    }
}

fn array_location_for_field(path: &[String]) -> Option<(Vec<String>, usize)> {
    let index = path
        .iter()
        .position(|segment| segment.parse::<usize>().is_ok())?;
    let item_index = path.get(index)?.parse::<usize>().ok()?;
    Some((path[..index].to_vec(), item_index))
}

fn json_pointer(path: &[String]) -> String {
    if path.is_empty() {
        return String::new();
    }
    let mut pointer = String::new();
    for segment in path {
        pointer.push('/');
        pointer.push_str(&segment.replace('~', "~0").replace('/', "~1"));
    }
    pointer
}

fn path_completion_context(source: &str) -> (PathBuf, String, String) {
    if source.is_empty() {
        return (PathBuf::from("."), String::new(), String::new());
    }

    let path = Path::new(source);
    if source.ends_with('/') {
        return (PathBuf::from(path), String::new(), source.to_owned());
    }

    match path.parent() {
        Some(parent) if parent != Path::new("") => {
            let prefix = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_owned();
            let mut base = parent.to_string_lossy().into_owned();
            if !base.ends_with('/') {
                base.push('/');
            }
            (parent.to_path_buf(), prefix, base)
        }
        _ => (PathBuf::from("."), source.to_owned(), String::new()),
    }
}

fn is_schema_path_entry(entry: &str) -> bool {
    entry.ends_with('/') || entry.ends_with(".json")
}

fn text_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        vec![String::new()]
    } else {
        text.lines().map(ToOwned::to_owned).collect()
    }
}

fn next_word_start(line: &str, col: usize) -> usize {
    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() {
        return 0;
    }
    let mut i = col.min(chars.len().saturating_sub(1));
    while i < chars.len() && !chars[i].is_whitespace() {
        i += 1;
    }
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    i.min(chars.len().saturating_sub(1))
}

fn prev_word_start(line: &str, col: usize) -> usize {
    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() {
        return 0;
    }
    let mut i = col.min(chars.len().saturating_sub(1));
    while i > 0 && chars[i].is_whitespace() {
        i -= 1;
    }
    while i > 0 && !chars[i - 1].is_whitespace() {
        i -= 1;
    }
    i
}

fn next_word_position(lines: &[String], row: usize, col: usize) -> (usize, usize) {
    if lines.is_empty() {
        return (0, 0);
    }
    let mut r = row.min(lines.len().saturating_sub(1));
    let c = next_word_start(&lines[r], col);
    if c < lines[r].chars().count().saturating_sub(1) || r + 1 >= lines.len() {
        return (r, c);
    }
    r += 1;
    while r < lines.len() {
        if lines[r].chars().any(|ch| !ch.is_whitespace()) {
            return (r, next_word_start(&lines[r], 0));
        }
        r += 1;
    }
    let last_row = lines.len().saturating_sub(1);
    (last_row, lines[last_row].chars().count().saturating_sub(1))
}

fn next_word_end_position(lines: &[String], row: usize, col: usize) -> (usize, usize) {
    if lines.is_empty() {
        return (0, 0);
    }

    let mut r = row.min(lines.len().saturating_sub(1));
    let mut c = col.min(lines[r].chars().count().saturating_sub(1));

    loop {
        let chars: Vec<char> = lines[r].chars().collect();
        if chars.is_empty() {
            if r + 1 >= lines.len() {
                return (r, 0);
            }
            r += 1;
            c = 0;
            continue;
        }

        let mut i = c;
        if i < chars.len()
            && !chars[i].is_whitespace()
            && (i + 1 >= chars.len() || chars[i + 1].is_whitespace())
        {
            i += 1;
        }
        if i < chars.len() && chars[i].is_whitespace() {
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
        }

        if i < chars.len() {
            while i + 1 < chars.len() && !chars[i + 1].is_whitespace() {
                i += 1;
            }
            return (r, i);
        }

        if r + 1 >= lines.len() {
            return (r, chars.len().saturating_sub(1));
        }
        r += 1;
        c = 0;
    }
}

fn prev_word_position(lines: &[String], row: usize, col: usize) -> (usize, usize) {
    if lines.is_empty() {
        return (0, 0);
    }
    let mut r = row.min(lines.len().saturating_sub(1));
    let prev = prev_word_start(&lines[r], col);
    if prev < col || r == 0 {
        return (r, prev);
    }
    while r > 0 {
        r -= 1;
        if lines[r].chars().any(|ch| !ch.is_whitespace()) {
            let end = lines[r].chars().count().saturating_sub(1);
            return (r, prev_word_start(&lines[r], end));
        }
    }
    (0, 0)
}

fn insert_char_at(text: &mut String, offset: usize, c: char) {
    let byte_idx = char_to_byte_index(text, offset);
    text.insert(byte_idx, c);
}

fn delete_char_before(text: &mut String, offset: usize) -> bool {
    if offset == 0 {
        return false;
    }
    let start = char_to_byte_index(text, offset - 1);
    let end = char_to_byte_index(text, offset);
    text.replace_range(start..end, "");
    true
}

fn delete_char_at(text: &mut String, offset: usize) -> bool {
    if offset >= text.chars().count() {
        return false;
    }
    let start = char_to_byte_index(text, offset);
    let end = char_to_byte_index(text, offset + 1);
    text.replace_range(start..end, "");
    true
}

fn delete_char_range(text: &mut String, start_offset: usize, end_offset: usize) -> bool {
    if start_offset >= end_offset || start_offset >= text.chars().count() {
        return false;
    }
    let start = char_to_byte_index(text, start_offset);
    let end = char_to_byte_index(text, end_offset.min(text.chars().count()));
    text.replace_range(start..end, "");
    true
}

fn delete_to_line_end(text: &mut String, offset: usize) -> bool {
    if offset >= text.chars().count() {
        return false;
    }
    let start = char_to_byte_index(text, offset);
    text.replace_range(start..text.len(), "");
    true
}

fn char_to_byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len())
}

fn insert_char_into_multiline(text: &str, row: usize, col: usize, c: char) -> String {
    let mut lines = text_lines(text);
    ensure_line(&mut lines, row);
    let len = lines[row].chars().count();
    insert_char_at(&mut lines[row], col.min(len), c);
    lines.join("\n")
}

fn offset_to_line_col(text: &str, offset: usize) -> (usize, usize) {
    let lines = text_lines(text);
    let mut remaining = offset;
    for (line_index, line) in lines.iter().enumerate() {
        let line_len = line.chars().count();
        if remaining <= line_len {
            return (line_index, remaining);
        }
        remaining = remaining.saturating_sub(line_len + 1);
    }
    let last_index = lines.len().saturating_sub(1);
    (last_index, lines[last_index].chars().count())
}

fn line_col_to_offset(text: &str, target_line: usize, target_col: usize) -> usize {
    let lines = text_lines(text);
    let clamped_line = target_line.min(lines.len().saturating_sub(1));
    let mut offset = 0;
    for line in lines.iter().take(clamped_line) {
        offset += line.chars().count() + 1;
    }
    offset + target_col.min(lines[clamped_line].chars().count())
}

fn form_accepts_char(field: &FormField, c: char) -> bool {
    match field.schema_type {
        SchemaType::Integer => c.is_ascii_digit() || matches!(c, '+' | '-'),
        SchemaType::Number => c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E'),
        _ => true,
    }
}

fn insert_newline_into_multiline(text: &str, row: usize, col: usize) -> String {
    let mut lines = text_lines(text);
    ensure_line(&mut lines, row);
    let idx = char_to_byte_index(&lines[row], col.min(lines[row].chars().count()));
    let tail = lines[row].split_off(idx);
    lines.insert(row + 1, tail);
    lines.join("\n")
}

fn delete_char_from_multiline(text: &str, row: usize, col: usize) -> (String, bool, usize, usize) {
    let mut lines = text_lines(text);
    ensure_line(&mut lines, row);
    if col > 0 {
        let removed = delete_char_before(&mut lines[row], col);
        return (lines.join("\n"), removed, row, col.saturating_sub(1));
    }
    if row == 0 {
        return (lines.join("\n"), false, row, col);
    }
    let current = lines.remove(row);
    let new_col = lines[row - 1].chars().count();
    lines[row - 1].push_str(&current);
    (lines.join("\n"), true, row - 1, new_col)
}

fn delete_char_at_multiline(text: &str, row: usize, col: usize) -> (String, bool, usize, usize) {
    let mut lines = text_lines(text);
    ensure_line(&mut lines, row);
    let line_len = lines[row].chars().count();
    if col < line_len {
        let changed = delete_char_at(&mut lines[row], col);
        return (lines.join("\n"), changed, row, col);
    }
    if row + 1 >= lines.len() {
        return (
            lines.join("\n"),
            false,
            row,
            col.min(line_len.saturating_sub(1)),
        );
    }
    let next = lines.remove(row + 1);
    lines[row].push_str(&next);
    (
        lines.join("\n"),
        true,
        row,
        col.min(lines[row].chars().count().saturating_sub(1)),
    )
}

fn delete_to_line_end_multiline(text: &str, row: usize, col: usize) -> String {
    let mut lines = text_lines(text);
    ensure_line(&mut lines, row);
    let _ = delete_to_line_end(&mut lines[row], col);
    lines.join("\n")
}

fn delete_range_from_multiline(text: &str, row: usize, start_col: usize, end_col: usize) -> String {
    let mut lines = text_lines(text);
    ensure_line(&mut lines, row);
    let _ = delete_char_range(&mut lines[row], start_col, end_col);
    lines.join("\n")
}

fn insert_empty_line(text: &str, row: usize, above: bool) -> String {
    let mut lines = text_lines(text);
    let insert_at = if above { row } else { row.saturating_add(1) }.min(lines.len());
    lines.insert(insert_at, String::new());
    lines.join("\n")
}

fn indent_line_in_multiline(text: &str, row: usize) -> String {
    let mut lines = text_lines(text);
    ensure_line(&mut lines, row);
    lines[row].insert_str(0, "    ");
    lines.join("\n")
}

fn outdent_line_in_multiline(text: &str, row: usize) -> (String, usize) {
    let mut lines = text_lines(text);
    ensure_line(&mut lines, row);
    let removed = lines[row]
        .chars()
        .take(4)
        .take_while(|ch| *ch == ' ')
        .count();
    if removed > 0 {
        let byte_end = char_to_byte_index(&lines[row], removed);
        lines[row].replace_range(0..byte_end, "");
    }
    (lines.join("\n"), removed)
}

fn delete_line_from_multiline(text: &str, row: usize) -> (String, usize) {
    let mut lines = text_lines(text);
    if lines.is_empty() {
        return (String::new(), 0);
    }
    let target = row.min(lines.len().saturating_sub(1));
    lines.remove(target);
    if lines.is_empty() {
        lines.push(String::new());
    }
    let next_row = target.min(lines.len().saturating_sub(1));
    (lines.join("\n"), next_row)
}

fn ensure_line(lines: &mut Vec<String>, row: usize) {
    while lines.len() <= row {
        lines.push(String::new());
    }
}

fn normalize_selection(
    a: SelectionAnchor,
    b: SelectionAnchor,
) -> (SelectionAnchor, SelectionAnchor) {
    if (a.row, a.col) <= (b.row, b.col) {
        (a, b)
    } else {
        (b, a)
    }
}

fn collect_selection_text(
    lines: &[String],
    start: SelectionAnchor,
    end: SelectionAnchor,
) -> String {
    if start.row == end.row {
        return slice_chars(&lines[start.row], start.col, end.col.saturating_add(1));
    }
    let mut chunks = Vec::new();
    chunks.push(slice_chars(
        &lines[start.row],
        start.col,
        lines[start.row].chars().count(),
    ));
    for line in lines.iter().take(end.row).skip(start.row + 1) {
        chunks.push(line.clone());
    }
    chunks.push(slice_chars(&lines[end.row], 0, end.col.saturating_add(1)));
    chunks.join("\n")
}

fn slice_chars(text: &str, start: usize, end: usize) -> String {
    text.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use crate::domain::form::{
        FormField, FormFieldKind, SchemaType, build_form_fields_with, default_value_for_schema,
    };
    use crate::domain::validation::validate_document;

    use super::{AppMode, AppState, InputMode, PaneId, ScreenMode, is_schema_path_entry};

    #[test]
    fn keeps_invalid_form_value_as_error_on_commit() {
        let mut state = AppState::new();
        let row = state
            .form_fields
            .iter()
            .position(|field| field.key == "count")
            .unwrap();
        state.active_pane = PaneId::Form;
        state.input_mode = InputMode::Insert;
        state.pane_cursors.get_mut(&PaneId::Form).unwrap().row = row;
        state.form_fields[row].edit_buffer = "oops".to_owned();

        state.commit_active_editor().unwrap();

        assert_eq!(state.output_json["count"], 1);
        assert!(state.field_errors.contains_key("count"));
        assert!(
            state
                .logs
                .last()
                .unwrap()
                .contains("validation error on count")
        );
    }

    #[test]
    fn keeps_last_good_schema_when_schema_json_is_invalid() {
        let mut state = AppState::new();
        let previous_output = state.output_json.clone();
        let previous_schema = state.schema_json.clone();
        state.active_pane = PaneId::Schema;
        state.schema_text = "{".to_owned();

        state.commit_active_editor().unwrap();

        assert_eq!(state.output_json, previous_output);
        assert_eq!(state.schema_json, previous_schema);
        assert!(state.schema_error.is_some());
        assert!(
            state
                .schema_error
                .as_ref()
                .unwrap()
                .message
                .contains("invalid schema json")
        );
        assert!(state.logs.last().unwrap().contains("schema error"));
        assert_eq!(state.schema_error.as_ref().unwrap().line, Some(1));
    }

    #[test]
    fn keeps_pattern_mismatch_as_field_error_on_commit() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Schema;
        state.schema_text = r#"{
  "type": "object",
  "properties": {
    "code": {
      "type": "string",
      "pattern": "^[A-Z]{3}$",
      "default": "ABC"
    }
  }
}"#
        .to_owned();
        state.commit_active_editor().unwrap();

        let row = state
            .form_fields
            .iter()
            .position(|field| field.key == "code")
            .unwrap();
        state.active_pane = PaneId::Form;
        state.input_mode = InputMode::Insert;
        state.pane_cursors.get_mut(&PaneId::Form).unwrap().row = row;
        state.form_fields[row].edit_buffer = "abc".to_owned();

        state.commit_active_editor().unwrap();

        assert_eq!(state.output_json["code"], "ABC");
        assert!(state.field_errors.contains_key("code"));
    }

    #[test]
    fn wafer_mask_layout_name_commit_updates_output() {
        let schema_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/wafer-mask-layout.schema.json");
        let schema_text =
            fs::read_to_string(schema_path).expect("read schema/wafer-mask-layout.schema.json");
        let schema_json: serde_json::Value =
            serde_json::from_str(&schema_text).expect("parse wafer-mask-layout schema");
        let output_json =
            default_value_for_schema(&schema_json, &schema_json).expect("build defaults");

        let mut state = AppState::new();
        state.schema_text = schema_text;
        state.schema_json = schema_json;
        state.output_json = output_json;
        state.form_fields = build_form_fields_with(
            &state.schema_json,
            &state.schema_json,
            &state.output_json,
            Some(&state.one_of_choices),
        );
        state.validation =
            validate_document(&state.schema_json, &state.output_json).expect("validate defaults");
        state.active_pane = PaneId::Form;
        state.input_mode = InputMode::Insert;

        let row = state
            .form_fields
            .iter()
            .position(|field| field.key == "name")
            .expect("find name field");
        state.pane_cursors.get_mut(&PaneId::Form).unwrap().row = row;
        state.form_fields[row].edit_buffer = "mask-a".to_owned();

        state.commit_active_editor().unwrap();

        assert_eq!(state.output_json["name"], "mask-a");
        assert!(state.validation.is_valid, "{:?}", state.validation.errors);
        assert!(!state.field_errors.contains_key("name"));
    }

    #[test]
    fn keeps_last_good_schema_when_schema_itself_is_invalid() {
        let mut state = AppState::new();
        let previous_output = state.output_json.clone();
        let previous_schema = state.schema_json.clone();
        state.active_pane = PaneId::Schema;
        state.schema_text = r#"{"type":"object","properties":{"name":{"type":"nope"}}}"#.to_owned();

        state.commit_active_editor().unwrap();

        assert_eq!(state.output_json, previous_output);
        assert_eq!(state.schema_json, previous_schema);
        assert!(state.schema_error.is_some());
        assert!(state.logs.last().unwrap().contains("schema error"));
    }

    #[test]
    fn rejects_json_that_is_not_a_json_schema() {
        let mut state = AppState::new();
        let previous_output = state.output_json.clone();
        let previous_schema = state.schema_json.clone();
        state.active_pane = PaneId::Schema;
        state.schema_text = r#"["not", "a", "schema"]"#.to_owned();

        state.commit_active_editor().unwrap();

        assert_eq!(state.output_json, previous_output);
        assert_eq!(state.schema_json, previous_schema);
        assert!(state.schema_error.is_some());
        assert!(state.logs.last().unwrap().contains("schema error"));
    }

    #[test]
    fn clamps_cursor_to_form_pane_bounds() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Form;
        for _ in 0..100 {
            state.move_cursor_down();
            state.move_cursor_right();
        }
        let cursor = state.pane_cursors.get(&PaneId::Form).unwrap();
        assert!(cursor.row < state.form_fields.len());
        assert!(cursor.col < 64);
    }

    #[test]
    fn word_motion_crosses_lines() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Schema;
        state.schema_text = "abc\n  def".to_owned();
        state.move_cursor_top();
        state.move_cursor_word_forward();
        let cursor = state.pane_cursors.get(&PaneId::Schema).unwrap();
        assert_eq!((cursor.row, cursor.col), (1, 2));
        state.move_cursor_top();
        state.move_cursor_word_end();
        let cursor = state.pane_cursors.get(&PaneId::Schema).unwrap();
        assert_eq!((cursor.row, cursor.col), (0, 2));
        state.move_cursor_right();
        state.move_cursor_word_end();
        let cursor = state.pane_cursors.get(&PaneId::Schema).unwrap();
        assert_eq!((cursor.row, cursor.col), (1, 4));
        state.move_cursor_word_backward();
        let cursor = state.pane_cursors.get(&PaneId::Schema).unwrap();
        assert_eq!((cursor.row, cursor.col), (1, 2));
        state.move_cursor_word_backward();
        let cursor = state.pane_cursors.get(&PaneId::Schema).unwrap();
        assert_eq!((cursor.row, cursor.col), (0, 0));
    }

    #[test]
    fn insert_respects_cursor_position() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Filter;
        state.filter_text = ".foo".to_owned();
        state.pane_cursors.get_mut(&PaneId::Filter).unwrap().col = 1;
        state.enter_insert_mode(false);
        state.insert_char('x');
        assert_eq!(state.filter_text, ".xfoo");
    }

    #[test]
    fn append_mode_inserts_after_current_char() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Filter;
        state.filter_text = ".foo".to_owned();
        state.pane_cursors.get_mut(&PaneId::Filter).unwrap().col = 1;

        state.enter_insert_mode(true);
        state.insert_char('x');

        assert_eq!(state.filter_text, ".fxoo");
    }

    #[test]
    fn one_of_selector_wraps_and_writes_branch_defaults() {
        let mut state = AppState::new();
        state.schema_json = serde_json::json!({
            "type": "object",
            "properties": {
                "electrode": { "$ref": "#/$defs/electrode" }
            },
            "$defs": {
                "metal": {
                    "type": "object",
                    "properties": {
                        "kind": { "const": "metal" },
                        "thickness_mm": { "type": "number", "default": 1.0 }
                    }
                },
                "composite": {
                    "type": "object",
                    "properties": {
                        "kind": { "const": "composite" },
                        "layers": { "type": "integer", "default": 2 }
                    }
                },
                "electrode": {
                    "oneOf": [
                        { "$ref": "#/$defs/metal" },
                        { "$ref": "#/$defs/composite" }
                    ]
                }
            }
        });
        state.output_json =
            default_value_for_schema(&state.schema_json, &state.schema_json).unwrap();
        state.form_fields = build_form_fields_with(
            &state.schema_json,
            &state.schema_json,
            &state.output_json,
            Some(&state.one_of_choices),
        );
        state.active_pane = PaneId::Form;
        let row = state
            .form_fields
            .iter()
            .position(|f| matches!(f.kind, FormFieldKind::OneOfSelector { .. }))
            .unwrap();
        state.pane_cursors.get_mut(&PaneId::Form).unwrap().row = row;

        state.move_cursor_right();
        assert_eq!(state.output_json["electrode"]["kind"], "composite");
        assert_eq!(state.output_json["electrode"]["layers"], 2);

        state.move_cursor_right();
        assert_eq!(state.output_json["electrode"]["kind"], "metal");
    }

    #[test]
    fn form_enum_cycles_with_horizontal_motion() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Form;
        state.output_json = serde_json::json!({"status": "draft"});
        state.schema_json = serde_json::json!({
            "type": "object",
            "properties": {
                "status": { "type": "string", "enum": ["draft", "live", "archived"] }
            }
        });
        state.form_fields = vec![FormField {
            path: vec!["status".to_owned()],
            key: "status".to_owned(),
            label: "Status".to_owned(),
            description: None,
            schema_type: SchemaType::String,
            enum_options: Some(vec![
                "draft".to_owned(),
                "live".to_owned(),
                "archived".to_owned(),
            ]),
            multiline: false,
            required: true,
            edit_buffer: "draft".to_owned(),
            kind: FormFieldKind::Scalar,
        }];

        state.move_cursor_right();
        assert_eq!(state.form_fields[0].edit_buffer, "live");
        assert_eq!(state.output_json["status"], "live");
        state.move_cursor_right();
        assert_eq!(state.form_fields[0].edit_buffer, "archived");
        assert_eq!(state.output_json["status"], "archived");
        state.move_cursor_left();
        assert_eq!(state.form_fields[0].edit_buffer, "live");
        assert_eq!(state.output_json["status"], "live");
    }

    #[test]
    fn insert_mode_h_and_l_cycle_form_select_fields() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Form;
        state.input_mode = InputMode::Insert;
        state.output_json = serde_json::json!({"status": "draft"});
        state.schema_json = serde_json::json!({
            "type": "object",
            "properties": {
                "status": { "type": "string", "enum": ["draft", "live", "archived"] }
            }
        });
        state.form_fields = vec![FormField {
            path: vec!["status".to_owned()],
            key: "status".to_owned(),
            label: "Status".to_owned(),
            description: None,
            schema_type: SchemaType::String,
            enum_options: Some(vec![
                "draft".to_owned(),
                "live".to_owned(),
                "archived".to_owned(),
            ]),
            multiline: false,
            required: true,
            edit_buffer: "draft".to_owned(),
            kind: FormFieldKind::Scalar,
        }];

        state.insert_char_form('l');
        assert_eq!(state.form_fields[0].edit_buffer, "live");
        state.insert_char_form('h');
        assert_eq!(state.form_fields[0].edit_buffer, "draft");
    }

    #[test]
    fn insert_newline_works_for_multiline_form_fields() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Form;
        state.input_mode = InputMode::Insert;
        state.form_fields = vec![FormField {
            path: vec!["note".to_owned()],
            key: "note".to_owned(),
            label: "Note".to_owned(),
            description: None,
            schema_type: SchemaType::String,
            enum_options: None,
            multiline: true,
            required: false,
            edit_buffer: "hello".to_owned(),
            kind: FormFieldKind::Scalar,
        }];
        state.pane_cursors.get_mut(&PaneId::Form).unwrap().col = 5;

        state.insert_newline();

        assert_eq!(state.form_fields[0].edit_buffer, "hello\n");
    }

    #[test]
    fn textarea_arrow_keys_move_within_multiline_field() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Form;
        state.input_mode = InputMode::Insert;
        state.form_fields = vec![FormField {
            path: vec!["note".to_owned()],
            key: "note".to_owned(),
            label: "Note".to_owned(),
            description: None,
            schema_type: SchemaType::String,
            enum_options: None,
            multiline: true,
            required: false,
            edit_buffer: "abc\nde".to_owned(),
            kind: FormFieldKind::Scalar,
        }];
        state.pane_cursors.get_mut(&PaneId::Form).unwrap().row = 0;
        state.pane_cursors.get_mut(&PaneId::Form).unwrap().col = 1;

        state.move_cursor_down();
        assert_eq!(state.pane_cursors.get(&PaneId::Form).unwrap().row, 0);
        assert_eq!(state.form_textarea_cursor(0), (1, 1));

        state.move_cursor_up();
        assert_eq!(state.form_textarea_cursor(0), (0, 1));
    }

    #[test]
    fn integer_form_fields_reject_non_numeric_characters() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Form;
        state.input_mode = InputMode::Insert;
        state.form_fields = vec![FormField {
            path: vec!["count".to_owned()],
            key: "count".to_owned(),
            label: "Count".to_owned(),
            description: None,
            schema_type: SchemaType::Integer,
            enum_options: None,
            multiline: false,
            required: false,
            edit_buffer: String::new(),
            kind: FormFieldKind::Scalar,
        }];

        state.insert_char('1');
        state.insert_char('a');

        assert_eq!(state.form_fields[0].edit_buffer, "1");
    }

    #[test]
    fn number_form_fields_accept_web_number_characters() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Form;
        state.input_mode = InputMode::Insert;
        state.form_fields = vec![FormField {
            path: vec!["ratio".to_owned()],
            key: "ratio".to_owned(),
            label: "Ratio".to_owned(),
            description: None,
            schema_type: SchemaType::Number,
            enum_options: None,
            multiline: false,
            required: false,
            edit_buffer: String::new(),
            kind: FormFieldKind::Scalar,
        }];

        for c in ['+', '-', '.', '1', 'e', 'E'] {
            state.insert_char(c);
        }

        assert_eq!(state.form_fields[0].edit_buffer, "+-.1eE");
    }

    #[test]
    fn standard_mode_hides_schema_pane_from_visible_panes() {
        let state = AppState::new_with_mode(AppMode::Standard);
        assert!(!state.is_pane_visible(PaneId::Schema));
        assert_eq!(state.visible_panes(), &PaneId::STANDARD);
    }

    #[test]
    fn standard_mode_keeps_schema_path_read_only() {
        let mut state = AppState::new_with_mode(AppMode::Standard);
        state.active_pane = PaneId::SchemaPath;

        state.enter_insert_mode(false);

        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn add_array_item_appends_new_form_entry() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Form;
        state.schema_json = serde_json::json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "title": "Tags",
                    "minItems": 2,
                    "items": {
                        "type": "string",
                        "default": "tag"
                    }
                }
            }
        });
        state.output_json = serde_json::json!({
            "tags": ["alpha", "beta"]
        });
        state.form_fields =
            build_form_fields_with(
                &state.schema_json,
                &state.schema_json,
                &state.output_json,
                Some(&state.one_of_choices),
            );
        state.pane_cursors.get_mut(&PaneId::Form).unwrap().row = 1;

        state.add_array_item_at_cursor().unwrap();

        assert_eq!(
            state.output_json["tags"],
            serde_json::json!(["alpha", "beta", "tag"])
        );
        assert!(
            state
                .form_fields
                .iter()
                .any(|field| field.path == vec!["tags".to_owned(), "2".to_owned()])
        );
        assert_eq!(state.cursor_row(PaneId::Form), 2);
    }

    #[test]
    fn add_array_item_ignores_non_array_fields() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Form;
        state.schema_json = serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "default": "demo" }
            }
        });
        state.output_json = serde_json::json!({
            "name": "demo"
        });
        state.form_fields =
            build_form_fields_with(
                &state.schema_json,
                &state.schema_json,
                &state.output_json,
                Some(&state.one_of_choices),
            );

        state.add_array_item_at_cursor().unwrap();

        assert_eq!(state.output_json["name"], "demo");
        assert_eq!(state.form_fields.len(), 1);
    }

    #[test]
    fn add_array_item_works_from_empty_array_placeholder() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Form;
        state.schema_json = serde_json::json!({
            "type": "object",
            "properties": {
                "alignment_holes": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "default": "hole" }
                        }
                    }
                }
            }
        });
        state.output_json = default_value_for_schema(&state.schema_json, &state.schema_json)
            .expect("build empty-array defaults");
        state.form_fields = build_form_fields_with(
            &state.schema_json,
            &state.schema_json,
            &state.output_json,
            Some(&state.one_of_choices),
        );
        let row = state
            .form_fields
            .iter()
            .position(|field| matches!(field.kind, FormFieldKind::ArrayPlaceholder))
            .expect("find array placeholder");
        state.pane_cursors.get_mut(&PaneId::Form).unwrap().row = row;

        state.add_array_item_at_cursor().unwrap();

        assert_eq!(
            state.output_json["alignment_holes"],
            serde_json::json!([{ "name": "hole" }])
        );
        assert!(state.form_fields.iter().any(|field| {
            field.path
                == vec![
                    "alignment_holes".to_owned(),
                    "0".to_owned(),
                    "name".to_owned()
                ]
        }));
    }

    #[test]
    fn remove_array_item_drops_current_array_entry() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Form;
        state.schema_json = serde_json::json!({
            "type": "object",
            "properties": {
                "members": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "default": "Alice" },
                            "active": { "type": "boolean", "default": true }
                        }
                    }
                }
            }
        });
        state.output_json = serde_json::json!({
            "members": [
                { "name": "Alice", "active": true },
                { "name": "Bob", "active": false }
            ]
        });
        state.form_fields =
            build_form_fields_with(
                &state.schema_json,
                &state.schema_json,
                &state.output_json,
                Some(&state.one_of_choices),
            );
        let row = state
            .form_fields
            .iter()
            .position(|field| {
                field.path == vec!["members".to_owned(), "1".to_owned(), "name".to_owned()]
            })
            .unwrap();
        state.pane_cursors.get_mut(&PaneId::Form).unwrap().row = row;

        state.remove_array_item_at_cursor().unwrap();

        assert_eq!(
            state.output_json["members"],
            serde_json::json!([{ "name": "Alice", "active": true }])
        );
        assert!(state.form_fields.iter().all(|field| {
            field
                .path
                .get(1)
                .map(|segment| segment != "1")
                .unwrap_or(true)
        }));
    }

    #[test]
    fn remove_array_item_respects_min_items() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Form;
        state.schema_json = serde_json::json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "minItems": 1,
                    "items": { "type": "string", "default": "tag" }
                }
            }
        });
        state.output_json = serde_json::json!({
            "tags": ["only"]
        });
        state.form_fields =
            build_form_fields_with(
                &state.schema_json,
                &state.schema_json,
                &state.output_json,
                Some(&state.one_of_choices),
            );

        let result = state.remove_array_item_at_cursor();

        assert!(result.is_err());
        assert_eq!(state.output_json["tags"], serde_json::json!(["only"]));
    }

    #[test]
    fn add_array_item_respects_max_items() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Form;
        state.schema_json = serde_json::json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "maxItems": 2,
                    "items": { "type": "string", "default": "tag" }
                }
            }
        });
        state.output_json = serde_json::json!({
            "tags": ["one", "two"]
        });
        state.form_fields =
            build_form_fields_with(
                &state.schema_json,
                &state.schema_json,
                &state.output_json,
                Some(&state.one_of_choices),
            );

        let result = state.add_array_item_at_cursor();

        assert!(result.is_err());
        assert_eq!(state.output_json["tags"], serde_json::json!(["one", "two"]));
    }

    #[test]
    fn reset_form_restores_schema_defaults() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Form;
        state.schema_json = serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "default": "example" },
                "count": { "type": "integer", "default": 1 }
            }
        });
        state.output_json = serde_json::json!({
            "name": "edited",
            "count": 9
        });
        state.form_fields =
            build_form_fields_with(
                &state.schema_json,
                &state.schema_json,
                &state.output_json,
                Some(&state.one_of_choices),
            );
        state
            .field_errors
            .insert("name".to_owned(), "bad".to_owned());

        state.reset_form_to_defaults().unwrap();

        assert_eq!(
            state.output_json,
            serde_json::json!({
                "name": "example",
                "count": 1
            })
        );
        assert!(state.field_errors.is_empty());
    }

    #[test]
    fn editor_mode_allows_schema_path_editing() {
        let mut state = AppState::new_with_mode(AppMode::Editor);
        state.active_pane = PaneId::SchemaPath;

        state.enter_insert_mode(false);

        assert_eq!(state.input_mode, InputMode::Insert);
    }

    #[test]
    fn undo_redo_is_kept_per_pane() {
        let mut state = AppState::new();

        state.active_pane = PaneId::Filter;
        state.filter_text = ".foo".to_owned();
        state.pane_cursors.get_mut(&PaneId::Filter).unwrap().col = 4;
        state.insert_char('x');
        assert_eq!(state.filter_text, ".foox");

        state.active_pane = PaneId::OutputPath;
        state.pane_cursors.get_mut(&PaneId::OutputPath).unwrap().col = 0;
        state.insert_char('a');
        assert_eq!(state.schema_path.output_path, "a./output.json");

        state.active_pane = PaneId::Filter;
        state.undo();
        assert_eq!(state.filter_text, ".foo");

        state.active_pane = PaneId::OutputPath;
        state.undo();
        assert_eq!(state.schema_path.output_path, "./output.json");

        state.redo();
        assert_eq!(state.schema_path.output_path, "a./output.json");
    }

    #[test]
    fn delete_commands_work_on_filter_text() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Filter;
        state.filter_text = ".alpha".to_owned();
        state.pane_cursors.get_mut(&PaneId::Filter).unwrap().col = 1;

        state.delete_char();
        assert_eq!(state.filter_text, ".lpha");

        state.delete_to_line_end();
        assert_eq!(state.filter_text, ".");
    }

    #[test]
    fn form_cursor_does_not_move_past_edit_buffer_end() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Form;
        state.pane_cursors.get_mut(&PaneId::Form).unwrap().row = 0;

        for _ in 0..100 {
            state.move_cursor_right();
        }

        let cursor = state.pane_cursors.get(&PaneId::Form).unwrap();
        let max = state.form_fields[0]
            .edit_buffer
            .chars()
            .count()
            .saturating_sub(1);
        assert_eq!(cursor.col, max);
    }

    #[test]
    fn open_line_below_and_above_work_for_filter() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Filter;
        state.filter_text = "one\ntwo".to_owned();
        state.pane_cursors.get_mut(&PaneId::Filter).unwrap().row = 0;

        state.open_line_below();
        assert_eq!(state.filter_text, "one\n\ntwo");
        assert_eq!(state.pane_cursors.get(&PaneId::Filter).unwrap().row, 1);
        assert_eq!(state.input_mode, InputMode::Insert);

        state.exit_mode();
        state.open_line_above();
        assert_eq!(state.filter_text, "one\n\n\ntwo");
        assert_eq!(state.pane_cursors.get(&PaneId::Filter).unwrap().row, 1);
    }

    #[test]
    fn delete_line_removes_current_line() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Filter;
        state.filter_text = "one\ntwo\nthree".to_owned();
        state.pane_cursors.get_mut(&PaneId::Filter).unwrap().row = 1;

        state.delete_line();

        assert_eq!(state.filter_text, "one\nthree");
        assert_eq!(state.pane_cursors.get(&PaneId::Filter).unwrap().row, 1);
        assert_eq!(state.pane_cursors.get(&PaneId::Filter).unwrap().col, 0);
    }

    #[test]
    fn offers_bundled_schema_candidates() {
        let mut state = AppState::new();
        state.schema_path.schema_source = "sample/".to_owned();
        let candidates = state.schema_candidates();
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.contains("sample/basic"))
        );
    }

    #[test]
    fn defaults_to_local_schema_file() {
        let state = AppState::new();
        assert_eq!(state.schema_path.schema_source, "./schema/basic.json");
    }

    #[test]
    fn offers_local_schema_directory_candidates() {
        let mut state = AppState::new();
        state.schema_path.schema_source = "schema/".to_owned();
        let candidates = state.schema_candidates();
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.contains("schema/basic.json"))
        );
    }

    #[test]
    fn tab_completion_expands_schema_prefix() {
        let mut state = AppState::new();
        state.schema_path.schema_source = "sample/b".to_owned();

        state.complete_schema_path();

        assert!(state.schema_path.schema_source.starts_with("sample/b"));
        assert!(state.schema_path.schema_source.len() >= "sample/b".len());
    }

    #[test]
    fn tab_completion_cycles_through_schema_candidates() {
        let mut state = AppState::new();
        state.schema_path.schema_source = "sample/".to_owned();

        state.complete_schema_path();
        let first = state.schema_path.schema_source.clone();
        state.complete_schema_path();
        let second = state.schema_path.schema_source.clone();
        state.complete_schema_path_prev();
        let previous = state.schema_path.schema_source.clone();

        assert_ne!(first, second);
        assert_eq!(previous, first);
    }

    #[test]
    fn schema_path_candidates_only_include_json_files_from_directory() {
        assert!(is_schema_path_entry("schema.json"));
        assert!(is_schema_path_entry("nested/"));
        assert!(!is_schema_path_entry("notes.txt"));
        assert!(!is_schema_path_entry("Cargo.toml"));
    }

    #[test]
    fn saves_output_json_to_output_path() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("saved.json");
        let mut state = AppState::new();
        state.schema_path.output_path = path.to_string_lossy().into_owned();

        state.save_output().unwrap();

        let written = fs::read_to_string(path).unwrap();
        assert!(written.contains("\"name\""));
        assert!(written.contains('\n'));
    }

    #[test]
    fn save_output_requests_confirmation_when_target_exists() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("saved.json");
        fs::write(&path, "{\"old\":true}").unwrap();
        let mut state = AppState::new();
        state.schema_path.output_path = path.to_string_lossy().into_owned();

        state.save_output().unwrap();

        assert_eq!(state.screen_mode, ScreenMode::ConfirmOverwrite);
        assert_eq!(
            state.overwrite_path.as_deref(),
            Some(state.schema_path.output_path.as_str())
        );
    }

    #[test]
    fn confirm_overwrite_writes_existing_target() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("saved.json");
        fs::write(&path, "{\"old\":true}").unwrap();
        let mut state = AppState::new();
        state.schema_path.output_path = path.to_string_lossy().into_owned();

        state.save_output().unwrap();
        state.confirm_overwrite().unwrap();

        let written = fs::read_to_string(path).unwrap();
        assert!(written.contains("\"name\""));
        assert_eq!(state.screen_mode, ScreenMode::Edit);
        assert!(state.overwrite_path.is_none());
    }

    #[test]
    fn schema_indent_and_outdent_use_four_spaces() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Schema;
        state.schema_text = "{\n\"name\": true\n}".to_owned();
        state.pane_cursors.get_mut(&PaneId::Schema).unwrap().row = 1;
        state.pane_cursors.get_mut(&PaneId::Schema).unwrap().col = 0;

        state.indent_schema_line();
        assert!(state.schema_text.contains("\n    \"name\": true\n"));
        assert_eq!(state.pane_cursors.get(&PaneId::Schema).unwrap().col, 4);

        state.outdent_schema_line();
        assert!(state.schema_text.contains("\n\"name\": true\n"));
        assert_eq!(state.pane_cursors.get(&PaneId::Schema).unwrap().col, 0);
    }

    #[test]
    fn delete_operator_supports_word_and_line_motions() {
        let mut state = AppState::new();
        state.active_pane = PaneId::Filter;
        state.filter_text = "alpha beta gamma".to_owned();
        state.pane_cursors.get_mut(&PaneId::Filter).unwrap().col = 0;

        state.delete_word_forward();
        assert_eq!(state.filter_text, "beta gamma");

        state.pane_cursors.get_mut(&PaneId::Filter).unwrap().col = 4;
        state.delete_to_line_start();
        assert_eq!(state.filter_text, " gamma");

        state.delete_to_line_end();
        assert_eq!(state.filter_text, "");
    }
}
