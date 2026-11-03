//! The egui front end. The UI only ever *reads* the board while drawing; every change is queued
//! as an [`Action`] and applied after the frame, which keeps borrowing simple and gives a single
//! place that marks the board as needing a save.

use crate::links::{Segment, segments};
use crate::model::{Board, ColumnId, TaskId};
use crate::storage::Storage;
use eframe::egui::{
    self, Align, Color32, ComboBox, CursorIcon, DragAndDrop, Frame, Hyperlink, Id, Key, Label,
    LayerId, Layout, Margin, Order, RichText, ScrollArea, Sense, Stroke, TextEdit, Ui, UiBuilder,
    emath::TSTransform, vec2,
};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const COLUMN_WIDTH: f32 = 300.0;
const SAVE_DELAY: Duration = Duration::from_millis(400);
const BLOCKED_COLOR: Color32 = Color32::from_rgb(224, 96, 96);

/// Drag-and-drop payload: the column being dragged.
#[derive(Clone, Copy)]
struct DragColumn(ColumnId);

/// Drag-and-drop payload: the top-level card being dragged.
#[derive(Clone, Copy)]
struct DragCard(TaskId);

enum Action {
    AddColumn(String),
    RenameColumn(ColumnId, String),
    RemoveColumn(ColumnId),
    MoveColumn(ColumnId, usize),
    AddTask(ColumnId, String),
    AddHeader(ColumnId, String),
    AddSubtask(TaskId, String),
    SetTitle(TaskId, String),
    SetDescription(TaskId, String),
    SetDone(TaskId, bool),
    RemoveTask(TaskId),
    MoveTask(TaskId, ColumnId, usize),
    MoveChild(TaskId, isize),
    SetParent(TaskId, Option<TaskId>),
    AddDependency(TaskId, TaskId),
    RemoveDependency(TaskId, TaskId),
    Edit(Option<TaskId>),
}

pub struct KanbanApp {
    board: Board,
    storage: Option<Storage>,
    /// Changes not yet written to disk.
    unsaved: bool,
    save_due: Option<Instant>,
    status: Option<String>,
    editing: Option<TaskId>,
    confirm_delete: bool,
    new_column: String,
    card_inputs: HashMap<ColumnId, String>,
    subtask_input: String,
}

impl KanbanApp {
    pub fn new() -> Self {
        let mut storage = Storage::from_xdg();
        let (board, status) = match &mut storage {
            Some(s) => s.load(),
            None => (
                Board::default(),
                Some("No home directory found: changes will not be saved.".to_owned()),
            ),
        };
        KanbanApp {
            board,
            storage,
            unsaved: false,
            save_due: None,
            status,
            editing: None,
            confirm_delete: false,
            new_column: String::new(),
            card_inputs: HashMap::new(),
            subtask_input: String::new(),
        }
    }

    fn apply(&mut self, action: Action) {
        let b = &mut self.board;
        match action {
            Action::Edit(id) => {
                self.editing = id;
                self.confirm_delete = false;
                self.subtask_input.clear();
                return; // UI state only
            }
            Action::AddColumn(name) => {
                b.add_column(&name);
            }
            Action::RenameColumn(id, name) => b.rename_column(id, &name),
            Action::RemoveColumn(id) => {
                b.remove_column(id);
            }
            Action::MoveColumn(id, index) => b.move_column_to(id, index),
            Action::AddTask(col, title) => {
                b.add_task(col, &title);
            }
            Action::AddHeader(col, title) => {
                b.add_header(col, &title);
            }
            Action::AddSubtask(parent, title) => {
                b.add_subtask(parent, &title);
            }
            Action::SetTitle(id, s) => {
                if let Some(t) = b.tasks.get_mut(&id) {
                    t.title = s;
                }
            }
            Action::SetDescription(id, s) => {
                if let Some(t) = b.tasks.get_mut(&id) {
                    t.description = s;
                }
            }
            Action::SetDone(id, done) => {
                if let Some(t) = b.tasks.get_mut(&id) {
                    t.done = done;
                }
            }
            Action::RemoveTask(id) => {
                b.remove_task(id);
                if self.editing.is_some_and(|e| !b.tasks.contains_key(&e)) {
                    self.editing = None;
                }
                self.confirm_delete = false;
            }
            Action::MoveTask(id, col, index) => {
                b.move_task(id, col, index);
            }
            Action::MoveChild(id, d) => b.move_child(id, d),
            Action::SetParent(id, p) => {
                b.set_parent(id, p);
            }
            Action::AddDependency(id, dep) => {
                b.add_dependency(id, dep);
            }
            Action::RemoveDependency(id, dep) => b.remove_dependency(id, dep),
        }
        self.unsaved = true;
        self.save_due = Some(Instant::now() + SAVE_DELAY);
    }

    fn flush(&mut self) {
        if !self.unsaved {
            return;
        }
        self.save_due = None;
        let Some(storage) = &mut self.storage else {
            return;
        };
        match storage.save(&self.board) {
            Ok(()) => {
                self.unsaved = false;
                self.status = None;
            }
            Err(e) => self.status = Some(format!("Failed to save: {e}")),
        }
    }

    fn autosave(&mut self, ctx: &egui::Context) {
        if let Some(due) = self.save_due {
            let now = Instant::now();
            if now >= due {
                self.flush();
            } else {
                ctx.request_repaint_after(due - now);
            }
        }
    }
}

impl Drop for KanbanApp {
    fn drop(&mut self) {
        self.flush();
    }
}

impl eframe::App for KanbanApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.viewport().close_requested()) {
            self.flush();
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let mut actions = Vec::new();

        egui::Panel::top("top_bar").show(ui, |ui| self.top_bar(ui, &mut actions));
        egui::CentralPanel::default().show(ui, |ui| self.board_ui(ui, &mut actions));
        self.edit_window(ui.ctx(), &mut actions);

        for action in actions {
            self.apply(action);
        }
        self.autosave(ui.ctx());
    }
}

// ------------------------------------------------------------------- top bar

impl KanbanApp {
    fn top_bar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.heading("Kanban");
            ui.separator();
            let response = ui.add(
                TextEdit::singleline(&mut self.new_column)
                    .hint_text("New column name")
                    .desired_width(180.0),
            );
            let submitted = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if (ui.button("Add column").clicked() || submitted)
                && !self.new_column.trim().is_empty()
            {
                actions.push(Action::AddColumn(self.new_column.trim().to_owned()));
                self.new_column.clear();
                response.request_focus();
            }
            if let Some(status) = &self.status {
                ui.separator();
                ui.colored_label(BLOCKED_COLOR, status);
            }
        });
    }
}

// --------------------------------------------------------------------- board

impl KanbanApp {
    fn board_ui(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let height = ui.available_height();
        ScrollArea::horizontal().show(ui, |ui| {
            let mut rects = Vec::with_capacity(self.board.columns.len());
            ui.horizontal_top(|ui| {
                for idx in 0..self.board.columns.len() {
                    let col_id = self.board.columns[idx].id;
                    let input = self.card_inputs.entry(col_id).or_default();
                    let board = &self.board;
                    let mut draw = |ui: &mut Ui| column_ui(ui, board, idx, height, input, actions);

                    // While being dragged, a column is painted on a tooltip layer that follows
                    // the pointer; its slot stays reserved so the layout doesn't jump.
                    let handle_id = column_handle_id(col_id);
                    let rect = if ui.ctx().is_being_dragged(handle_id) {
                        DragAndDrop::set_payload(ui.ctx(), DragColumn(col_id));
                        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                        let layer_id = LayerId::new(Order::Tooltip, handle_id);
                        let rect = ui
                            .scope_builder(UiBuilder::new().layer_id(layer_id), draw)
                            .inner;
                        if let Some(pointer) = ui.ctx().pointer_interact_pos() {
                            let delta = pointer - rect.center();
                            ui.ctx().transform_layer_shapes(
                                layer_id,
                                TSTransform::from_translation(delta),
                            );
                        }
                        rect
                    } else {
                        draw(ui)
                    };
                    rects.push(rect);
                }
            });

            // Dropping a column: the gap is chosen by how many column centres are left of the pointer.
            if let Some(dragged) = DragAndDrop::payload::<DragColumn>(ui.ctx()) {
                let pointer = ui.ctx().pointer_interact_pos();
                if let (Some(p), false) = (pointer, rects.is_empty()) {
                    let gap = rects.iter().filter(|r| r.center().x < p.x).count();
                    let spacing = ui.spacing().item_spacing.x;
                    let x = match gap {
                        0 => rects[0].left() - spacing / 2.0,
                        n => rects[n - 1].right() + spacing / 2.0,
                    };
                    let y = rects[0].y_range();
                    ui.painter()
                        .vline(x, y, Stroke::new(3.0, ui.visuals().selection.bg_fill));
                    if ui.input(|i| i.pointer.any_released()) {
                        actions.push(Action::MoveColumn(dragged.0, gap));
                    }
                }
            }
        });
    }
}

fn column_ui(
    ui: &mut Ui,
    board: &Board,
    idx: usize,
    height: f32,
    input: &mut String,
    actions: &mut Vec<Action>,
) -> egui::Rect {
    const MARGIN: i8 = 8;
    let col = &board.columns[idx];
    let frame = Frame::new()
        .inner_margin(Margin::same(MARGIN))
        .corner_radius(6)
        .fill(ui.visuals().faint_bg_color)
        .stroke(Stroke::new(
            1.0,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ));

    let (zone, dropped) = ui.dnd_drop_zone::<DragCard, _>(frame, |ui| {
        ui.set_width(COLUMN_WIDTH);
        ui.set_min_height(height - f32::from(2 * MARGIN) - 4.0);

        // The row of columns is a left-to-right layout that child UIs inherit; without this
        // the cards and text boxes would be laid out sideways and get squeezed.
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            // Header: editable name + column menu.
            ui.horizontal(|ui| {
                let mut name = col.name.clone();
                let edit = ui.add(
                    TextEdit::singleline(&mut name)
                        .font(egui::TextStyle::Heading)
                        .frame(Frame::NONE)
                        .desired_width(COLUMN_WIDTH - 100.0),
                );
                if edit.changed() {
                    actions.push(Action::RenameColumn(col.id, name));
                }
                let cards = col
                    .tasks
                    .iter()
                    .filter(|t| !board.tasks[*t].is_header)
                    .count();
                ui.weak(cards.to_string());
                ui.menu_button("…", |ui| {
                    let can_delete = board.columns.len() > 1;
                    let delete = ui
                        .add_enabled(can_delete, egui::Button::new("Delete column"))
                        .on_hover_text("Cards are moved to the neighbouring column")
                        .on_disabled_hover_text("The last column cannot be deleted");
                    if delete.clicked() {
                        actions.push(Action::RemoveColumn(col.id));
                        ui.close();
                    }
                });
                drag_handle(ui, column_handle_id(col.id), "Drag to move column");
            });

            // Quick add: Enter or "+" adds a card; "Header" adds a divider (empty text gives
            // it a default name that can be edited in place).
            ui.horizontal(|ui| {
                let response = ui.add(
                    TextEdit::singleline(input)
                        .hint_text("New card…")
                        .desired_width(COLUMN_WIDTH - 100.0),
                );
                let submitted = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                if (ui.button("+").clicked() || submitted) && !input.trim().is_empty() {
                    actions.push(Action::AddTask(col.id, input.trim().to_owned()));
                    input.clear();
                    response.request_focus();
                }
                if ui
                    .button("Header")
                    .on_hover_text("Add a sub-header (divider) to this column")
                    .clicked()
                {
                    let title = input.trim();
                    let title = if title.is_empty() {
                        "New header"
                    } else {
                        title
                    };
                    actions.push(Action::AddHeader(col.id, title.to_owned()));
                    input.clear();
                }
            });
            ui.separator();

            // Cards.
            let mut rects = Vec::with_capacity(col.tasks.len());
            ScrollArea::vertical()
                .id_salt(("column_scroll", col.id))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for &id in &col.tasks {
                        rects.push(card_ui(ui, board, id, actions));
                    }
                });

            // Where would a dropped card land? Count cards whose centre is above the pointer.
            let column_top = ui.max_rect().top();
            let pointer = ui.ctx().pointer_interact_pos().filter(|p| {
                DragAndDrop::has_payload_of_type::<DragCard>(ui.ctx())
                    && p.x >= ui.max_rect().left()
                    && p.x <= ui.max_rect().left() + COLUMN_WIDTH
                    && p.y >= column_top
                    && p.y <= column_top + height
            });
            let insert = pointer.map(|p| rects.iter().filter(|r| r.center().y < p.y).count());
            if let Some(i) = insert {
                let spacing = ui.spacing().item_spacing.y;
                let y = match i {
                    0 => rects
                        .first()
                        .map_or(column_top, |r| r.top() - spacing / 2.0),
                    _ => rects[i - 1].bottom() + spacing / 2.0,
                };
                let x = ui.max_rect().x_range();
                ui.painter()
                    .hline(x, y, Stroke::new(3.0, ui.visuals().selection.bg_fill));
            }
            insert
        })
        .inner
    });

    if let Some(card) = dropped {
        let index = zone.inner.unwrap_or(col.tasks.len());
        actions.push(Action::MoveTask(card.0, col.id, index));
    }
    zone.response.rect
}

// --------------------------------------------------------------------- cards

fn column_handle_id(id: ColumnId) -> Id {
    Id::unique(("column_drag_handle", id))
}

fn card_handle_id(id: TaskId) -> Id {
    Id::unique(("card_drag_handle", id))
}

/// Draws a card and returns its rect. Only the grip on the card (see [`drag_handle`]) starts a
/// drag; everything else on the card stays a normal clickable widget. While dragging, the whole
/// card is painted on a tooltip layer that follows the pointer.
fn card_ui(ui: &mut Ui, board: &Board, id: TaskId, actions: &mut Vec<Action>) -> egui::Rect {
    let task = &board.tasks[&id];
    let body = |ui: &mut Ui| {
        if task.is_header {
            header_row(ui, task, actions);
            return;
        }
        Frame::group(ui.style())
            .fill(ui.visuals().extreme_bg_color)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                task_line(ui, board, id, true, actions);
                if !task.description.is_empty() {
                    rich_text(ui, &task.description, TextLook::default());
                }
                if !task.children.is_empty() {
                    ui.add_space(2.0);
                    subtasks_ui(ui, board, id, actions);
                }
                ui.horizontal(|ui| {
                    if ui.small_button("Edit").clicked() {
                        actions.push(Action::Edit(Some(id)));
                    }
                    let (done, total) = board.progress(id);
                    if total > 0 {
                        ui.weak(format!("{done}/{total} subtasks"));
                    }
                });
            });
    };

    let handle_id = card_handle_id(id);
    if ui.ctx().is_being_dragged(handle_id) {
        DragAndDrop::set_payload(ui.ctx(), DragCard(id));
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        let layer_id = LayerId::new(Order::Tooltip, handle_id);
        let response = ui
            .scope_builder(UiBuilder::new().layer_id(layer_id), body)
            .response;
        if let Some(pointer) = ui.ctx().pointer_interact_pos() {
            let delta = pointer - response.rect.center();
            ui.ctx()
                .transform_layer_shapes(layer_id, TSTransform::from_translation(delta));
        }
        response.rect
    } else {
        ui.scope(body).response.rect
    }
}

/// A sub-header: a tinted divider row with an in-place editable title and a delete button.
fn header_row(ui: &mut Ui, task: &crate::model::Task, actions: &mut Vec<Action>) {
    let tint = ui.visuals().selection.bg_fill.gamma_multiply(0.35);
    Frame::new()
        .fill(tint)
        .corner_radius(4)
        .inner_margin(Margin::symmetric(6, 3))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                drag_handle(ui, card_handle_id(task.id), "Drag to move");
                let mut title = task.title.clone();
                let edit = ui.add(
                    TextEdit::singleline(&mut title)
                        .font(egui::TextStyle::Button)
                        .text_color(ui.visuals().strong_text_color())
                        .frame(Frame::NONE)
                        .hint_text("Header")
                        .desired_width((ui.available_width() - 30.0).max(40.0)),
                );
                if edit.changed() {
                    actions.push(Action::SetTitle(task.id, title));
                }
                if ui
                    .small_button("x")
                    .on_hover_text("Delete header (cards are kept)")
                    .clicked()
                {
                    actions.push(Action::RemoveTask(task.id));
                }
            });
        });
}

/// A small grip (two columns of dots) that is the only part of a card or column that starts a drag.
fn drag_handle(ui: &mut Ui, id: Id, tooltip: &str) {
    let size = vec2(16.0, ui.spacing().interact_size.y);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let response = ui
        .interact(rect, id, Sense::drag())
        .on_hover_cursor(CursorIcon::Grab)
        .on_hover_text(tooltip);
    let active = response.hovered() || ui.ctx().is_being_dragged(id);
    let color = if active {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    for col in 0..2 {
        for row in 0..3 {
            let offset = vec2((col as f32 - 0.5) * 5.0, (row as f32 - 1.0) * 5.0);
            ui.painter()
                .circle_filled(rect.center() + offset, 1.5, color);
        }
    }
}

/// Checkbox + title (+ "blocked by" line) for a task, used for cards and sub-tasks alike.
fn task_line(ui: &mut Ui, board: &Board, id: TaskId, is_card: bool, actions: &mut Vec<Action>) {
    let task = &board.tasks[&id];
    ui.horizontal_top(|ui| {
        if is_card {
            drag_handle(ui, card_handle_id(id), "Drag to move");
        }
        let mut done = task.done;
        if ui.checkbox(&mut done, "").changed() {
            actions.push(Action::SetDone(id, done));
        }
        // Leave room for the Edit button on sub-task rows so a long title wraps instead of
        // pushing the button (and the whole column) wider than the column width.
        let reserved = if is_card { 0.0 } else { 56.0 };
        let text_width = (ui.available_width() - reserved).max(60.0);
        ui.vertical(|ui| {
            ui.set_max_width(text_width);
            let look = TextLook {
                strong: is_card,
                strikethrough: task.done,
                weak: task.done,
            };
            rich_text(ui, &task.title, look);
            let blockers = board.blockers(id);
            if !blockers.is_empty() {
                let names: Vec<_> = blockers.iter().map(|b| board.path(*b)).collect();
                ui.label(
                    RichText::new(format!("Blocked by: {}", names.join(", ")))
                        .small()
                        .color(BLOCKED_COLOR),
                );
            }
        });
        if !is_card && ui.small_button("Edit").clicked() {
            actions.push(Action::Edit(Some(id)));
        }
    });
}

fn subtasks_ui(ui: &mut Ui, board: &Board, parent: TaskId, actions: &mut Vec<Action>) {
    ui.indent(("subtasks", parent), |ui| {
        for &child in &board.tasks[&parent].children {
            task_line(ui, board, child, false, actions);
            if !board.tasks[&child].children.is_empty() {
                subtasks_ui(ui, board, child, actions);
            }
        }
    });
}

// ----------------------------------------------------------------- rich text

#[derive(Clone, Copy, Default)]
struct TextLook {
    strong: bool,
    strikethrough: bool,
    weak: bool,
}

impl TextLook {
    fn apply(self, text: &str) -> RichText {
        let mut t = RichText::new(text);
        if self.strong {
            t = t.strong();
        }
        if self.strikethrough {
            t = t.strikethrough();
        }
        if self.weak {
            t = t.weak();
        }
        t
    }
}

/// Renders text with `http(s)://` links as clickable hyperlinks. Labels are not selectable so
/// that a press on card text always starts dragging the card.
fn rich_text(ui: &mut Ui, text: &str, look: TextLook) {
    for line in text.split('\n') {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            if line.is_empty() {
                ui.add(Label::new(" ").selectable(false));
                return;
            }
            for segment in segments(line) {
                match segment {
                    Segment::Text(t) => {
                        ui.add(Label::new(look.apply(t)).selectable(false));
                    }
                    Segment::Link(url) => {
                        ui.add(Hyperlink::from_label_and_url(look.apply(url), url));
                    }
                }
            }
        });
    }
}

// --------------------------------------------------------------- edit window

impl KanbanApp {
    fn edit_window(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some(id) = self.editing else {
            return;
        };
        if !self.board.tasks.contains_key(&id) {
            self.editing = None;
            return;
        }
        let board = &self.board;
        let mut open = true;
        egui::Window::new("Edit task")
            .id(Id::unique("edit_window"))
            .open(&mut open)
            .default_width(440.0)
            .show(ctx, |ui| {
                let task = &board.tasks[&id];

                if task.parent.is_some() {
                    ui.weak(board.path(id));
                }

                let mut title = task.title.clone();
                let edit = ui.add(
                    TextEdit::singleline(&mut title)
                        .hint_text("Title")
                        .desired_width(f32::INFINITY),
                );
                if edit.changed() {
                    actions.push(Action::SetTitle(id, title));
                }

                let mut description = task.description.clone();
                let edit = ui.add(
                    TextEdit::multiline(&mut description)
                        .hint_text("Description. Links like https://example.com are clickable on the card.")
                        .desired_rows(5)
                        .desired_width(f32::INFINITY),
                );
                if edit.changed() {
                    actions.push(Action::SetDescription(id, description));
                }

                let mut done = task.done;
                if ui.checkbox(&mut done, "Done").changed() {
                    actions.push(Action::SetDone(id, done));
                }

                ui.separator();
                parent_section(ui, board, id, actions);

                ui.separator();
                subtasks_section(ui, board, id, &mut self.subtask_input, actions);

                ui.separator();
                dependencies_section(ui, board, id, actions);

                ui.separator();
                if self.confirm_delete {
                    let n = board.descendants(id).len();
                    let extra = if n > 0 {
                        format!(" and its {n} subtask(s)")
                    } else {
                        String::new()
                    };
                    ui.horizontal(|ui| {
                        ui.colored_label(BLOCKED_COLOR, format!("Delete this task{extra}?"));
                        if ui.button("Delete").clicked() {
                            actions.push(Action::RemoveTask(id));
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_delete = false;
                        }
                    });
                } else if ui.button("Delete task…").clicked() {
                    self.confirm_delete = true;
                }
            });
        if !open {
            actions.push(Action::Edit(None));
        }
    }
}

fn parent_section(ui: &mut Ui, board: &Board, id: TaskId, actions: &mut Vec<Action>) {
    let task = &board.tasks[&id];
    ui.horizontal(|ui| {
        ui.label("Parent:");
        let selected = task
            .parent
            .map_or_else(|| "None (top-level card)".to_owned(), |p| board.path(p));
        ComboBox::from_id_salt("parent_combo")
            .selected_text(selected)
            .width(280.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(task.parent.is_none(), "None (top-level card)")
                    .clicked()
                {
                    actions.push(Action::SetParent(id, None));
                }
                for c in board.parent_candidates(id) {
                    if ui
                        .selectable_label(task.parent == Some(c), board.path(c))
                        .clicked()
                    {
                        actions.push(Action::SetParent(id, Some(c)));
                    }
                }
            });
    });
}

fn subtasks_section(
    ui: &mut Ui,
    board: &Board,
    id: TaskId,
    input: &mut String,
    actions: &mut Vec<Action>,
) {
    ui.strong("Subtasks");
    let task = &board.tasks[&id];
    let last = task.children.len().saturating_sub(1);
    for (i, &child) in task.children.iter().enumerate() {
        ui.horizontal(|ui| {
            let c = &board.tasks[&child];
            let mut done = c.done;
            if ui.checkbox(&mut done, "").changed() {
                actions.push(Action::SetDone(child, done));
            }
            ui.label(&c.title);
            if ui.small_button("Edit").clicked() {
                actions.push(Action::Edit(Some(child)));
            }
            if ui
                .add_enabled(i > 0, egui::Button::new("↑").small())
                .clicked()
            {
                actions.push(Action::MoveChild(child, -1));
            }
            if ui
                .add_enabled(i < last, egui::Button::new("↓").small())
                .clicked()
            {
                actions.push(Action::MoveChild(child, 1));
            }
            if ui.small_button("Delete").clicked() {
                actions.push(Action::RemoveTask(child));
            }
        });
    }
    ui.horizontal(|ui| {
        let response = ui.add(
            TextEdit::singleline(input)
                .hint_text("New subtask…")
                .desired_width(280.0),
        );
        let submitted = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        if (ui.button("Add").clicked() || submitted) && !input.trim().is_empty() {
            actions.push(Action::AddSubtask(id, input.trim().to_owned()));
            input.clear();
            response.request_focus();
        }
    });
}

fn dependencies_section(ui: &mut Ui, board: &Board, id: TaskId, actions: &mut Vec<Action>) {
    ui.strong("Depends on");
    let task = &board.tasks[&id];
    if task.depends_on.is_empty() {
        ui.weak("Nothing: this task is not waiting on anything.");
    }
    for &dep in &task.depends_on {
        ui.horizontal(|ui| {
            let d = &board.tasks[&dep];
            let label = if d.done {
                RichText::new(board.path(dep)).strikethrough().weak()
            } else {
                RichText::new(board.path(dep))
            };
            ui.label(label);
            if ui.small_button("Remove").clicked() {
                actions.push(Action::RemoveDependency(id, dep));
            }
        });
    }
    let candidates = board.dependency_candidates(id);
    ComboBox::from_id_salt("dependency_combo")
        .selected_text("Add dependency…")
        .width(280.0)
        .show_ui(ui, |ui| {
            if candidates.is_empty() {
                ui.weak("No tasks available");
            }
            for c in candidates {
                if ui.selectable_label(false, board.path(c)).clicked() {
                    actions.push(Action::AddDependency(id, c));
                }
            }
        });

    let dependents = board.dependents(id);
    if !dependents.is_empty() {
        ui.add_space(4.0);
        ui.strong("Blocks");
        for d in dependents {
            ui.label(board.path(d));
        }
    }
}
