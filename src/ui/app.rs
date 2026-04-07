use std::io;

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseButton, MouseEvent,
    MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::layout::{Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};
use tui_textarea::{CursorMove, TextArea};
use unicode_width::UnicodeWidthStr;

use crate::app::actions::Action;
use crate::app::reducer::reduce;
use crate::app::state::{
    AppState, FormArrayButtonKind, InputMode, MainFullwidthPane, PaneId, SelectionAnchor,
};
use crate::domain::form::{FormField, FormFieldKind, ResolveCtx, SchemaType, resolve_schema_at_path_with};

pub fn run_app(mut state: AppState) -> Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    let terminal = ratatui::init();
    let result = run_loop(terminal, &mut state);
    ratatui::restore();
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
    result
}

fn run_loop(mut terminal: DefaultTerminal, state: &mut AppState) -> Result<()> {
    loop {
        terminal.draw(|frame| render(frame, state))?;
        match event::read()? {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if state.screen_mode == crate::app::state::ScreenMode::ConfirmOverwrite {
                    let action = match key.code {
                        crossterm::event::KeyCode::Char('y')
                        | crossterm::event::KeyCode::Char('Y')
                        | crossterm::event::KeyCode::Enter => Action::ConfirmOverwrite,
                        crossterm::event::KeyCode::Char('n')
                        | crossterm::event::KeyCode::Char('N')
                        | crossterm::event::KeyCode::Esc => Action::CancelOverwrite,
                        _ => Action::Noop,
                    };
                    let should_quit = reduce(state, action)?;
                    if should_quit {
                        break;
                    }
                    continue;
                }
                let action = Action::from_key(
                    state.app_mode,
                    state.input_mode,
                    state.pending_g,
                    state.pending_d,
                    state.pending_z,
                    state.active_pane,
                    key,
                );
                let should_quit = reduce(state, action)?;
                if should_quit {
                    break;
                }
            }
            Event::Mouse(mouse) => {
                let size = terminal.size()?;
                handle_mouse(state, Rect::new(0, 0, size.width, size.height), mouse);
            }
            _ => {}
        }
    }
    Ok(())
}

fn render(frame: &mut Frame<'_>, state: &AppState) {
    let panes = pane_layout(state, frame.area());

    render_schema_path(frame, panes.schema_path, state);
    if state.is_pane_visible(PaneId::Schema) {
        render_schema_pane(frame, panes.schema, state);
    }
    render_form(frame, panes.form, state);
    render_output_path(frame, panes.output_path, state);
    render_filter_pane(frame, panes.filter, state);
    render_output(frame, panes.output, state);
    render_logs(frame, panes.log, state);
    render_footer(frame, panes.footer, state);
    if state.screen_mode == crate::app::state::ScreenMode::Help {
        render_help_overlay(frame, frame.area(), state);
    } else if state.screen_mode == crate::app::state::ScreenMode::ConfirmOverwrite {
        render_overwrite_overlay(frame, frame.area(), state);
    }
}

struct PaneLayout {
    schema_path: Rect,
    schema: Rect,
    form: Rect,
    output_path: Rect,
    filter: Rect,
    output: Rect,
    log: Rect,
    footer: Rect,
}

enum FormClickTarget {
    Field(usize, usize),
    Button(Vec<String>, FormArrayButtonKind),
}

fn pane_layout(state: &AppState, area: Rect) -> PaneLayout {
    let [schema_path, middle, log, footer] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Min(12),
            Constraint::Length(8),
            // Top border + 2 text lines (nested Blocks per row need more space than Length(2)).
            Constraint::Length(3),
        ])
        .areas(area);

    let empty = Rect::new(middle.x, middle.y, 0, 0);

    match state.main_fullwidth {
        Some(MainFullwidthPane::Schema) if state.is_pane_visible(PaneId::Schema) => {
            return PaneLayout {
                schema_path,
                schema: middle,
                form: empty,
                output_path: empty,
                filter: empty,
                output: empty,
                log,
                footer,
            };
        }
        Some(MainFullwidthPane::Form) => {
            return PaneLayout {
                schema_path,
                schema: empty,
                form: if state.is_pane_collapsed(PaneId::Form) {
                    Rect::new(middle.x, middle.y, middle.width, 3)
                } else {
                    middle
                },
                output_path: empty,
                filter: empty,
                output: empty,
                log,
                footer,
            };
        }
        Some(MainFullwidthPane::OutputColumn) => {
            let [output_path, filter, output] = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Length(5),
                    Constraint::Min(10),
                ])
                .areas(middle);
            return PaneLayout {
                schema_path,
                schema: empty,
                form: empty,
                output_path,
                filter,
                output,
                log,
                footer,
            };
        }
        _ => {}
    }

    let [left, right] = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .areas(middle);

    let (schema, form) = if state.is_pane_visible(PaneId::Schema) {
        let schema_constraint = if state.is_pane_collapsed(PaneId::Schema) {
            Constraint::Length(3)
        } else if state.is_pane_collapsed(PaneId::Form) {
            Constraint::Min(3)
        } else {
            Constraint::Percentage(50)
        };
        let form_constraint = if state.is_pane_collapsed(PaneId::Form) {
            Constraint::Length(3)
        } else if state.is_pane_collapsed(PaneId::Schema) {
            Constraint::Min(3)
        } else {
            Constraint::Percentage(50)
        };
        let [schema, form] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([schema_constraint, form_constraint])
            .areas(left);
        (schema, form)
    } else {
        let form = if state.is_pane_collapsed(PaneId::Form) {
            Rect::new(left.x, left.y, left.width, 3)
        } else {
            left
        };
        (Rect::new(left.x, left.y, 0, 0), form)
    };
    let [output_path, filter, output] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(5),
            Constraint::Min(10),
        ])
        .areas(right);
    PaneLayout {
        schema_path,
        schema,
        form,
        output_path,
        filter,
        output,
        log,
        footer,
    }
}

fn handle_mouse(state: &mut AppState, area: Rect, mouse: MouseEvent) {
    let panes = pane_layout(state, area);
    let x = mouse.column;
    let y = mouse.row;

    match mouse.kind {
        MouseEventKind::ScrollUp => {
            handle_scroll(state, panes, x, y, -1);
            return;
        }
        MouseEventKind::ScrollDown => {
            handle_scroll(state, panes, x, y, 1);
            return;
        }
        MouseEventKind::Down(MouseButton::Left) => {}
        _ => return,
    }

    if point_in_rect(panes.form, x, y) {
        state.focus_pane_at(PaneId::Form);
        if let Some(target) = form_click_target(state, panes.form, x, y) {
            match target {
                FormClickTarget::Field(row, col) => {
                    state.set_pane_cursor(PaneId::Form, row, col);
                    state.enter_insert_mode(false);
                }
                FormClickTarget::Button(array_path, kind) => {
                    if let Err(err) = state.activate_form_button(array_path, kind) {
                        state.log_error(format!("array action error: {err}"));
                    }
                }
            }
        }
        return;
    }
    if point_in_rect(panes.schema_path, x, y) {
        state.focus_pane_at(PaneId::SchemaPath);
        set_text_pane_cursor(
            state,
            PaneId::SchemaPath,
            panes.schema_path,
            x,
            y,
            !state.schema_candidates().is_empty(),
        );
        return;
    }
    if point_in_rect(panes.schema, x, y) {
        state.focus_pane_at(PaneId::Schema);
        let block = pane_block(state, PaneId::Schema);
        let inner = block.inner(panes.schema);
        let has_schema_footer = state.schema_error.is_some() && inner.height > 1;
        set_text_pane_cursor(
            state,
            PaneId::Schema,
            panes.schema,
            x,
            y,
            has_schema_footer,
        );
        return;
    }
    if point_in_rect(panes.output_path, x, y) {
        state.focus_pane_at(PaneId::OutputPath);
        set_text_pane_cursor(state, PaneId::OutputPath, panes.output_path, x, y, false);
        return;
    }
    if point_in_rect(panes.filter, x, y) {
        state.focus_pane_at(PaneId::Filter);
        set_text_pane_cursor(
            state,
            PaneId::Filter,
            panes.filter,
            x,
            y,
            state.filter_outcome.error.is_some(),
        );
        return;
    }
    if point_in_rect(panes.output, x, y) {
        state.focus_pane_at(PaneId::Output);
        let block = pane_block(state, PaneId::Output);
        let inner = block.inner(panes.output);
        let has_val_footer =
            state.schema_error.is_none() && !state.validation.is_valid && inner.height > 1;
        set_text_pane_cursor(state, PaneId::Output, panes.output, x, y, has_val_footer);
        return;
    }
    if point_in_rect(panes.log, x, y) {
        state.focus_pane_at(PaneId::Log);
        set_text_pane_cursor(state, PaneId::Log, panes.log, x, y, false);
    }
}

/// Mouse wheel notches needed to move the Form cursor by one field (higher = less sensitive).
pub(crate) const FORM_MOUSE_SCROLL_TICKS_PER_ROW: i8 = 3;

fn handle_scroll(state: &mut AppState, panes: PaneLayout, x: u16, y: u16, delta: isize) {
    let pane = if point_in_rect(panes.schema, x, y) {
        Some(PaneId::Schema)
    } else if point_in_rect(panes.form, x, y) {
        Some(PaneId::Form)
    } else if point_in_rect(panes.output, x, y) {
        Some(PaneId::Output)
    } else if point_in_rect(panes.log, x, y) {
        Some(PaneId::Log)
    } else {
        None
    };
    let Some(pane) = pane else {
        state.form_mouse_scroll_accum = 0;
        return;
    };

    state.focus_pane_at(pane);
    let current = state.pane_cursors.get(&pane).cloned().unwrap_or_default();

    if pane == PaneId::Form {
        let d = delta.clamp(i8::MIN as isize, i8::MAX as isize) as i8;
        state.form_mouse_scroll_accum = state
            .form_mouse_scroll_accum
            .saturating_add(d);
        let mut steps: isize = 0;
        while state.form_mouse_scroll_accum >= FORM_MOUSE_SCROLL_TICKS_PER_ROW {
            state.form_mouse_scroll_accum -= FORM_MOUSE_SCROLL_TICKS_PER_ROW;
            steps += 1;
        }
        while state.form_mouse_scroll_accum <= -FORM_MOUSE_SCROLL_TICKS_PER_ROW {
            state.form_mouse_scroll_accum += FORM_MOUSE_SCROLL_TICKS_PER_ROW;
            steps -= 1;
        }
        let next_row = if steps >= 0 {
            current.row.saturating_add(steps as usize)
        } else {
            current.row.saturating_sub((-steps) as usize)
        };
        state.set_pane_cursor(PaneId::Form, next_row, current.col);
        return;
    }

    state.form_mouse_scroll_accum = 0;
    let next_row = if delta >= 0 {
        current.row.saturating_add(delta as usize)
    } else {
        current.row.saturating_sub(delta.unsigned_abs())
    };
    state.set_pane_cursor(pane, next_row, current.col);
}

fn point_in_rect(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
}

fn set_text_pane_cursor(
    state: &mut AppState,
    pane: PaneId,
    area: Rect,
    x: u16,
    y: u16,
    has_footer: bool,
) {
    let block = pane_block(state, pane);
    let inner = block.inner(area);
    if !point_in_rect(inner, x, y) {
        return;
    }
    let content_area = split_footer_area(inner, has_footer).0;
    if !point_in_rect(content_area, x, y) {
        return;
    }

    let lines = pane_lines_for_mouse(state, pane);
    let current = state.pane_cursors.get(&pane).cloned().unwrap_or_default();
    let row_start = vertical_scroll_start(
        current.row,
        lines.len().max(1),
        content_area.height as usize,
    );
    let row = row_start + y.saturating_sub(content_area.y) as usize;
    let col = match pane {
        PaneId::Schema | PaneId::Output => {
            let gutter_width = lines.len().max(1).to_string().len().max(4);
            let separator_width = " │ ".chars().count();
            let content_width = content_area
                .width
                .saturating_sub((gutter_width + separator_width) as u16)
                as usize;
            let col_start = horizontal_scroll_start(current.col, content_width);
            let content_x = content_area.x + (gutter_width + separator_width) as u16;
            if x < content_x {
                col_start
            } else {
                col_start + x.saturating_sub(content_x) as usize
            }
        }
        _ => {
            let col_start = horizontal_scroll_start(current.col, content_area.width as usize);
            col_start + x.saturating_sub(content_area.x) as usize
        }
    };
    state.set_pane_cursor(pane, row, col);
}

fn pane_lines_for_mouse(state: &AppState, pane: PaneId) -> Vec<String> {
    match pane {
        PaneId::SchemaPath => vec![state.schema_path.schema_source.clone()],
        PaneId::OutputPath => vec![state.schema_path.output_path.clone()],
        PaneId::Schema => text_lines_for_mouse(&state.schema_text),
        PaneId::Filter => text_lines_for_mouse(&state.filter_text),
        PaneId::Output => {
            let lines: Vec<String> = state
                .filter_outcome
                .text
                .lines()
                .map(ToOwned::to_owned)
                .collect();
            if lines.is_empty() {
                vec![String::new()]
            } else {
                lines
            }
        }
        PaneId::Log => {
            if state.logs.is_empty() {
                vec![String::new()]
            } else {
                state.logs.clone()
            }
        }
        PaneId::Form => Vec::new(),
    }
}

fn text_lines_for_mouse(text: &str) -> Vec<String> {
    if text.is_empty() {
        vec![String::new()]
    } else {
        text.lines().map(ToOwned::to_owned).collect()
    }
}

fn form_click_target(state: &AppState, area: Rect, x: u16, y: u16) -> Option<FormClickTarget> {
    let block = pane_block(state, PaneId::Form);
    let inner = block.inner(area);
    if !point_in_rect(inner, x, y) || state.form_fields.is_empty() {
        return None;
    }

    let [_, body_area] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .areas(inner);
    if !point_in_rect(body_area, x, y) {
        return None;
    }

    let skip = state
        .pane_cursors
        .get(&PaneId::Form)
        .map(|cursor| cursor.row.saturating_sub(1))
        .unwrap_or(0);

    let mut cursor_y = body_area.y;
    let mut scan = 0usize;
    let n = state.form_fields.len();

    while scan < n {
        let remaining_height = body_area
            .y
            .saturating_add(body_area.height)
            .saturating_sub(cursor_y);
        if remaining_height == 0 {
            break;
        }

        if let Some(array_path) = array_path_for_form_group(&state.form_fields[scan].path) {
            let mut group_start = scan;
            while group_start > 0
                && array_path_for_form_group(&state.form_fields[group_start - 1].path).as_ref()
                    == Some(&array_path)
            {
                group_start -= 1;
            }
            let mut end = scan + 1;
            while end < n
                && array_path_for_form_group(&state.form_fields[end].path).as_ref()
                    == Some(&array_path)
            {
                end += 1;
            }
            if skip >= end {
                scan = end;
                continue;
            }

            let can_add = array_group_can_add(state, &array_path);
            let can_remove = array_group_can_remove(state, &array_path);
            let indices: Vec<usize> = (group_start..end).collect();
            let parts = partition_form_parts(
                &state.form_fields,
                &indices,
                Some(array_path.as_slice()),
                None,
            );
            let inner_body_h = form_part_heights(
                &parts,
                &state.form_fields,
                state,
                Some(array_path.as_slice()),
                skip,
            );
            let body_height: u16 =
                inner_body_h + if can_add || can_remove { 1 } else { 0 };
            let group_height = body_height.saturating_add(2);
            let draw_h = group_height.min(remaining_height);
            if draw_h < 3 {
                break;
            }
            let group_rect = Rect::new(body_area.x, cursor_y, body_area.width, draw_h);
            let group_inner = Block::default().borders(Borders::ALL).inner(group_rect);
            let inner_bottom = group_inner.y.saturating_add(group_inner.height);
            let (hit, content_end_cy) = hit_test_form_partitioned(
                &parts,
                &state.form_fields,
                state,
                x,
                y,
                group_inner.x,
                group_inner.y,
                group_inner.width,
                inner_bottom,
                Some(array_path.as_slice()),
                skip,
            );
            if hit.is_some() {
                return hit;
            }
            let group_y = content_end_cy;
            if (can_add || can_remove) && y == group_y && group_y < inner_bottom {
                let button_kind = if can_add && x < group_inner.x + (group_inner.width / 2) {
                    Some(FormArrayButtonKind::Add)
                } else if can_remove {
                    Some(FormArrayButtonKind::Remove)
                } else if can_add {
                    Some(FormArrayButtonKind::Add)
                } else {
                    None
                };
                if let Some(kind) = button_kind {
                    return Some(FormClickTarget::Button(array_path, kind));
                }
            }
            cursor_y += draw_h.saturating_add(1);
            scan = end;
            continue;
        }

        let mut run_start = scan;
        while run_start > 0
            && array_path_for_form_group(&state.form_fields[run_start - 1].path).is_none()
        {
            run_start -= 1;
        }
        let mut run_end = run_start;
        while run_end < n && array_path_for_form_group(&state.form_fields[run_end].path).is_none()
        {
            run_end += 1;
        }
        if skip >= run_end {
            scan = run_end;
            continue;
        }

        let run_indices: Vec<usize> = (run_start..run_end).collect();
        let parts = partition_form_parts(&state.form_fields, &run_indices, None, None);
        let body_bottom = body_area.y.saturating_add(body_area.height);
        let (hit, end_cy) = hit_test_form_partitioned(
            &parts,
            &state.form_fields,
            state,
            x,
            y,
            body_area.x,
            cursor_y,
            body_area.width,
            body_bottom,
            None,
            skip,
        );
        if hit.is_some() {
            return hit;
        }
        cursor_y = end_cy.saturating_add(1);
        scan = run_end;
    }
    None
}

fn render_schema_path(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let block = pane_block(state, PaneId::SchemaPath);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let has_footer = state.app_mode == crate::app::state::AppMode::Editor
        && !state.schema_candidates().is_empty()
        && inner.height > 1;
    let (content_area, footer_area) = split_footer_area(inner, has_footer);
    let lines = vec![state.schema_path.schema_source.clone()];
    let highlighted = highlight_lines(
        &lines,
        state,
        PaneId::SchemaPath,
        content_area.width as usize,
        content_area.height as usize,
    );
    frame.render_widget(Paragraph::new(highlighted.lines), content_area);
    render_insert_bar_cursor(frame, content_area, state, PaneId::SchemaPath, lines.len());
    if let Some(footer_area) = footer_area {
        if let Some(line) = schema_candidates_footer_line(state) {
            frame.render_widget(
                Paragraph::new(fit_text(&line, footer_area.width as usize)).style(
                    Style::default()
                        .fg(pane_color(PaneId::SchemaPath))
                        .add_modifier(Modifier::BOLD),
                ),
                footer_area,
            );
        }
    }
}

fn render_output_path(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    render_lines_pane(
        frame,
        area,
        state,
        PaneId::OutputPath,
        std::slice::from_ref(&state.schema_path.output_path),
    );
}

fn render_schema_pane(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let block = pane_block(state, PaneId::Schema);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 || state.is_pane_collapsed(PaneId::Schema) {
        return;
    }

    let has_error = state.schema_error.is_some() && inner.height > 1;
    let (content_area, error_area) = split_footer_area(inner, has_error);

    let lines: Vec<String> = if state.schema_text.is_empty() {
        vec![String::new()]
    } else {
        state.schema_text.lines().map(ToOwned::to_owned).collect()
    };
    render_numbered_text_lines(frame, content_area, state, PaneId::Schema, &lines);

    if let (Some(error), Some(error_area)) = (&state.schema_error, error_area) {
        frame.render_widget(
            Paragraph::new(fit_text(
                &schema_error_line(error),
                error_area.width as usize,
            ))
            .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            error_area,
        );
    }
}

fn render_filter_pane(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let block = pane_block(state, PaneId::Filter);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let has_footer = state.filter_outcome.error.is_some() && inner.height > 1;
    let (content_area, footer_area) = split_footer_area(inner, has_footer);
    let lines: Vec<String> = if state.filter_text.is_empty() {
        vec![String::new()]
    } else {
        state.filter_text.lines().map(ToOwned::to_owned).collect()
    };
    let highlighted = highlight_lines(
        &lines,
        state,
        PaneId::Filter,
        content_area.width as usize,
        content_area.height as usize,
    );
    frame.render_widget(Paragraph::new(highlighted.lines), content_area);
    render_insert_bar_cursor(frame, content_area, state, PaneId::Filter, lines.len());

    if let (Some(error), Some(footer_area)) = (&state.filter_outcome.error, footer_area) {
        frame.render_widget(
            Paragraph::new(fit_text(
                &format!("ERR | {error}"),
                footer_area.width as usize,
            ))
            .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            footer_area,
        );
    }
}

fn render_form(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let block = pane_block(state, PaneId::Form);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 || state.is_pane_collapsed(PaneId::Form) {
        return;
    }

    let [breadcrumb_area, body_area] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .areas(inner);
    if let Some(breadcrumb) = state.current_form_breadcrumb() {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("PATH ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    fit_text(
                        &breadcrumb,
                        breadcrumb_area.width.saturating_sub(5) as usize,
                    ),
                    Style::default()
                        .fg(pane_color(PaneId::Form))
                        .add_modifier(Modifier::BOLD),
                ),
            ])),
            breadcrumb_area,
        );
    }

    if state.form_fields.is_empty() {
        frame.render_widget(
            Paragraph::new("No editable scalar fields").wrap(Wrap { trim: false }),
            body_area,
        );
        return;
    }

    let mut y = body_area.y;
    let skip = state
        .pane_cursors
        .get(&PaneId::Form)
        .map(|cursor| cursor.row.saturating_sub(1))
        .unwrap_or(0);

    let body_bottom = body_area.y.saturating_add(body_area.height);
    let mut scan = 0usize;
    let n = state.form_fields.len();

    while scan < n {
        let remaining_height = body_bottom.saturating_sub(y);
        if remaining_height == 0 {
            break;
        }

        if let Some(array_path) = array_path_for_form_group(&state.form_fields[scan].path) {
            let mut group_start = scan;
            while group_start > 0
                && array_path_for_form_group(&state.form_fields[group_start - 1].path).as_ref()
                    == Some(&array_path)
            {
                group_start -= 1;
            }
            let mut end = scan + 1;
            while end < n
                && array_path_for_form_group(&state.form_fields[end].path).as_ref()
                    == Some(&array_path)
            {
                end += 1;
            }
            if skip >= end {
                scan = end;
                continue;
            }

            let title = array_group_title(state, &array_path);
            let item_count = array_group_item_count(state, &array_path);
            let can_add = array_group_can_add(state, &array_path);
            let can_remove = array_group_can_remove(state, &array_path);
            let indices: Vec<usize> = (group_start..end).collect();
            let parts = partition_form_parts(
                &state.form_fields,
                &indices,
                Some(array_path.as_slice()),
                None,
            );
            let inner_body_h = form_part_heights(
                &parts,
                &state.form_fields,
                state,
                Some(array_path.as_slice()),
                skip,
            );
            let body_height: u16 =
                inner_body_h + if can_add || can_remove { 1 } else { 0 };
            let group_height = body_height.saturating_add(2);
            let draw_h = group_height.min(remaining_height);
            if draw_h < 3 {
                if remaining_height >= 1 {
                    let msg = fit_text(form_scroll_hint_line(), body_area.width as usize);
                    frame.render_widget(
                        Paragraph::new(msg).style(Style::default().fg(Color::DarkGray)),
                        Rect::new(body_area.x, y, body_area.width, 1),
                    );
                }
                break;
            }
            let group_rect = Rect::new(body_area.x, y, body_area.width, draw_h);
            let mut title_suffix = format!(" ({item_count} items)");
            if can_add {
                title_suffix.push_str("  [+] add");
            }
            if can_remove {
                title_suffix.push_str("  [-] remove");
            }
            let group_block = Block::default()
                .title(Line::from(vec![
                    Span::styled(title, Style::default().fg(Color::White)),
                    Span::styled(title_suffix, Style::default().fg(Color::DarkGray)),
                ]))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray));
            let group_inner = group_block.inner(group_rect);
            frame.render_widget(group_block, group_rect);

            let group_inner_bottom = group_inner.y.saturating_add(group_inner.height);
            let group_y = render_form_partitioned(
                &parts,
                &state.form_fields,
                state,
                frame,
                group_inner.x,
                group_inner.y,
                group_inner.width,
                group_inner_bottom,
                Some(array_path.as_slice()),
                skip,
            );
            if (can_add || can_remove) && group_y < group_inner_bottom {
                let mut spans = Vec::new();
                if can_add {
                    spans.push(array_button_span(
                        "+ Add Item",
                        state.is_form_button_focused(&array_path, FormArrayButtonKind::Add),
                    ));
                }
                if can_add && can_remove {
                    spans.push(Span::raw("   "));
                }
                if can_remove {
                    spans.push(array_button_span(
                        "- Remove Item",
                        state.is_form_button_focused(&array_path, FormArrayButtonKind::Remove),
                    ));
                }
                frame.render_widget(
                    Paragraph::new(Line::from(spans)),
                    Rect::new(group_inner.x, group_y, group_inner.width, 1),
                );
            }
            y += draw_h.saturating_add(1);
            scan = end;
            continue;
        }

        let mut run_start = scan;
        while run_start > 0
            && array_path_for_form_group(&state.form_fields[run_start - 1].path).is_none()
        {
            run_start -= 1;
        }
        let mut run_end = run_start;
        while run_end < n && array_path_for_form_group(&state.form_fields[run_end].path).is_none()
        {
            run_end += 1;
        }
        if skip >= run_end {
            scan = run_end;
            continue;
        }

        let run_indices: Vec<usize> = (run_start..run_end).collect();
        let parts = partition_form_parts(&state.form_fields, &run_indices, None, None);
        y = render_form_partitioned(
            &parts,
            &state.form_fields,
            state,
            frame,
            body_area.x,
            y,
            body_area.width,
            body_bottom,
            None,
            skip,
        );
        y = y.saturating_add(1);
        scan = run_end;
    }
}

fn render_form_field(frame: &mut Frame<'_>, state: &AppState, index: usize, area: Rect) {
    let field = &state.form_fields[index];
    let input_height = field_height(field);
    let mut y = area.y;
    let active = index == state.pane_cursors[&PaneId::Form].row;
    let label_style = if active {
        Style::default()
            .fg(pane_color(PaneId::Form))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    let required = if field.required { " *" } else { "" };
    frame.render_widget(
        Paragraph::new(format!("{}{}", field.label, required)).style(label_style),
        Rect::new(area.x, y, area.width, 1),
    );
    y += 1;

    if let Some(description) = &field.description {
        frame.render_widget(
            Paragraph::new(description.clone()).style(Style::default().fg(Color::DarkGray)),
            Rect::new(area.x, y, area.width, 1),
        );
        y += 1;
    }

    let input_rect = Rect::new(area.x, y, area.width, input_height);
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if active {
            pane_color(PaneId::Form)
        } else {
            Color::DarkGray
        }));
    let input_inner = input_rect.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });
    frame.render_widget(input_block, input_rect);

    if field.enum_options.is_some() {
        frame.render_widget(
            Paragraph::new(enum_field_line(field, active)).wrap(Wrap { trim: false }),
            input_inner,
        );
    } else if active {
        let mut textarea = build_field_textarea(field);
        let (cursor_line, cursor_col) = state.form_textarea_cursor(index);
        textarea.move_cursor(CursorMove::Jump(
            cursor_line.min(u16::MAX as usize) as u16,
            cursor_col.min(u16::MAX as usize) as u16,
        ));
        frame.render_widget(&textarea, input_inner);
    } else {
        frame.render_widget(
            Paragraph::new(field.edit_buffer.clone()).wrap(Wrap { trim: false }),
            input_inner,
        );
    }
    y += input_height;

    if let Some(error) = state.field_errors.get(&field.key) {
        frame.render_widget(
            Paragraph::new(format!("Error: {error}")).style(Style::default().fg(Color::Red)),
            Rect::new(area.x, y, area.width, 1),
        );
    }
}

fn field_render_height(state: &AppState, field: &crate::domain::form::FormField) -> u16 {
    let mut needed = 1 + field_height(field) + 1;
    if field.description.is_some() {
        needed += 1;
    }
    if state.field_errors.contains_key(&field.key) {
        needed += 1;
    }
    needed
}

#[derive(Clone)]
enum FormPart {
    Field(usize),
    ObjectFrame(Vec<String>, Vec<usize>),
}

fn object_frame_stack(field: &FormField) -> Vec<Vec<String>> {
    let path = &field.path;
    let max_k = match field.kind {
        FormFieldKind::Scalar | FormFieldKind::OneOfSelector { .. } => path.len(),
    };
    let mut out = Vec::new();
    for k in 1..max_k {
        if path[k - 1].parse::<usize>().is_ok() {
            continue;
        }
        out.push(path[..k].to_vec());
    }
    out
}

/// Drop leading object-frame prefixes that lie on the path to `parent` (inclusive).
/// This avoids infinite `ObjectFrame` recursion when the parent path is deeper than one
/// stack segment (e.g. inside `cell_configuration.working_electrode` the first frame
/// can still be `cell_configuration`).
fn trim_frames_against_parent_path(frames: &mut Vec<Vec<String>>, parent: &[String]) {
    if parent.is_empty() {
        return;
    }
    while let Some(frm) = frames.first() {
        if frm.len() <= parent.len() && parent[..frm.len()] == frm[..] {
            frames.remove(0);
        } else {
            break;
        }
    }
}

fn trimmed_object_frames(
    field: &FormField,
    array_ctx: Option<&[String]>,
    parent_object: Option<&[String]>,
) -> Vec<Vec<String>> {
    let mut f = object_frame_stack(field);
    if let Some(ap) = array_ctx {
        trim_frames_against_parent_path(&mut f, ap);
    }
    if let Some(p) = parent_object {
        trim_frames_against_parent_path(&mut f, p);
    }
    f
}

fn object_frame_title(state: &AppState, prefix: &[String]) -> String {
    let ctx = ResolveCtx {
        root: &state.schema_json,
        instance: Some(&state.output_json),
        choices: Some(&state.one_of_choices),
    };
    resolve_schema_at_path_with(prefix, ctx)
        .ok()
        .and_then(|schema| schema.get("title").and_then(serde_json::Value::as_str))
        .map(ToOwned::to_owned)
        .or_else(|| prefix.last().cloned())
        .unwrap_or_else(|| "Object".to_owned())
}

fn partition_form_parts(
    fields: &[FormField],
    indices: &[usize],
    array_ctx: Option<&[String]>,
    parent_object: Option<&[String]>,
) -> Vec<FormPart> {
    let mut parts = Vec::new();
    let mut i = 0;
    while i < indices.len() {
        let idx = indices[i];
        let fr = trimmed_object_frames(&fields[idx], array_ctx, parent_object);
        if fr.is_empty() {
            parts.push(FormPart::Field(idx));
            i += 1;
            continue;
        }
        let outer = fr[0].clone();
        let mut group = vec![idx];
        i += 1;
        while i < indices.len() {
            let fr2 = trimmed_object_frames(&fields[indices[i]], array_ctx, parent_object);
            if fr2.first() == Some(&outer) {
                group.push(indices[i]);
                i += 1;
            } else {
                break;
            }
        }
        parts.push(FormPart::ObjectFrame(outer, group));
    }
    parts
}

fn form_part_heights(
    parts: &[FormPart],
    fields: &[FormField],
    state: &AppState,
    array_ctx: Option<&[String]>,
    skip_field_index: usize,
) -> u16 {
    let mut total = 0u16;
    for part in parts {
        match part {
            FormPart::Field(idx) => {
                if *idx < skip_field_index {
                    continue;
                }
                total = total.saturating_add(field_render_height(state, &fields[*idx]));
            }
            FormPart::ObjectFrame(prefix, sub) => {
                let inner = partition_form_parts(fields, sub, array_ctx, Some(prefix.as_slice()));
                let inner_h = form_part_heights(&inner, fields, state, array_ctx, skip_field_index);
                if inner_h == 0 {
                    continue;
                }
                total = total.saturating_add(inner_h.saturating_add(2).saturating_add(1));
            }
        }
    }
    total
}

fn form_scroll_hint_line() -> &'static str {
    "▼ More below (move form cursor down with j or ↓)"
}

fn render_form_partitioned(
    parts: &[FormPart],
    fields: &[FormField],
    state: &AppState,
    frame: &mut Frame<'_>,
    ox: u16,
    mut cy: u16,
    width: u16,
    body_bottom: u16,
    array_ctx: Option<&[String]>,
    skip_field_index: usize,
) -> u16 {
    for part in parts {
        let space = body_bottom.saturating_sub(cy);
        if space == 0 {
            break;
        }
        match part {
            FormPart::Field(idx) => {
                if *idx < skip_field_index {
                    continue;
                }
                let h = field_render_height(state, &fields[*idx]);
                if cy.saturating_add(h) <= body_bottom {
                    render_form_field(
                        frame,
                        state,
                        *idx,
                        Rect::new(ox, cy, width, h),
                    );
                    cy = cy.saturating_add(h);
                } else {
                    if space >= 1 {
                        let msg = fit_text(form_scroll_hint_line(), width as usize);
                        frame.render_widget(
                            Paragraph::new(msg).style(Style::default().fg(Color::DarkGray)),
                            Rect::new(ox, cy, width, 1),
                        );
                        cy = cy.saturating_add(1);
                    }
                    break;
                }
            }
            FormPart::ObjectFrame(prefix, sub) => {
                let inner_parts =
                    partition_form_parts(fields, sub, array_ctx, Some(prefix.as_slice()));
                let inner_h =
                    form_part_heights(&inner_parts, fields, state, array_ctx, skip_field_index);
                if inner_h == 0 {
                    continue;
                }
                let group_h = inner_h.saturating_add(2);
                let draw_h = group_h.min(space);
                if draw_h < 3 {
                    if space >= 1 {
                        let title = object_frame_title(state, prefix);
                        let msg = fit_text(
                            &format!("▼ {title} … {}", form_scroll_hint_line()),
                            width as usize,
                        );
                        frame.render_widget(
                            Paragraph::new(msg).style(Style::default().fg(Color::DarkGray)),
                            Rect::new(ox, cy, width, 1),
                        );
                        cy = cy.saturating_add(1);
                    }
                    break;
                }
                let rect = Rect::new(ox, cy, width, draw_h);
                let title = object_frame_title(state, prefix);
                let block = Block::default()
                    .title(Line::from(Span::styled(
                        title,
                        Style::default().fg(Color::White),
                    )))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray));
                let inner = block.inner(rect);
                frame.render_widget(block, rect);
                let inner_bottom = inner.y.saturating_add(inner.height);
                render_form_partitioned(
                    &inner_parts,
                    fields,
                    state,
                    frame,
                    inner.x,
                    inner.y,
                    inner.width,
                    inner_bottom,
                    array_ctx,
                    skip_field_index,
                );
                cy = cy.saturating_add(draw_h.saturating_add(1));
            }
        }
    }
    cy
}

fn hit_test_form_partitioned(
    parts: &[FormPart],
    fields: &[FormField],
    state: &AppState,
    x: u16,
    y: u16,
    ox: u16,
    mut cy: u16,
    width: u16,
    body_bottom: u16,
    array_ctx: Option<&[String]>,
    skip_field_index: usize,
) -> (Option<FormClickTarget>, u16) {
    for part in parts {
        let space = body_bottom.saturating_sub(cy);
        if space == 0 {
            break;
        }
        match part {
            FormPart::Field(idx) => {
                if *idx < skip_field_index {
                    continue;
                }
                let field = &fields[*idx];
                let h = field_render_height(state, field);
                if cy.saturating_add(h) <= body_bottom {
                    let input_height = field_height(field);
                    let label_y = cy;
                    let mut row = cy.saturating_add(1);
                    if field.description.is_some() {
                        row = row.saturating_add(1);
                    }
                    let input_rect = Rect::new(ox, row, width, input_height);
                    let input_inner = input_rect.inner(Margin {
                        vertical: 1,
                        horizontal: 1,
                    });

                    if y == label_y {
                        return (Some(FormClickTarget::Field(*idx, 0)), cy.saturating_add(h));
                    }
                    if point_in_rect(input_rect, x, y) {
                        let col = if point_in_rect(input_inner, x, y) {
                            x.saturating_sub(input_inner.x) as usize
                        } else {
                            0
                        };
                        return (Some(FormClickTarget::Field(*idx, col)), cy.saturating_add(h));
                    }
                    cy = cy.saturating_add(h);
                } else {
                    if space >= 1 {
                        cy = cy.saturating_add(1);
                    }
                    break;
                }
            }
            FormPart::ObjectFrame(prefix, sub) => {
                let inner_parts =
                    partition_form_parts(fields, sub, array_ctx, Some(prefix.as_slice()));
                let inner_h =
                    form_part_heights(&inner_parts, fields, state, array_ctx, skip_field_index);
                if inner_h == 0 {
                    continue;
                }
                let group_h = inner_h.saturating_add(2);
                let draw_h = group_h.min(space);
                if draw_h < 3 {
                    if space >= 1 {
                        cy = cy.saturating_add(1);
                    }
                    break;
                }
                let rect = Rect::new(ox, cy, width, draw_h);
                let inner = Block::default().borders(Borders::ALL).inner(rect);
                let inner_bottom = inner.y.saturating_add(inner.height);
                let (hit, _) = hit_test_form_partitioned(
                    &inner_parts,
                    fields,
                    state,
                    x,
                    y,
                    inner.x,
                    inner.y,
                    inner.width,
                    inner_bottom,
                    array_ctx,
                    skip_field_index,
                );
                if hit.is_some() {
                    return (hit, cy.saturating_add(draw_h.saturating_add(1)));
                }
                cy = cy.saturating_add(draw_h.saturating_add(1));
            }
        }
    }
    (None, cy)
}

fn render_output(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let block = pane_block(state, PaneId::Output);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let show_val_footer =
        state.schema_error.is_none() && !state.validation.is_valid && inner.height > 1;
    let (content_area, val_footer) = split_footer_area(inner, show_val_footer);

    let lines: Vec<String> = if state.filter_outcome.text.is_empty() {
        vec![String::new()]
    } else {
        state
            .filter_outcome
            .text
            .lines()
            .map(ToOwned::to_owned)
            .collect()
    };
    render_numbered_text_lines(frame, content_area, state, PaneId::Output, &lines);

    if let (true, Some(fa)) = (show_val_footer, val_footer) {
        frame.render_widget(
            Paragraph::new(fit_text(
                &format!("validation: {}", state.validation.status_line()),
                fa.width as usize,
            ))
            .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            fa,
        );
    }
}

fn render_logs(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let block = pane_block(state, PaneId::Log);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let cursor = state
        .pane_cursors
        .get(&PaneId::Log)
        .cloned()
        .unwrap_or_default();
    let lines = if state.logs.is_empty() {
        vec![String::new()]
    } else {
        state.logs.clone()
    };
    let row_start = vertical_scroll_start(cursor.row, lines.len(), inner.height as usize);
    let visible = lines
        .iter()
        .enumerate()
        .skip(row_start)
        .take(inner.height as usize);
    let selection = selection_for_pane(state, PaneId::Log);
    let insert_mode = state.input_mode == InputMode::Insert && state.active_pane == PaneId::Log;
    let gutter_width = state
        .next_log_line
        .saturating_sub(1)
        .max(1)
        .to_string()
        .len()
        .max(4);
    let separator = " │ ";
    let content_width = inner
        .width
        .saturating_sub((gutter_width + separator.chars().count()) as u16)
        as usize;
    let content_offset = horizontal_scroll_start(
        cursor
            .col
            .saturating_sub(gutter_width + separator.chars().count()),
        content_width,
    );
    let rendered: Vec<Line<'static>> = visible
        .map(|(row, line)| {
            render_log_line(
                line,
                row,
                cursor.row,
                cursor.col,
                content_offset,
                content_width,
                gutter_width,
                separator,
                insert_mode,
                pane_color(PaneId::Log),
                selection,
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(rendered), inner);
}

fn render_numbered_text_lines(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    pane: PaneId,
    lines: &[String],
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let cursor = state.pane_cursors.get(&pane).cloned().unwrap_or_default();
    let selection = selection_for_pane(state, pane);
    let insert_mode = state.input_mode == InputMode::Insert && state.active_pane == pane;
    let row_start = vertical_scroll_start(cursor.row, lines.len().max(1), area.height as usize);
    let gutter_width = lines.len().max(1).to_string().len().max(4);
    let separator = " │ ";
    let content_width =
        area.width
            .saturating_sub((gutter_width + separator.chars().count()) as u16) as usize;
    let content_offset = horizontal_scroll_start(cursor.col, content_width);
    let use_terminal_cursor = insert_mode;

    let rendered: Vec<Line<'static>> = lines
        .iter()
        .enumerate()
        .skip(row_start)
        .take(area.height as usize)
        .map(|(row, line)| {
            render_numbered_text_line(
                line,
                row,
                cursor.row,
                cursor.col,
                content_offset,
                content_width,
                gutter_width,
                separator,
                insert_mode && !use_terminal_cursor,
                pane_color(pane),
                selection,
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(rendered), area);

    if use_terminal_cursor
        && cursor.row >= row_start
        && cursor.row < row_start.saturating_add(area.height as usize)
        && content_width > 0
    {
        let visible_col = cursor.col.saturating_sub(content_offset).min(content_width);
        let cursor_x =
            area.x + (gutter_width + separator.chars().count()) as u16 + visible_col as u16;
        let cursor_y = area.y + (cursor.row - row_start) as u16;
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::White));
    let inner = block.inner(area);
    let w = inner.width.max(1) as usize;

    let footer_widget = if inner.height >= 2 {
        Paragraph::new(vec![
            footer_status_line(state, w),
            footer_keys_line(state, w),
        ])
        .wrap(Wrap { trim: true })
        .block(block)
    } else {
        Paragraph::new(footer_line_single_row(state, w))
            .wrap(Wrap { trim: true })
            .block(block)
    };
    frame.render_widget(footer_widget, area);
}

/// Styled key cap for footer / help (high contrast).
fn key_cap_span(label: impl Into<String>) -> Span<'static> {
    Span::styled(
        format!(" {} ", label.into()),
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
}

fn footer_sep_dot() -> Span<'static> {
    Span::styled("  ·  ", Style::default().fg(Color::DarkGray))
}

fn help_section_heading(title: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::styled("▸ ", Style::default().fg(Color::Yellow)),
        Span::styled(
            title,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

fn help_key_desc_line(pairs: &[(&'static str, &'static str)]) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, (key, desc)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(
                "    ",
                Style::default().fg(Color::DarkGray),
            ));
        }
        spans.push(key_cap_span(*key));
        spans.push(Span::styled(
            format!(" {desc}"),
            Style::default().fg(Color::White),
        ));
    }
    Line::from(spans)
}

fn help_overlay_lines(state: &AppState) -> Vec<Line<'static>> {
    let mode_label = match state.app_mode {
        crate::app::state::AppMode::Standard => "standard",
        crate::app::state::AppMode::Editor => "editor",
    };
    let pane_hint = match state.app_mode {
        crate::app::state::AppMode::Editor => "1–7 jump to pane · Tab / Shift-Tab cycle",
        crate::app::state::AppMode::Standard => "1–6 jump to pane · Tab / Shift-Tab cycle",
    };

    vec![
        Line::from(vec![
            Span::styled(
                "Keyboard reference",
                Style::default()
                    .fg(pane_color(state.active_pane))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled(
                format!("{mode_label} mode"),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![Span::styled(
            "? or Esc closes this overlay",
            Style::default().fg(Color::DarkGray),
        )]),
        Line::from(""),
        help_section_heading("Essentials"),
        help_key_desc_line(&[
            ("?", "Help overlay"),
            ("q", "Quit app"),
            ("Esc", "Normal / commit editor / cancel"),
        ]),
        help_key_desc_line(&[("u", "Undo"), ("^r", "Redo"), ("r", "Save JSON to output path")]),
        Line::from(""),
        help_section_heading("Panes & focus"),
        Line::from(vec![
            Span::styled(pane_hint, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![Span::styled(
            "Click a pane or form control with the mouse to focus",
            Style::default().fg(Color::DarkGray),
        )]),
        Line::from(""),
        help_section_heading("Movement (Normal / Visual)"),
        help_key_desc_line(&[
            ("h j k l", "Move"),
            ("arrows", "Move"),
            ("0 $", "Line start / end"),
        ]),
        help_key_desc_line(&[
            ("w b e", "Word forward / back / end"),
            ("g g", "Buffer top"),
            ("G", "Buffer bottom"),
        ]),
        Line::from(""),
        help_section_heading("Modes"),
        help_key_desc_line(&[
            ("i", "Insert before"),
            ("a", "Append after cursor"),
            ("o O", "Open line below / above"),
        ]),
        help_key_desc_line(&[
            ("v", "Visual selection"),
            ("Enter", "On Form: toggle field edit (single-line)"),
        ]),
        Line::from(vec![Span::styled(
            "Insert: Esc or Ctrl-[ commits · Tab / Shift-Tab depends on pane (see below)",
            Style::default().fg(Color::DarkGray),
        )]),
        Line::from(""),
        help_section_heading("Text & deletion (Normal)"),
        help_key_desc_line(&[
            ("x", "Delete char"),
            ("dd", "Delete line"),
            ("D d$", "Delete to line end"),
        ]),
        help_key_desc_line(&[
            ("dw", "Delete word forward"),
            ("d0", "Delete to line start"),
        ]),
        Line::from(""),
        help_section_heading("Form"),
        help_key_desc_line(&[
            ("Tab", "Next field / button (insert)"),
            ("S-Tab", "Previous field / button"),
        ]),
        help_key_desc_line(&[
            ("h l", "Cycle enum / boolean"),
            ("+ -", "Add / remove array item"),
            ("R", "Reset entire form to schema defaults"),
        ]),
        Line::from(vec![Span::styled(
            "oneOf: type a letter or use h / l to switch branch (first alternative only)",
            Style::default().fg(Color::DarkGray),
        )]),
        Line::from(""),
        help_section_heading("Schema path & output path"),
        help_key_desc_line(&[
            ("Tab", "Next schema path candidate"),
            ("S-Tab", "Previous candidate"),
            ("Enter", "Commit single-line value"),
        ]),
        Line::from(""),
        help_section_heading("Schema editor (JSON)"),
        help_key_desc_line(&[
            ("Tab", "Indent +4 spaces"),
            ("S-Tab", "Outdent"),
        ]),
        Line::from(vec![Span::styled(
            "Invalid JSON is reported in-pane and in the log; last good schema is kept",
            Style::default().fg(Color::DarkGray),
        )]),
        Line::from(""),
        help_section_heading("Filter & output"),
        Line::from(vec![
            Span::styled("Filter (jq-like subset): ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                ".   .foo   .foo.bar   .items[0]",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        help_section_heading("Fold & layout"),
        help_key_desc_line(&[
            ("z a", "Toggle pane fold"),
            ("z c z o", "Collapse / expand pane"),
        ]),
        Line::from(vec![Span::styled(
            "z w: full-width Schema, Form, or Output column (toggle) while that stack is focused",
            Style::default().fg(Color::DarkGray),
        )]),
        Line::from(""),
        help_section_heading("Overwrite dialog"),
        help_key_desc_line(&[("y Enter", "Overwrite file"), ("n Esc", "Cancel")]),
        Line::from(""),
        help_section_heading("Visual mode"),
        help_key_desc_line(&[("y", "Yank selection to clipboard"), ("v Esc", "Leave visual")]),
    ]
}

fn render_help_overlay(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let popup = centered_rect(area, 90, 88);
    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Help ",
                Style::default()
                    .fg(Color::Black)
                    .bg(pane_color(state.active_pane))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                "jsonschema-form-tui",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                "? / Esc close",
                Style::default().fg(Color::DarkGray),
            ),
        ]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pane_color(state.active_pane)));
    let inner = block.inner(popup);
    let lines = help_overlay_lines(state);
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(Color::White)),
        inner,
    );
}

fn render_overwrite_overlay(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let popup = centered_rect(area, 60, 20);
    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                "Overwrite File",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                "y/Enter confirm  n/Esc cancel",
                Style::default().fg(Color::DarkGray),
            ),
        ]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(popup);
    let path = state
        .overwrite_path
        .as_deref()
        .unwrap_or(state.schema_path.output_path.as_str());
    let lines = vec![
        Line::from("The target file already exists."),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "Path: ",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(path.to_owned(), Style::default().fg(Color::Yellow)),
        ]),
    ];
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn split_footer_area(inner: Rect, has_footer: bool) -> (Rect, Option<Rect>) {
    if has_footer && inner.height > 1 {
        let [content, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .areas(inner);
        (content, Some(footer))
    } else {
        (inner, None)
    }
}

fn render_lines_pane(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    pane: PaneId,
    lines: &[String],
) {
    let block = pane_block(state, pane);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let highlighted = highlight_lines(
        lines,
        state,
        pane,
        inner.width as usize,
        inner.height as usize,
    );
    let paragraph = Paragraph::new(highlighted.lines);
    frame.render_widget(paragraph, inner);
    render_insert_bar_cursor(frame, inner, state, pane, lines.len());
}

fn pane_block(state: &AppState, pane: PaneId) -> Block<'static> {
    let active = state.active_pane == pane;
    let pane_color = pane_color(pane);
    let title_style = if active {
        Style::default().fg(pane_color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    let mut pane_title = state.pane_title(pane);
    if let Some((current, total)) = state.pane_line_progress(pane) {
        pane_title.push_str(&format!(" ({current}/{total})"));
    }
    if state.is_pane_collapsed(pane) {
        pane_title.push_str(" [collapsed]");
    }
    if let Some(fw) = state.main_fullwidth {
        let matches = match (fw, pane) {
            (MainFullwidthPane::Schema, PaneId::Schema) => true,
            (MainFullwidthPane::Form, PaneId::Form) => true,
            (
                MainFullwidthPane::OutputColumn,
                PaneId::OutputPath | PaneId::Filter | PaneId::Output,
            ) => true,
            _ => false,
        };
        if matches {
            pane_title.push_str(" [fullwidth]");
        }
    }
    let mut title = vec![Span::styled(pane_title, title_style)];
    if active {
        match state.input_mode {
            InputMode::Insert => {
                title.push(Span::raw(" "));
                title.push(Span::styled(
                    "INSERT",
                    Style::default()
                        .fg(pane_color)
                        .add_modifier(Modifier::BOLD | Modifier::REVERSED),
                ));
            }
            InputMode::Visual => {
                title.push(Span::raw(" "));
                title.push(Span::styled(
                    "VISUAL",
                    Style::default()
                        .fg(pane_color)
                        .add_modifier(Modifier::BOLD | Modifier::REVERSED),
                ));
            }
            InputMode::Normal => {}
        }
    }
    Block::default()
        .title(Line::from(title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if active { pane_color } else { Color::White }))
}

fn fit_text(input: &str, width: usize) -> String {
    if UnicodeWidthStr::width(input) <= width {
        return input.to_owned();
    }

    let mut result = String::new();
    for ch in input.chars() {
        let next = format!("{result}{ch}");
        if UnicodeWidthStr::width(next.as_str()) > width.saturating_sub(1) {
            result.push('…');
            break;
        }
        result.push(ch);
    }
    result
}

struct HighlightedLines {
    lines: Vec<Line<'static>>,
}

fn highlight_lines(
    lines: &[String],
    state: &AppState,
    pane: PaneId,
    width: usize,
    height: usize,
) -> HighlightedLines {
    let cursor = state.pane_cursors.get(&pane).cloned().unwrap_or_default();
    let selection = selection_for_pane(state, pane);
    let insert_mode = state.input_mode == InputMode::Insert && state.active_pane == pane;
    let row_start = vertical_scroll_start(cursor.row, lines.len(), height);
    let col_start = horizontal_scroll_start(cursor.col, width);
    let lines = lines
        .iter()
        .enumerate()
        .skip(row_start)
        .take(height)
        .map(|(row, line)| {
            let rendered = highlight_line(
                line,
                row,
                cursor.row,
                cursor.col,
                col_start,
                width,
                insert_mode,
                pane_color(pane),
                selection,
            );
            rendered
        })
        .collect();
    HighlightedLines { lines }
}

fn highlight_line(
    line: &str,
    row: usize,
    cursor_row: usize,
    col: usize,
    col_start: usize,
    width: usize,
    insert_mode: bool,
    color: Color,
    selection: Option<(SelectionAnchor, SelectionAnchor)>,
) -> Line<'static> {
    if line.is_empty() {
        let is_cursor = row == cursor_row && col == 0;
        return Line::from(Span::styled(
            " ",
            Style::default()
                .fg(Color::Black)
                .bg(if is_cursor { color } else { Color::DarkGray }),
        ));
    }

    let mut spans = Vec::new();
    let chars: Vec<char> = line.chars().collect();

    for (index, ch) in chars.iter().enumerate().skip(col_start).take(width.max(1)) {
        let is_cursor = !insert_mode && row == cursor_row && index == col;
        let in_selection = selection
            .map(|(start, end)| selection_contains(start, end, row, index))
            .unwrap_or(false);
        spans.push(segment_span(*ch, is_cursor, in_selection, color));
    }

    if spans.is_empty() {
        Line::from(Span::raw(" "))
    } else {
        Line::from(spans)
    }
}

fn segment_span(ch: char, is_cursor: bool, in_selection: bool, color: Color) -> Span<'static> {
    if is_cursor {
        Span::styled(
            ch.to_string(),
            Style::default()
                .fg(Color::Black)
                .bg(color)
                .add_modifier(Modifier::BOLD),
        )
    } else if in_selection {
        Span::styled(
            ch.to_string(),
            Style::default().fg(Color::White).bg(Color::DarkGray),
        )
    } else {
        Span::raw(ch.to_string())
    }
}

fn insert_cursor_span(color: Color) -> Span<'static> {
    Span::styled(
        "│".to_owned(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

fn render_insert_bar_cursor(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    pane: PaneId,
    total_lines: usize,
) {
    if state.input_mode != InputMode::Insert
        || state.active_pane != pane
        || area.width == 0
        || area.height == 0
    {
        return;
    }
    let cursor = state.pane_cursors.get(&pane).cloned().unwrap_or_default();
    let row_start = vertical_scroll_start(cursor.row, total_lines.max(1), area.height as usize);
    let col_start = horizontal_scroll_start(cursor.col, area.width as usize);
    if cursor.row < row_start || cursor.row >= row_start.saturating_add(area.height as usize) {
        return;
    }
    let visible_col = cursor
        .col
        .saturating_sub(col_start)
        .min(area.width.saturating_sub(1) as usize);
    let cursor_x = area.x + visible_col as u16;
    let cursor_y = area.y + (cursor.row - row_start) as u16;
    frame.set_cursor_position((cursor_x, cursor_y));
}

fn pane_color(pane: PaneId) -> Color {
    match pane {
        PaneId::SchemaPath => Color::Blue,
        PaneId::Schema => Color::Magenta,
        PaneId::Form => Color::Green,
        PaneId::OutputPath => Color::LightBlue,
        PaneId::Filter => Color::Yellow,
        PaneId::Output => Color::Cyan,
        PaneId::Log => Color::Red,
    }
}

fn vertical_scroll_start(cursor_row: usize, total_lines: usize, height: usize) -> usize {
    if total_lines <= height || height == 0 {
        0
    } else {
        cursor_row
            .saturating_sub(height.saturating_sub(1))
            .min(total_lines.saturating_sub(height))
    }
}

fn horizontal_scroll_start(cursor_col: usize, width: usize) -> usize {
    if width == 0 || cursor_col < width {
        0
    } else {
        cursor_col + 1 - width
    }
}

fn split_log_line(line: &str) -> (&str, &str) {
    line.split_once(" | ").unwrap_or(("", line))
}

fn render_log_line(
    raw_line: &str,
    row: usize,
    cursor_row: usize,
    cursor_col: usize,
    content_offset: usize,
    content_width: usize,
    gutter_width: usize,
    separator: &str,
    insert_mode: bool,
    color: Color,
    selection: Option<(SelectionAnchor, SelectionAnchor)>,
) -> Line<'static> {
    let (line_no, content) = split_log_line(raw_line);
    let number_pad = gutter_width.saturating_sub(line_no.chars().count());
    let mut spans = Vec::new();

    for pad_index in 0..number_pad {
        let raw_col = pad_index;
        spans.push(log_span(
            ' ',
            row,
            raw_col,
            cursor_row,
            cursor_col,
            selection,
            insert_mode,
            color,
            Style::default().fg(Color::DarkGray),
        ));
    }

    for (index, ch) in line_no.chars().enumerate() {
        let raw_col = number_pad + index;
        let base = if row == cursor_row {
            Style::default().fg(color).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(log_span(
            ch,
            row,
            raw_col,
            cursor_row,
            cursor_col,
            selection,
            insert_mode,
            color,
            base,
        ));
    }

    for (index, ch) in separator.chars().enumerate() {
        let raw_col = gutter_width + index;
        spans.push(log_span(
            ch,
            row,
            raw_col,
            cursor_row,
            cursor_col,
            selection,
            insert_mode,
            color,
            Style::default().fg(Color::DarkGray),
        ));
    }

    if content_width == 0 {
        return Line::from(spans);
    }

    let prefix_len = line_no.chars().count() + 3;
    let content_spans: Vec<_> = content
        .chars()
        .enumerate()
        .skip(content_offset)
        .take(content_width)
        .map(|(index, ch)| {
            let raw_col = prefix_len + index;
            log_span(
                ch,
                row,
                raw_col,
                cursor_row,
                cursor_col,
                selection,
                insert_mode,
                color,
                Style::default(),
            )
        })
        .collect();

    if content_spans.is_empty() {
        spans.push(log_span(
            ' ',
            row,
            prefix_len,
            cursor_row,
            cursor_col,
            selection,
            insert_mode,
            color,
            Style::default(),
        ));
    } else {
        spans.extend(content_spans);
    }

    Line::from(spans)
}

fn render_numbered_text_line(
    line: &str,
    row: usize,
    cursor_row: usize,
    cursor_col: usize,
    content_offset: usize,
    content_width: usize,
    gutter_width: usize,
    separator: &str,
    insert_mode: bool,
    color: Color,
    selection: Option<(SelectionAnchor, SelectionAnchor)>,
) -> Line<'static> {
    let line_no = (row + 1).to_string();
    let number_pad = gutter_width.saturating_sub(line_no.chars().count());
    let mut spans = Vec::new();

    for _ in 0..number_pad {
        spans.push(Span::styled(
            " ".to_owned(),
            Style::default().fg(Color::DarkGray),
        ));
    }

    for ch in line_no.chars() {
        let style = if row == cursor_row {
            Style::default().fg(color).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::styled(ch.to_string(), style));
    }

    for ch in separator.chars() {
        spans.push(Span::styled(
            ch.to_string(),
            Style::default().fg(Color::DarkGray),
        ));
    }

    if content_width == 0 {
        return Line::from(spans);
    }

    spans.extend(render_text_content_spans(
        line,
        row,
        cursor_row,
        cursor_col,
        content_offset,
        content_width,
        insert_mode,
        color,
        selection,
    ));

    Line::from(spans)
}

fn render_text_content_spans(
    line: &str,
    row: usize,
    cursor_row: usize,
    col: usize,
    col_start: usize,
    width: usize,
    insert_mode: bool,
    color: Color,
    selection: Option<(SelectionAnchor, SelectionAnchor)>,
) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }
    if line.is_empty() {
        let is_cursor = row == cursor_row && col == 0;
        return vec![Span::styled(
            if insert_mode && is_cursor { "│" } else { " " },
            if insert_mode && is_cursor {
                Style::default().fg(color).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::Black)
                    .bg(if is_cursor { color } else { Color::Reset })
            },
        )];
    }

    let mut spans = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let visible_end = col_start.saturating_add(width.max(1));

    if insert_mode && row == cursor_row && col >= col_start && col < visible_end {
        let insert_at_end = col >= chars.len();
        if !insert_at_end && col == col_start {
            spans.push(insert_cursor_span(color));
        }
    }

    for (index, ch) in chars.iter().enumerate().skip(col_start).take(width.max(1)) {
        if insert_mode && row == cursor_row && index == col && index != col_start {
            spans.push(insert_cursor_span(color));
        }
        let is_cursor = !insert_mode && row == cursor_row && index == col;
        let in_selection = selection
            .map(|(start, end)| selection_contains(start, end, row, index))
            .unwrap_or(false);
        spans.push(segment_span(*ch, is_cursor, in_selection, color));
    }

    if insert_mode
        && row == cursor_row
        && col == chars.len()
        && col >= col_start
        && col < visible_end
    {
        spans.push(insert_cursor_span(color));
    }

    if spans.is_empty() {
        vec![Span::raw(" ")]
    } else {
        spans
    }
}

fn log_span(
    ch: char,
    row: usize,
    col: usize,
    cursor_row: usize,
    cursor_col: usize,
    selection: Option<(SelectionAnchor, SelectionAnchor)>,
    insert_mode: bool,
    color: Color,
    base: Style,
) -> Span<'static> {
    let is_cursor = row == cursor_row && col == cursor_col;
    let in_selection = selection
        .map(|(start, end)| selection_contains(start, end, row, col))
        .unwrap_or(false);
    if is_cursor && insert_mode {
        Span::styled("│".to_owned(), base.fg(color).add_modifier(Modifier::BOLD))
    } else if is_cursor {
        Span::styled(
            ch.to_string(),
            base.fg(Color::Black).bg(color).add_modifier(Modifier::BOLD),
        )
    } else if in_selection {
        Span::styled(ch.to_string(), base.fg(Color::White).bg(Color::DarkGray))
    } else {
        Span::styled(ch.to_string(), base)
    }
}

fn field_height(field: &crate::domain::form::FormField) -> u16 {
    if field.enum_options.is_some() {
        return 3;
    }
    if field.multiline {
        let visible_lines = field.edit_buffer.lines().count().max(1).clamp(5, 20) as u16;
        return visible_lines + 2;
    }
    3
}

fn array_path_for_form_group(path: &[String]) -> Option<Vec<String>> {
    path.iter()
        .position(|segment| segment.parse::<usize>().is_ok())
        .map(|index| path[..index].to_vec())
}

fn array_group_title(state: &AppState, array_path: &[String]) -> String {
    let ctx = ResolveCtx {
        root: &state.schema_json,
        instance: Some(&state.output_json),
        choices: Some(&state.one_of_choices),
    };
    resolve_schema_at_path_with(array_path, ctx)
        .ok()
        .and_then(|schema| schema.get("title").and_then(serde_json::Value::as_str))
        .map(ToOwned::to_owned)
        .or_else(|| array_path.last().cloned())
        .unwrap_or_else(|| "Array".to_owned())
}

fn array_group_item_count(state: &AppState, array_path: &[String]) -> usize {
    state
        .output_json
        .pointer(&json_pointer(array_path))
        .and_then(serde_json::Value::as_array)
        .map(|items| items.len())
        .unwrap_or(0)
}

fn array_group_can_add(state: &AppState, array_path: &[String]) -> bool {
    let item_count = array_group_item_count(state, array_path);
    let ctx = ResolveCtx {
        root: &state.schema_json,
        instance: Some(&state.output_json),
        choices: Some(&state.one_of_choices),
    };
    let Ok(schema) = resolve_schema_at_path_with(array_path, ctx) else {
        return false;
    };
    let has_additional_schema = schema.get("items").is_some()
        || schema
            .get("prefixItems")
            .and_then(serde_json::Value::as_array)
            .map(|items| item_count < items.len())
            .unwrap_or(false);
    if !has_additional_schema {
        return false;
    }
    match schema
        .get("maxItems")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize)
    {
        Some(max_items) => item_count < max_items,
        None => true,
    }
}

fn array_group_can_remove(state: &AppState, array_path: &[String]) -> bool {
    let item_count = array_group_item_count(state, array_path);
    let ctx = ResolveCtx {
        root: &state.schema_json,
        instance: Some(&state.output_json),
        choices: Some(&state.one_of_choices),
    };
    let Ok(schema) = resolve_schema_at_path_with(array_path, ctx) else {
        return false;
    };
    let min_items = schema
        .get("minItems")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(0);
    item_count > min_items
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

fn build_field_textarea(field: &crate::domain::form::FormField) -> TextArea<'static> {
    let mut textarea = TextArea::from(if field.edit_buffer.is_empty() {
        vec![String::new()]
    } else {
        field.edit_buffer.lines().map(ToOwned::to_owned).collect()
    });
    textarea.set_placeholder_text(match field.schema_type {
        SchemaType::String if field.multiline => "textarea",
        SchemaType::String => "text",
        SchemaType::Number => "number",
        SchemaType::Integer => "integer",
        SchemaType::Boolean => "true / false",
        SchemaType::Null => "null",
    });
    textarea.set_tab_length(2);
    textarea
}

fn enum_field_line(field: &crate::domain::form::FormField, active: bool) -> Line<'static> {
    let mut spans = Vec::new();
    let value = if field.edit_buffer.is_empty() {
        "select…".to_owned()
    } else {
        field.edit_buffer.clone()
    };
    spans.push(Span::styled(
        value,
        Style::default().fg(Color::White).add_modifier(if active {
            Modifier::BOLD
        } else {
            Modifier::empty()
        }),
    ));
    spans.push(Span::raw(" "));
    spans.push(Span::styled("▾", Style::default().fg(Color::DarkGray)));
    if let Some(options) = &field.enum_options {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("({})", options.join(" / ")),
            Style::default().fg(Color::DarkGray),
        ));
    }
    Line::from(spans)
}

fn array_button_span(label: &str, active: bool) -> Span<'static> {
    let style = if active {
        Style::default()
            .fg(Color::Black)
            .bg(pane_color(PaneId::Form))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    };
    Span::styled(format!(" {label} "), style)
}

fn footer_status_spans(state: &AppState) -> Vec<Span<'static>> {
    let app_mode = match state.app_mode {
        crate::app::state::AppMode::Standard => "STANDARD",
        crate::app::state::AppMode::Editor => "EDITOR",
    };
    let mode = match state.input_mode {
        InputMode::Normal => ("NORMAL", Color::White),
        InputMode::Insert => ("INSERT", pane_color(state.active_pane)),
        InputMode::Visual => ("VISUAL", pane_color(state.active_pane)),
    };
    let validation = if state.validation.is_valid {
        ("OK".to_owned(), Color::Green)
    } else {
        (state.validation.status_line(), Color::Red)
    };
    let schema = fit_text(&state.schema_path.schema_source, 18);
    let filter = fit_text(&state.filter_text, 14);
    let fields = state.form_fields.len().to_string();

    vec![
        footer_label(" APP "),
        footer_badge(app_mode, Color::Cyan),
        footer_sep(),
        footer_label(" MODE "),
        footer_badge(mode.0, mode.1),
        footer_sep(),
        footer_label(" FOCUS "),
        footer_value(
            &state.pane_title(state.active_pane),
            pane_color(state.active_pane),
        ),
        footer_sep(),
        footer_label(" VALID "),
        footer_value(&validation.0, validation.1),
        footer_sep(),
        footer_label(" FIELDS "),
        footer_value(&fields, Color::Cyan),
        footer_sep(),
        footer_label(" SCHEMA "),
        footer_value(&schema, Color::Magenta),
        footer_sep(),
        footer_label(" FILTER "),
        footer_value(&filter, Color::Yellow),
    ]
}

fn footer_status_line(state: &AppState, width: usize) -> Line<'static> {
    truncate_spans(footer_status_spans(state), width)
}

/// Second footer row: contextual shortcuts with styled key caps.
fn footer_keys_line(state: &AppState, width: usize) -> Line<'static> {
    use crate::app::state::AppMode;
    use PaneId::*;

    let mut spans: Vec<Span<'static>> = vec![
        footer_label(" KEYS "),
        key_cap_span("?"),
        Span::styled(" help", Style::default().fg(Color::DarkGray)),
        footer_sep_dot(),
    ];

    match state.input_mode {
        InputMode::Insert => {
            spans.push(key_cap_span("Esc"));
            spans.push(Span::styled(
                " commit",
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(footer_sep_dot());
            spans.push(key_cap_span("^u"));
            spans.push(Span::styled(
                " undo line",
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(footer_sep_dot());
            spans.push(key_cap_span("^r"));
            spans.push(Span::styled(
                " redo",
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(footer_sep_dot());
            match state.active_pane {
                Form => {
                    spans.push(key_cap_span("Tab"));
                    spans.push(Span::styled(
                        " next field",
                        Style::default().fg(Color::DarkGray),
                    ));
                    spans.push(footer_sep_dot());
                    spans.push(key_cap_span("S-Tab"));
                    spans.push(Span::styled(
                        " prev field",
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                SchemaPath => {
                    spans.push(key_cap_span("Tab"));
                    spans.push(Span::styled(
                        " next candidate",
                        Style::default().fg(Color::DarkGray),
                    ));
                    spans.push(footer_sep_dot());
                    spans.push(key_cap_span("S-Tab"));
                    spans.push(Span::styled(
                        " prev candidate",
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                Schema => {
                    spans.push(key_cap_span("Tab"));
                    spans.push(Span::styled(
                        " indent +4",
                        Style::default().fg(Color::DarkGray),
                    ));
                    spans.push(footer_sep_dot());
                    spans.push(key_cap_span("S-Tab"));
                    spans.push(Span::styled(
                        " outdent",
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                _ => {
                    spans.push(key_cap_span("arrows"));
                    spans.push(Span::styled(
                        " move",
                        Style::default().fg(Color::DarkGray),
                    ));
                }
            }
        }
        InputMode::Visual => {
            spans.push(key_cap_span("y"));
            spans.push(Span::styled(
                " yank",
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(footer_sep_dot());
            spans.push(key_cap_span("v Esc"));
            spans.push(Span::styled(
                " leave",
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(footer_sep_dot());
            spans.push(key_cap_span("hjkl"));
            spans.push(Span::styled(
                " extend",
                Style::default().fg(Color::DarkGray),
            ));
        }
        InputMode::Normal => {
            match state.app_mode {
                AppMode::Editor => {
                    spans.push(key_cap_span("1–7"));
                    spans.push(Span::styled(
                        " panes",
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                AppMode::Standard => {
                    spans.push(key_cap_span("1–6"));
                    spans.push(Span::styled(
                        " panes",
                        Style::default().fg(Color::DarkGray),
                    ));
                }
            }
            spans.push(footer_sep_dot());
            spans.push(key_cap_span("Tab"));
            spans.push(Span::styled(
                " next pane",
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(footer_sep_dot());
            spans.push(key_cap_span("hjkl"));
            spans.push(Span::styled(
                " move",
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(footer_sep_dot());
            match state.active_pane {
                Form => {
                    spans.push(key_cap_span("Enter"));
                    spans.push(Span::styled(
                        " edit field",
                        Style::default().fg(Color::DarkGray),
                    ));
                    spans.push(footer_sep_dot());
                    spans.push(key_cap_span("hl"));
                    spans.push(Span::styled(
                        " enum",
                        Style::default().fg(Color::DarkGray),
                    ));
                    spans.push(footer_sep_dot());
                    spans.push(key_cap_span("+-"));
                    spans.push(Span::styled(
                        " array",
                        Style::default().fg(Color::DarkGray),
                    ));
                    spans.push(footer_sep_dot());
                    spans.push(key_cap_span("R"));
                    spans.push(Span::styled(
                        " reset form",
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                SchemaPath | Schema | OutputPath | Filter | Output | Log => {
                    spans.push(key_cap_span("i a"));
                    spans.push(Span::styled(
                        " insert",
                        Style::default().fg(Color::DarkGray),
                    ));
                }
            }
            spans.push(footer_sep_dot());
            spans.push(key_cap_span("z"));
            spans.push(Span::styled(
                "+a/w fold/full",
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(footer_sep_dot());
            spans.push(key_cap_span("u"));
            spans.push(Span::styled(
                " undo",
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(footer_sep_dot());
            spans.push(key_cap_span("r"));
            spans.push(Span::styled(
                " save JSON",
                Style::default().fg(Color::DarkGray),
            ));
        }
    }

    truncate_spans(spans, width)
}

/// Single-row footer when vertical space is only one line.
fn footer_line_single_row(state: &AppState, width: usize) -> Line<'static> {
    let mut spans = footer_status_spans(state);
    spans.push(footer_sep());
    spans.push(footer_label(" TIP "));
    spans.push(Span::styled(
        "? help · Tab panes · r save",
        Style::default().fg(Color::White),
    ));
    truncate_spans(spans, width)
}

fn centered_rect(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let [_, vertical, _] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height_pct) / 2),
            Constraint::Percentage(height_pct),
            Constraint::Percentage((100 - height_pct) / 2),
        ])
        .areas(area);
    let [_, horizontal, _] = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_pct) / 2),
            Constraint::Percentage(width_pct),
            Constraint::Percentage((100 - width_pct) / 2),
        ])
        .areas(vertical);
    horizontal
}

fn schema_error_line(error: &crate::app::state::SchemaError) -> String {
    match (error.line, error.column) {
        (Some(line), Some(column)) => format!("ERR L{line}:C{column} | {}", error.message),
        (Some(line), None) => format!("ERR L{line} | {}", error.message),
        _ => format!("ERR | {}", error.message),
    }
}

fn schema_candidates_footer_line(state: &AppState) -> Option<String> {
    let candidates = state.schema_candidates();
    if candidates.is_empty() {
        return None;
    }
    let index = candidates
        .iter()
        .position(|candidate| candidate.starts_with("> "))
        .map(|idx| idx + 1)
        .unwrap_or(1);
    let current = candidates
        .iter()
        .find(|candidate| candidate.starts_with("> "))
        .or_else(|| candidates.first())?;
    Some(format!(
        "CAND {index}/{} | {}",
        candidates.len(),
        current.trim_start_matches("> ").trim_start()
    ))
}

fn footer_label(text: &str) -> Span<'static> {
    Span::styled(
        text.to_owned(),
        Style::default()
            .fg(Color::Black)
            .bg(Color::White)
            .add_modifier(Modifier::BOLD),
    )
}

fn footer_badge(text: &str, color: Color) -> Span<'static> {
    Span::styled(
        format!(" {text} "),
        Style::default()
            .fg(Color::Black)
            .bg(color)
            .add_modifier(Modifier::BOLD),
    )
}

fn footer_value(text: &str, color: Color) -> Span<'static> {
    Span::styled(text.to_owned(), Style::default().fg(color))
}

fn footer_sep() -> Span<'static> {
    Span::styled("  ".to_owned(), Style::default().fg(Color::DarkGray))
}

fn truncate_spans(spans: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let mut remaining = width;
    let mut rendered = Vec::new();

    for span in spans {
        if remaining == 0 {
            break;
        }
        let content = span.content.to_string();
        let span_width = UnicodeWidthStr::width(content.as_str());
        if span_width <= remaining {
            remaining -= span_width;
            rendered.push(span);
            continue;
        }

        let truncated = fit_text(&content, remaining);
        if !truncated.is_empty() {
            rendered.push(Span::styled(truncated, span.style));
        }
        break;
    }

    Line::from(rendered)
}

fn selection_for_pane(
    state: &AppState,
    pane: PaneId,
) -> Option<(SelectionAnchor, SelectionAnchor)> {
    if state.input_mode != InputMode::Visual || state.active_pane != pane {
        return None;
    }
    let anchor = state.visual_anchor?;
    if anchor.pane != pane {
        return None;
    }
    let current = SelectionAnchor {
        pane,
        row: state
            .pane_cursors
            .get(&pane)
            .map(|cursor| cursor.row)
            .unwrap_or(0),
        col: state
            .pane_cursors
            .get(&pane)
            .map(|cursor| cursor.col)
            .unwrap_or(0),
    };
    Some(if (anchor.row, anchor.col) <= (current.row, current.col) {
        (anchor, current)
    } else {
        (current, anchor)
    })
}

fn selection_contains(
    start: SelectionAnchor,
    end: SelectionAnchor,
    row: usize,
    col: usize,
) -> bool {
    (row > start.row || (row == start.row && col >= start.col))
        && (row < end.row || (row == end.row && col <= end.col))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;

    use crate::app::state::{AppMode, AppState, PaneId};

    use super::{
        form_part_heights, handle_scroll, horizontal_scroll_start, pane_layout, partition_form_parts,
        render, split_log_line, vertical_scroll_start, FormPart, FORM_MOUSE_SCROLL_TICKS_PER_ROW,
    };

    #[test]
    fn scrolls_vertically_to_keep_cursor_visible() {
        assert_eq!(vertical_scroll_start(0, 20, 5), 0);
        assert_eq!(vertical_scroll_start(4, 20, 5), 0);
        assert_eq!(vertical_scroll_start(5, 20, 5), 1);
        assert_eq!(vertical_scroll_start(19, 20, 5), 15);
    }

    #[test]
    fn scrolls_horizontally_to_keep_cursor_visible() {
        assert_eq!(horizontal_scroll_start(0, 8), 0);
        assert_eq!(horizontal_scroll_start(7, 8), 0);
        assert_eq!(horizontal_scroll_start(8, 8), 1);
        assert_eq!(horizontal_scroll_start(12, 8), 5);
    }

    #[test]
    fn splits_log_line_into_number_and_message() {
        assert_eq!(
            split_log_line("0007 | schema rebuilt"),
            ("0007", "schema rebuilt")
        );
        assert_eq!(split_log_line("plain text"), ("", "plain text"));
    }

    #[test]
    fn material_form_partition_keeps_root_object_frame_with_full_indices() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/material.json");
        let mut state = AppState::new_with_mode(AppMode::Standard);
        state
            .set_schema_source(path.to_string_lossy().to_string())
            .expect("load material schema");
        assert!(
            !state.form_fields.is_empty(),
            "expected form fields from material.json"
        );
        let n = state.form_fields.len();
        let indices: Vec<usize> = (0..n).collect();
        let parts = partition_form_parts(&state.form_fields, &indices, None, None);
        assert!(
            parts.iter().any(|p| matches!(p, FormPart::ObjectFrame(_, _))),
            "full index run should include at least one ObjectFrame (root nesting)"
        );
    }

    #[test]
    fn form_part_heights_respects_skip_field_index() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/material.json");
        let mut state = AppState::new_with_mode(AppMode::Standard);
        state
            .set_schema_source(path.to_string_lossy().to_string())
            .expect("load material schema");
        let n = state.form_fields.len();
        assert!(n > 2, "need several fields for skip test");
        let indices: Vec<usize> = (0..n).collect();
        let parts = partition_form_parts(&state.form_fields, &indices, None, None);
        let h0 = form_part_heights(&parts, &state.form_fields, &state, None, 0);
        let h_skip = form_part_heights(&parts, &state.form_fields, &state, None, 2);
        assert!(
            h_skip < h0,
            "skipping leading fields should reduce measured form height"
        );
    }

    #[test]
    fn render_does_not_panic_with_material_schema() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/material.json");
        let mut state = AppState::new_with_mode(AppMode::Standard);
        state
            .set_schema_source(path.to_string_lossy().to_string())
            .expect("load material schema");
        assert!(
            !state.form_fields.is_empty(),
            "expected form fields from material.json"
        );
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|f| render(f, &state))
            .expect("render one frame");
    }

    #[test]
    fn mouse_wheel_scrolls_schema_and_log_panes() {
        let mut state = AppState::new_with_mode(crate::app::state::AppMode::Editor);
        state.schema_text = "{\n  \"a\": 1,\n  \"b\": 2,\n  \"c\": 3\n}".to_owned();
        state.logs = vec![
            "0001 | one".to_owned(),
            "0002 | two".to_owned(),
            "0003 | three".to_owned(),
        ];
        let area = Rect::new(0, 0, 120, 40);
        let panes = pane_layout(&state, area);
        let schema_x = panes.schema.x + 1;
        let schema_y = panes.schema.y + 1;

        handle_scroll(&mut state, panes, schema_x, schema_y, 1);
        assert_eq!(state.active_pane, PaneId::Schema);
        assert_eq!(state.pane_cursors.get(&PaneId::Schema).unwrap().row, 1);

        let panes = pane_layout(&state, area);
        let log_x = panes.log.x + 1;
        let log_y = panes.log.y + 1;
        handle_scroll(&mut state, panes, log_x, log_y, 1);
        assert_eq!(state.active_pane, PaneId::Log);
        assert_eq!(state.pane_cursors.get(&PaneId::Log).unwrap().row, 1);
    }

    #[test]
    fn mouse_wheel_scrolls_output_pane() {
        let mut state = AppState::new_with_mode(AppMode::Editor);
        state.filter_outcome.text = "{\n  \"a\": 1,\n  \"b\": 2,\n  \"c\": 3\n}".to_owned();
        let area = Rect::new(0, 0, 120, 40);
        let panes = pane_layout(&state, area);
        let ox = panes.output.x + 1;
        let oy = panes.output.y + 1;
        handle_scroll(&mut state, panes, ox, oy, 1);
        assert_eq!(state.active_pane, PaneId::Output);
        assert_eq!(state.pane_cursors.get(&PaneId::Output).unwrap().row, 1);
    }

    #[test]
    fn form_mouse_scroll_moves_one_row_every_n_ticks() {
        let mut state = AppState::new_with_mode(AppMode::Standard);
        assert!(
            state.form_fields.len() > 1,
            "expected multiple form fields from default schema"
        );
        let area = Rect::new(0, 0, 120, 40);
        let initial = pane_layout(&state, area);
        let fx = initial.form.x + 2;
        let fy = initial.form.y + 3;
        for _ in 0..(FORM_MOUSE_SCROLL_TICKS_PER_ROW as usize).saturating_sub(1) {
            let panes = pane_layout(&state, area);
            handle_scroll(&mut state, panes, fx, fy, 1);
            assert_eq!(state.pane_cursors.get(&PaneId::Form).unwrap().row, 0);
        }
        let panes = pane_layout(&state, area);
        handle_scroll(&mut state, panes, fx, fy, 1);
        assert_eq!(state.pane_cursors.get(&PaneId::Form).unwrap().row, 1);
    }
}
