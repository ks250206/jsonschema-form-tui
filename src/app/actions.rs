use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::state::{AppMode, InputMode, PaneId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    ToggleHelp,
    FocusPane(PaneId),
    FocusNextPane,
    FocusPrevPane,
    FocusNextFormField,
    FocusPrevFormField,
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    MoveLineStart,
    MoveLineEnd,
    MoveWordForward,
    MoveWordEnd,
    MoveWordBackward,
    MoveTop,
    MoveBottom,
    AwaitSecondG,
    AwaitSecondD,
    AwaitSecondZ,
    EnterInsertBefore,
    EnterInsertAfter,
    OpenLineBelow,
    OpenLineAbove,
    EnterVisual,
    ExitMode,
    YankSelection,
    Backspace,
    DeleteChar,
    DeleteLine,
    DeleteToLineStart,
    DeleteWordForward,
    DeleteToLineEnd,
    AddArrayItem,
    RemoveArrayItem,
    ResetForm,
    TogglePaneCollapse,
    ToggleMainFullwidth,
    CollapsePane,
    ExpandPane,
    Undo,
    Redo,
    InsertChar(char),
    InsertNewline,
    IndentSchemaLine,
    OutdentSchemaLine,
    CompleteSchemaPath,
    CompleteSchemaPathPrev,
    SaveOutput,
    ConfirmOverwrite,
    CancelOverwrite,
    CommitActiveEditor,
    Noop,
}

impl Action {
    pub fn from_key(
        app_mode: AppMode,
        mode: InputMode,
        pending_g: bool,
        pending_d: bool,
        pending_z: bool,
        active_pane: PaneId,
        key: KeyEvent,
    ) -> Self {
        match mode {
            InputMode::Normal => match key.code {
                KeyCode::Char('q') => Self::Quit,
                KeyCode::Char('?') => Self::ToggleHelp,
                KeyCode::Esc => Self::ExitMode,
                KeyCode::Tab => Self::FocusNextPane,
                KeyCode::BackTab => Self::FocusPrevPane,
                KeyCode::Left => Self::MoveLeft,
                KeyCode::Down => Self::MoveDown,
                KeyCode::Up => Self::MoveUp,
                KeyCode::Right => Self::MoveRight,
                KeyCode::Char('h') => Self::MoveLeft,
                KeyCode::Char('j') => Self::MoveDown,
                KeyCode::Char('k') => Self::MoveUp,
                KeyCode::Char('l') => Self::MoveRight,
                KeyCode::Char('0') if pending_d => Self::DeleteToLineStart,
                KeyCode::Char('0') => Self::MoveLineStart,
                KeyCode::Char('$') if pending_d => Self::DeleteToLineEnd,
                KeyCode::Char('$') => Self::MoveLineEnd,
                KeyCode::Char('w') | KeyCode::Char('W') if pending_d => Self::DeleteWordForward,
                KeyCode::Char('e') | KeyCode::Char('E') => Self::MoveWordEnd,
                KeyCode::Char('b') | KeyCode::Char('B') => Self::MoveWordBackward,
                KeyCode::Char('d') if pending_d => Self::DeleteLine,
                KeyCode::Char('d') => Self::AwaitSecondD,
                KeyCode::Char('D') => Self::DeleteToLineEnd,
                KeyCode::Char('+') if active_pane == PaneId::Form => Self::AddArrayItem,
                KeyCode::Char('-') if active_pane == PaneId::Form => Self::RemoveArrayItem,
                KeyCode::Char('R') if active_pane == PaneId::Form => Self::ResetForm,
                KeyCode::Char('g') if pending_g => Self::MoveTop,
                KeyCode::Char('g') => Self::AwaitSecondG,
                KeyCode::Char('a') if pending_z => Self::TogglePaneCollapse,
                KeyCode::Char('w') | KeyCode::Char('W') if pending_z => Self::ToggleMainFullwidth,
                KeyCode::Char('c') if pending_z => Self::CollapsePane,
                KeyCode::Char('o') if pending_z => Self::ExpandPane,
                KeyCode::Char('z') => Self::AwaitSecondZ,
                KeyCode::Char('w') | KeyCode::Char('W') => Self::MoveWordForward,
                KeyCode::Char('a') => Self::EnterInsertAfter,
                KeyCode::Char('G') => Self::MoveBottom,
                KeyCode::Char('i') => Self::EnterInsertBefore,
                KeyCode::Enter if active_pane == PaneId::Form => Self::EnterInsertBefore,
                KeyCode::Char('o') => Self::OpenLineBelow,
                KeyCode::Char('O') => Self::OpenLineAbove,
                KeyCode::Char('v') => Self::EnterVisual,
                KeyCode::Char('u') => Self::Undo,
                KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => Self::Redo,
                KeyCode::Char('r') => Self::SaveOutput,
                KeyCode::Char('1') => Self::FocusPane(PaneId::SchemaPath),
                KeyCode::Char('2') if app_mode == AppMode::Editor => {
                    Self::FocusPane(PaneId::Schema)
                }
                KeyCode::Char('2') => Self::FocusPane(PaneId::Form),
                KeyCode::Char('3') if app_mode == AppMode::Editor => Self::FocusPane(PaneId::Form),
                KeyCode::Char('3') => Self::FocusPane(PaneId::OutputPath),
                KeyCode::Char('4') if app_mode == AppMode::Editor => {
                    Self::FocusPane(PaneId::OutputPath)
                }
                KeyCode::Char('4') => Self::FocusPane(PaneId::Filter),
                KeyCode::Char('5') if app_mode == AppMode::Editor => {
                    Self::FocusPane(PaneId::Filter)
                }
                KeyCode::Char('5') => Self::FocusPane(PaneId::Output),
                KeyCode::Char('6') if app_mode == AppMode::Editor => {
                    Self::FocusPane(PaneId::Output)
                }
                KeyCode::Char('6') => Self::FocusPane(PaneId::Log),
                KeyCode::Char('7') => Self::FocusPane(PaneId::Log),
                _ => Self::Noop,
            },
            InputMode::Insert => match key.code {
                KeyCode::Esc => Self::CommitActiveEditor,
                KeyCode::Enter if active_pane == PaneId::SchemaPath => Self::CommitActiveEditor,
                KeyCode::Enter if active_pane == PaneId::OutputPath => Self::CommitActiveEditor,
                KeyCode::Tab if active_pane == PaneId::Schema => Self::IndentSchemaLine,
                KeyCode::BackTab if active_pane == PaneId::Schema => Self::OutdentSchemaLine,
                KeyCode::Tab if active_pane == PaneId::SchemaPath => Self::CompleteSchemaPath,
                KeyCode::BackTab if active_pane == PaneId::SchemaPath => {
                    Self::CompleteSchemaPathPrev
                }
                KeyCode::Tab if active_pane == PaneId::Form => Self::FocusNextFormField,
                KeyCode::BackTab if active_pane == PaneId::Form => Self::FocusPrevFormField,
                KeyCode::Left => Self::MoveLeft,
                KeyCode::Down => Self::MoveDown,
                KeyCode::Up => Self::MoveUp,
                KeyCode::Right => Self::MoveRight,
                KeyCode::Home => Self::MoveLineStart,
                KeyCode::End => Self::MoveLineEnd,
                KeyCode::Enter => Self::InsertNewline,
                KeyCode::Backspace => Self::Backspace,
                KeyCode::Delete => Self::DeleteChar,
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => Self::Undo,
                KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => Self::Redo,
                KeyCode::Char('[') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Self::CommitActiveEditor
                }
                KeyCode::Char(c) => Self::InsertChar(c),
                _ => Self::Noop,
            },
            InputMode::Visual => match key.code {
                KeyCode::Esc | KeyCode::Char('v') => Self::ExitMode,
                KeyCode::Char('?') => Self::ToggleHelp,
                KeyCode::Left => Self::MoveLeft,
                KeyCode::Down => Self::MoveDown,
                KeyCode::Up => Self::MoveUp,
                KeyCode::Right => Self::MoveRight,
                KeyCode::Char('h') => Self::MoveLeft,
                KeyCode::Char('j') => Self::MoveDown,
                KeyCode::Char('k') => Self::MoveUp,
                KeyCode::Char('l') => Self::MoveRight,
                KeyCode::Char('0') => Self::MoveLineStart,
                KeyCode::Char('$') => Self::MoveLineEnd,
                KeyCode::Char('w') | KeyCode::Char('W') => Self::MoveWordForward,
                KeyCode::Char('e') | KeyCode::Char('E') => Self::MoveWordEnd,
                KeyCode::Char('b') | KeyCode::Char('B') => Self::MoveWordBackward,
                KeyCode::Char('g') if pending_g => Self::MoveTop,
                KeyCode::Char('g') => Self::AwaitSecondG,
                KeyCode::Char('G') => Self::MoveBottom,
                KeyCode::Char('y') => Self::YankSelection,
                _ => Self::Noop,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::Action;
    use crate::app::state::{InputMode, PaneId};

    #[test]
    fn insert_mode_arrow_keys_move_cursor() {
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Insert,
                false,
                false,
                false,
                PaneId::Filter,
                KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)
            ),
            Action::MoveLeft
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Insert,
                false,
                false,
                false,
                PaneId::Filter,
                KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)
            ),
            Action::MoveRight
        );
    }

    #[test]
    fn normal_mode_maps_delete_undo_and_redo() {
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Normal,
                false,
                false,
                false,
                PaneId::Filter,
                KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
            ),
            Action::FocusNextPane
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Normal,
                false,
                false,
                false,
                PaneId::Form,
                KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
            ),
            Action::FocusNextPane
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Insert,
                false,
                false,
                false,
                PaneId::Form,
                KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
            ),
            Action::FocusNextFormField
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Normal,
                false,
                true,
                false,
                PaneId::Filter,
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE)
            ),
            Action::DeleteWordForward
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Normal,
                false,
                true,
                false,
                PaneId::Filter,
                KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE)
            ),
            Action::DeleteToLineStart
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Normal,
                false,
                true,
                false,
                PaneId::Filter,
                KeyEvent::new(KeyCode::Char('$'), KeyModifiers::SHIFT)
            ),
            Action::DeleteToLineEnd
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Normal,
                false,
                false,
                false,
                PaneId::Filter,
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE)
            ),
            Action::AwaitSecondD
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Normal,
                false,
                true,
                false,
                PaneId::Filter,
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE)
            ),
            Action::DeleteLine
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Normal,
                false,
                false,
                false,
                PaneId::Filter,
                KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT)
            ),
            Action::DeleteToLineEnd
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Normal,
                false,
                false,
                false,
                PaneId::Filter,
                KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE)
            ),
            Action::Undo
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Normal,
                false,
                false,
                false,
                PaneId::Filter,
                KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)
            ),
            Action::EnterInsertBefore
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Normal,
                false,
                false,
                false,
                PaneId::Filter,
                KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)
            ),
            Action::EnterInsertAfter
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Normal,
                false,
                false,
                false,
                PaneId::Form,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
            ),
            Action::EnterInsertBefore
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Normal,
                false,
                false,
                false,
                PaneId::Filter,
                KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE)
            ),
            Action::OpenLineBelow
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Normal,
                false,
                false,
                false,
                PaneId::Filter,
                KeyEvent::new(KeyCode::Char('O'), KeyModifiers::SHIFT)
            ),
            Action::OpenLineAbove
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Normal,
                false,
                false,
                false,
                PaneId::Filter,
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)
            ),
            Action::SaveOutput
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Normal,
                false,
                false,
                false,
                PaneId::Filter,
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)
            ),
            Action::Redo
        );
    }

    #[test]
    fn schema_path_supports_tab_completion_in_insert_mode() {
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Insert,
                false,
                false,
                false,
                PaneId::Schema,
                KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
            ),
            Action::IndentSchemaLine
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Insert,
                false,
                false,
                false,
                PaneId::Schema,
                KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)
            ),
            Action::OutdentSchemaLine
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Insert,
                false,
                false,
                false,
                PaneId::SchemaPath,
                KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
            ),
            Action::CompleteSchemaPath
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Insert,
                false,
                false,
                false,
                PaneId::SchemaPath,
                KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)
            ),
            Action::CompleteSchemaPathPrev
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Insert,
                false,
                false,
                false,
                PaneId::SchemaPath,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
            ),
            Action::CommitActiveEditor
        );
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Insert,
                false,
                false,
                false,
                PaneId::OutputPath,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
            ),
            Action::CommitActiveEditor
        );
    }

    #[test]
    fn z_then_w_toggles_main_fullwidth() {
        assert_eq!(
            Action::from_key(
                crate::app::state::AppMode::Editor,
                InputMode::Normal,
                false,
                false,
                true,
                PaneId::Form,
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE)
            ),
            Action::ToggleMainFullwidth
        );
    }
}
