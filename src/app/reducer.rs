use anyhow::Result;

use crate::app::actions::Action;
use crate::app::state::{AppState, PaneId};

pub fn reduce(state: &mut AppState, action: Action) -> Result<bool> {
    if state.screen_mode == crate::app::state::ScreenMode::Help {
        match action {
            Action::Quit | Action::ToggleHelp | Action::ExitMode => {
                state.screen_mode = crate::app::state::ScreenMode::Edit;
                return Ok(false);
            }
            _ => {
                state.screen_mode = crate::app::state::ScreenMode::Edit;
                return Ok(false);
            }
        }
    }

    if state.screen_mode == crate::app::state::ScreenMode::ConfirmOverwrite {
        match action {
            Action::ConfirmOverwrite => {
                state.confirm_overwrite()?;
                return Ok(false);
            }
            Action::CancelOverwrite | Action::ExitMode | Action::Quit => {
                state.cancel_overwrite();
                return Ok(false);
            }
            _ => return Ok(false),
        }
    }

    match action {
        Action::Quit => return Ok(true),
        Action::ToggleHelp => {
            state.screen_mode = crate::app::state::ScreenMode::Help;
        }
        Action::FocusPane(pane) => {
            state.focus_pane_at(pane);
        }
        Action::FocusNextPane => state.focus_next_pane(),
        Action::FocusPrevPane => state.focus_prev_pane(),
        Action::FocusNextFormField => state.focus_next_form_field(),
        Action::FocusPrevFormField => state.focus_prev_form_field(),
        Action::MoveUp => state.move_cursor_up(),
        Action::MoveDown => state.move_cursor_down(),
        Action::MoveLeft => state.move_cursor_left(),
        Action::MoveRight => state.move_cursor_right(),
        Action::MoveLineStart => state.move_cursor_line_start(),
        Action::MoveLineEnd => state.move_cursor_line_end(),
        Action::MoveWordForward => state.move_cursor_word_forward(),
        Action::MoveWordEnd => state.move_cursor_word_end(),
        Action::MoveWordBackward => state.move_cursor_word_backward(),
        Action::MoveTop => state.move_cursor_top(),
        Action::MoveBottom => state.move_cursor_bottom(),
        Action::AwaitSecondG => {
            state.pending_g = true;
            return Ok(false);
        }
        Action::AwaitSecondD => {
            state.pending_d = true;
            return Ok(false);
        }
        Action::AwaitSecondZ => {
            state.pending_z = true;
            return Ok(false);
        }
        Action::EnterInsertBefore => {
            if let Some(button) = state.focused_form_button() {
                if let Err(err) = state.activate_form_button(button.array_path, button.kind) {
                    state.log_error(format!("array action error: {err}"));
                }
            } else {
                state.enter_insert_mode(false);
            }
        }
        Action::EnterInsertAfter => state.enter_insert_mode(true),
        Action::OpenLineBelow => state.open_line_below(),
        Action::OpenLineAbove => state.open_line_above(),
        Action::EnterVisual => state.enter_visual_mode(),
        Action::ExitMode => state.exit_mode(),
        Action::YankSelection => {
            state.yank_selection()?;
            state.exit_mode();
        }
        Action::Backspace => state.backspace(),
        Action::DeleteChar => state.delete_char(),
        Action::DeleteLine => state.delete_line(),
        Action::DeleteToLineStart => state.delete_to_line_start(),
        Action::DeleteWordForward => state.delete_word_forward(),
        Action::DeleteToLineEnd => state.delete_to_line_end(),
        Action::AddArrayItem => {
            if let Err(err) = state.add_array_item_at_cursor() {
                state.log_error(format!("array add error: {err}"));
            }
        }
        Action::RemoveArrayItem => {
            if let Err(err) = state.remove_array_item_at_cursor() {
                state.log_error(format!("array remove error: {err}"));
            }
        }
        Action::ResetForm => {
            if let Err(err) = state.reset_form_to_defaults() {
                state.log_error(format!("form reset error: {err}"));
            }
        }
        Action::TogglePaneCollapse => state.toggle_active_pane_collapse(),
        Action::CollapsePane => state.collapse_active_pane(),
        Action::ExpandPane => state.expand_active_pane(),
        Action::Undo => state.undo(),
        Action::Redo => state.redo(),
        Action::SaveOutput => state.save_output()?,
        Action::ConfirmOverwrite => state.confirm_overwrite()?,
        Action::CancelOverwrite => state.cancel_overwrite(),
        Action::InsertChar(c) => state.insert_char(c),
        Action::InsertNewline => {
            if state.form_enter_commits() {
                if let Err(err) = state.commit_active_editor() {
                    state.log_error(format!("commit error: {err}"));
                }
                state.exit_mode();
            } else {
                state.insert_newline();
            }
        }
        Action::IndentSchemaLine => state.indent_schema_line(),
        Action::OutdentSchemaLine => state.outdent_schema_line(),
        Action::CompleteSchemaPath => state.complete_schema_path(),
        Action::CompleteSchemaPathPrev => state.complete_schema_path_prev(),
        Action::CommitActiveEditor => {
            if let Err(err) = state.commit_active_editor() {
                state.log_error(format!("commit error: {err}"));
            }
            state.exit_mode();
        }
        Action::Noop => {}
    }

    state.pending_g = false;
    state.pending_d = false;
    state.pending_z = false;
    Ok(false)
}

#[allow(dead_code)]
fn _pane_label(pane: PaneId) -> &'static str {
    pane.base_title()
}
