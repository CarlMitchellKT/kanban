//! Pure board data model: columns, nested tasks and task dependencies.
//!
//! Layout of the data:
//! * Top-level tasks (`parent == None`) live in exactly one column's ordered `tasks` list.
//! * Sub-tasks (`parent == Some(_)`) live in their parent's ordered `children` list.
//! * `depends_on` is an arbitrary "must be done first" relation between any two tasks. It is
//!   kept acyclic by [`Board::add_dependency`].

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub type TaskId = u64;
pub type ColumnId = u64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub done: bool,
    /// A sub-header: a divider card inside a column. Headers are always top-level, can be
    /// reordered like any card, and take no part in nesting or dependencies.
    #[serde(default)]
    pub is_header: bool,
    #[serde(default)]
    pub parent: Option<TaskId>,
    #[serde(default)]
    pub children: Vec<TaskId>,
    #[serde(default)]
    pub depends_on: Vec<TaskId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Column {
    pub id: ColumnId,
    pub name: String,
    #[serde(default)]
    pub tasks: Vec<TaskId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Board {
    #[serde(default = "Board::current_version")]
    pub version: u32,
    pub next_id: u64,
    pub columns: Vec<Column>,
    pub tasks: BTreeMap<TaskId, Task>,
}

impl Default for Board {
    fn default() -> Self {
        let mut board = Board {
            version: Self::current_version(),
            next_id: 1,
            columns: Vec::new(),
            tasks: BTreeMap::new(),
        };
        for name in ["To Do", "In Progress", "Done"] {
            board.add_column(name);
        }
        board
    }
}

impl Board {
    fn current_version() -> u32 {
        1
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    // ---------------------------------------------------------------- columns

    pub fn add_column(&mut self, name: &str) -> ColumnId {
        let id = self.alloc_id();
        self.columns.push(Column {
            id,
            name: name.to_owned(),
            tasks: Vec::new(),
        });
        id
    }

    pub fn column_index(&self, id: ColumnId) -> Option<usize> {
        self.columns.iter().position(|c| c.id == id)
    }

    pub fn rename_column(&mut self, id: ColumnId, name: &str) {
        if let Some(i) = self.column_index(id) {
            self.columns[i].name = name.to_owned();
        }
    }

    /// Removes a column. Its tasks are appended to the neighbouring column (the next one, or the
    /// previous one if it was last), so no work is lost. The last remaining column cannot be
    /// removed; returns `false` in that case.
    pub fn remove_column(&mut self, id: ColumnId) -> bool {
        let Some(i) = self.column_index(id) else {
            return false;
        };
        if self.columns.len() <= 1 {
            return false;
        }
        let removed = self.columns.remove(i);
        let target = i.min(self.columns.len() - 1);
        self.columns[target].tasks.extend(removed.tasks);
        true
    }

    /// Moves a column so it lands at `index`, a gap position in the column list *as it was
    /// before the move* (what a drop position computed from the visible columns naturally is).
    /// Dropping a column next to itself is a no-op; indexes past the end are clamped.
    pub fn move_column_to(&mut self, id: ColumnId, index: usize) {
        let Some(from) = self.column_index(id) else {
            return;
        };
        let mut index = index.min(self.columns.len());
        if from < index {
            index -= 1;
        }
        let col = self.columns.remove(from);
        self.columns.insert(index, col);
    }

    // ------------------------------------------------------------------ tasks

    fn new_task(&mut self, title: &str, parent: Option<TaskId>) -> TaskId {
        let id = self.alloc_id();
        self.tasks.insert(
            id,
            Task {
                id,
                title: title.to_owned(),
                description: String::new(),
                done: false,
                is_header: false,
                parent,
                children: Vec::new(),
                depends_on: Vec::new(),
            },
        );
        id
    }

    /// Appends a new top-level task to the end of a column.
    pub fn add_task(&mut self, column: ColumnId, title: &str) -> Option<TaskId> {
        let i = self.column_index(column)?;
        let id = self.new_task(title, None);
        self.columns[i].tasks.push(id);
        Some(id)
    }

    /// Appends a new sub-header (divider) to the end of a column.
    pub fn add_header(&mut self, column: ColumnId, title: &str) -> Option<TaskId> {
        let id = self.add_task(column, title)?;
        self.tasks.get_mut(&id)?.is_header = true;
        Some(id)
    }

    fn is_header(&self, id: TaskId) -> bool {
        self.tasks.get(&id).is_some_and(|t| t.is_header)
    }

    /// Appends a new sub-task to a task (headers cannot have sub-tasks).
    pub fn add_subtask(&mut self, parent: TaskId, title: &str) -> Option<TaskId> {
        if !self.tasks.contains_key(&parent) || self.is_header(parent) {
            return None;
        }
        let id = self.new_task(title, Some(parent));
        self.tasks.get_mut(&parent)?.children.push(id);
        Some(id)
    }

    /// Index of the column holding a top-level task.
    pub fn column_of(&self, id: TaskId) -> Option<usize> {
        self.columns.iter().position(|c| c.tasks.contains(&id))
    }

    /// All transitive sub-tasks of `id` (not including `id`), depth-first in display order.
    pub fn descendants(&self, id: TaskId) -> Vec<TaskId> {
        let mut out = Vec::new();
        let mut stack: Vec<TaskId> = self
            .tasks
            .get(&id)
            .map(|t| t.children.iter().rev().copied().collect())
            .unwrap_or_default();
        while let Some(next) = stack.pop() {
            out.push(next);
            if let Some(t) = self.tasks.get(&next) {
                stack.extend(t.children.iter().rev().copied());
            }
        }
        out
    }

    /// Detaches a task from its column or parent's child list (leaves `parent` untouched).
    fn detach(&mut self, id: TaskId) {
        let parent = self.tasks.get(&id).and_then(|t| t.parent);
        match parent {
            Some(p) => {
                if let Some(p) = self.tasks.get_mut(&p) {
                    p.children.retain(|c| *c != id);
                }
            }
            None => {
                for col in &mut self.columns {
                    col.tasks.retain(|t| *t != id);
                }
            }
        }
    }

    /// Deletes a task, all of its sub-tasks, and every dependency pointing at them.
    pub fn remove_task(&mut self, id: TaskId) {
        if !self.tasks.contains_key(&id) {
            return;
        }
        self.detach(id);
        let mut doomed: BTreeSet<TaskId> = self.descendants(id).into_iter().collect();
        doomed.insert(id);
        for d in &doomed {
            self.tasks.remove(d);
        }
        for t in self.tasks.values_mut() {
            t.depends_on.retain(|d| !doomed.contains(d));
        }
    }

    /// Moves a top-level task into `column` at `index`.
    ///
    /// `index` is a position in the target column's list *as it was before the move* (which is
    /// what a drop position computed from the visible cards naturally is), so dropping a card
    /// just below itself is a no-op.
    pub fn move_task(&mut self, id: TaskId, column: ColumnId, index: usize) -> bool {
        match self.tasks.get(&id) {
            Some(t) if t.parent.is_none() => {}
            _ => return false,
        }
        let (Some(to), Some(from)) = (self.column_index(column), self.column_of(id)) else {
            return false;
        };
        let from_pos = self.columns[from]
            .tasks
            .iter()
            .position(|t| *t == id)
            .expect("column_of found it");
        let mut index = index.min(self.columns[to].tasks.len());
        if from == to && from_pos < index {
            index -= 1;
        }
        self.columns[from].tasks.remove(from_pos);
        self.columns[to].tasks.insert(index, id);
        true
    }

    /// Moves a sub-task up/down among its siblings.
    pub fn move_child(&mut self, id: TaskId, delta: isize) {
        let Some(parent) = self.tasks.get(&id).and_then(|t| t.parent) else {
            return;
        };
        let Some(p) = self.tasks.get_mut(&parent) else {
            return;
        };
        let Some(i) = p.children.iter().position(|c| *c == id) else {
            return;
        };
        let to = i.saturating_add_signed(delta).min(p.children.len() - 1);
        let c = p.children.remove(i);
        p.children.insert(to, c);
    }

    /// Re-parents a task. `None` makes it a top-level task (appended to the first column).
    /// Refuses to nest a task under itself or one of its own descendants.
    pub fn set_parent(&mut self, id: TaskId, new_parent: Option<TaskId>) -> bool {
        let Some(task) = self.tasks.get(&id) else {
            return false;
        };
        if task.parent == new_parent {
            return true;
        }
        if task.is_header {
            return false;
        }
        match new_parent {
            Some(p) => {
                if p == id
                    || self.is_header(p)
                    || !self.tasks.contains_key(&p)
                    || self.descendants(id).contains(&p)
                {
                    return false;
                }
            }
            None if self.columns.is_empty() => return false,
            None => {}
        }
        self.detach(id);
        match new_parent {
            Some(p) => self.tasks.get_mut(&p).expect("checked").children.push(id),
            None => self.columns[0].tasks.push(id),
        }
        self.tasks.get_mut(&id).expect("checked").parent = new_parent;
        true
    }

    /// Tasks that `id` could be nested under.
    pub fn parent_candidates(&self, id: TaskId) -> Vec<TaskId> {
        let excluded: BTreeSet<TaskId> = self.descendants(id).into_iter().chain([id]).collect();
        self.ordered_tasks()
            .into_iter()
            .filter(|t| !excluded.contains(t))
            .collect()
    }

    // ----------------------------------------------------------- dependencies

    /// Whether `id` depends (directly or transitively) on `target`.
    fn depends_transitively(&self, id: TaskId, target: TaskId) -> bool {
        let mut seen = BTreeSet::new();
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur) {
                continue;
            }
            if let Some(t) = self.tasks.get(&cur) {
                for d in &t.depends_on {
                    if *d == target {
                        return true;
                    }
                    stack.push(*d);
                }
            }
        }
        false
    }

    /// `id` may depend on `dep` if both exist, they differ, it isn't already a dependency, and
    /// it would not create a cycle.
    pub fn can_depend(&self, id: TaskId, dep: TaskId) -> bool {
        id != dep
            && !self.is_header(id)
            && !self.is_header(dep)
            && self.tasks.contains_key(&dep)
            && self
                .tasks
                .get(&id)
                .is_some_and(|t| !t.depends_on.contains(&dep))
            && !self.depends_transitively(dep, id)
    }

    pub fn add_dependency(&mut self, id: TaskId, dep: TaskId) -> bool {
        if !self.can_depend(id, dep) {
            return false;
        }
        self.tasks
            .get_mut(&id)
            .expect("can_depend checked")
            .depends_on
            .push(dep);
        true
    }

    pub fn remove_dependency(&mut self, id: TaskId, dep: TaskId) {
        if let Some(t) = self.tasks.get_mut(&id) {
            t.depends_on.retain(|d| *d != dep);
        }
    }

    pub fn dependency_candidates(&self, id: TaskId) -> Vec<TaskId> {
        self.ordered_tasks()
            .into_iter()
            .filter(|d| self.can_depend(id, *d))
            .collect()
    }

    /// Unfinished tasks that `id` is waiting on.
    pub fn blockers(&self, id: TaskId) -> Vec<TaskId> {
        self.tasks
            .get(&id)
            .map(|t| {
                t.depends_on
                    .iter()
                    .copied()
                    .filter(|d| self.tasks.get(d).is_some_and(|d| !d.done))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Tasks that list `id` as a dependency.
    pub fn dependents(&self, id: TaskId) -> Vec<TaskId> {
        self.tasks
            .values()
            .filter(|t| t.depends_on.contains(&id))
            .map(|t| t.id)
            .collect()
    }

    // ---------------------------------------------------------------- queries

    /// Every real task id (headers excluded) in board order: columns left to right, each card
    /// followed by its subtree.
    pub fn ordered_tasks(&self) -> Vec<TaskId> {
        let mut out = Vec::with_capacity(self.tasks.len());
        for col in &self.columns {
            for &t in col.tasks.iter().filter(|t| !self.is_header(**t)) {
                out.push(t);
                out.extend(self.descendants(t));
            }
        }
        out
    }

    /// Human readable label such as `Release / Write changelog`.
    pub fn path(&self, id: TaskId) -> String {
        let mut parts = Vec::new();
        let mut cur = Some(id);
        while let Some(c) = cur {
            match self.tasks.get(&c) {
                Some(t) => {
                    parts.push(t.title.as_str());
                    cur = t.parent;
                }
                None => break,
            }
        }
        parts.reverse();
        parts.join(" / ")
    }

    /// `(done, total)` over all transitive sub-tasks.
    pub fn progress(&self, id: TaskId) -> (usize, usize) {
        let all = self.descendants(id);
        let done = all
            .iter()
            .filter(|t| self.tasks.get(t).is_some_and(|t| t.done))
            .count();
        (done, all.len())
    }

    /// Fixes up inconsistencies in data loaded from disk (e.g. hand edited files) so the rest
    /// of the code can rely on the invariants.
    pub fn repair(&mut self) {
        let ids: BTreeSet<TaskId> = self.tasks.keys().copied().collect();
        for t in self.tasks.values_mut() {
            let me = t.id;
            t.children.retain(|c| ids.contains(c) && *c != me);
            let mut seen = BTreeSet::new();
            t.depends_on
                .retain(|d| ids.contains(d) && *d != me && seen.insert(*d));
            if t.parent.is_some_and(|p| !ids.contains(&p)) {
                t.parent = None;
            }
        }
        if self.columns.is_empty() {
            self.add_column("To Do");
        }
        let mut placed = BTreeSet::new();
        for col in &mut self.columns {
            col.tasks.retain(|t| ids.contains(t) && placed.insert(*t));
        }
        // Top-level tasks that no column mentions go to the first column.
        let orphans: Vec<TaskId> = self
            .tasks
            .values()
            .filter(|t| t.parent.is_none() && !placed.contains(&t.id))
            .map(|t| t.id)
            .collect();
        self.columns[0].tasks.extend(orphans);
        let max = self
            .tasks
            .keys()
            .copied()
            .chain(self.columns.iter().map(|c| c.id))
            .max()
            .unwrap_or(0);
        self.next_id = self.next_id.max(max + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board_with_tasks() -> (Board, ColumnId, ColumnId, [TaskId; 3]) {
        let mut b = Board::default();
        let todo = b.columns[0].id;
        let doing = b.columns[1].id;
        let a = b.add_task(todo, "a").unwrap();
        let c = b.add_task(todo, "b").unwrap();
        let d = b.add_task(todo, "c").unwrap();
        (b, todo, doing, [a, c, d])
    }

    #[test]
    fn default_board_has_three_columns() {
        let b = Board::default();
        let names: Vec<_> = b.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["To Do", "In Progress", "Done"]);
    }

    #[test]
    fn ids_are_unique_across_columns_and_tasks() {
        let (b, ..) = board_with_tasks();
        let mut ids: Vec<_> = b
            .tasks
            .keys()
            .copied()
            .chain(b.columns.iter().map(|c| c.id))
            .collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);
    }

    #[test]
    fn rename_and_add_columns() {
        let mut b = Board::default();
        let id = b.add_column("Review");
        b.rename_column(id, "QA");
        assert_eq!(b.columns.last().unwrap().name, "QA");
        assert_eq!(b.columns.len(), 4);
    }

    #[test]
    fn remove_column_moves_tasks_to_next_then_previous() {
        let (mut b, todo, doing, [a, ..]) = board_with_tasks();
        assert!(b.remove_column(todo));
        assert_eq!(b.columns[0].id, doing);
        assert!(b.columns[0].tasks.contains(&a));
        // Removing the last column sends tasks to the previous one.
        let last = b.columns.last().unwrap().id;
        let task = b.add_task(last, "z").unwrap();
        assert!(b.remove_column(last));
        assert!(b.columns.last().unwrap().tasks.contains(&task));
    }

    #[test]
    fn cannot_remove_last_column() {
        let mut b = Board::default();
        let ids: Vec<_> = b.columns.iter().map(|c| c.id).collect();
        assert!(b.remove_column(ids[0]));
        assert!(b.remove_column(ids[1]));
        assert!(!b.remove_column(ids[2]));
        assert_eq!(b.columns.len(), 1);
    }

    #[test]
    fn move_column_to_uses_pre_removal_gap_index() {
        let mut b = Board::default();
        let ids: Vec<_> = b.columns.iter().map(|c| c.id).collect();
        let order = |b: &Board| b.columns.iter().map(|c| c.id).collect::<Vec<_>>();
        // Drop the first column after the second (gap 2).
        b.move_column_to(ids[0], 2);
        assert_eq!(order(&b), [ids[1], ids[0], ids[2]]);
        // Drop the last column at the very start.
        b.move_column_to(ids[2], 0);
        assert_eq!(order(&b), [ids[2], ids[1], ids[0]]);
        // Dropping next to itself (either neighbouring gap) changes nothing.
        b.move_column_to(ids[1], 1);
        assert_eq!(order(&b), [ids[2], ids[1], ids[0]]);
        b.move_column_to(ids[1], 2);
        assert_eq!(order(&b), [ids[2], ids[1], ids[0]]);
        // Past the end is clamped; unknown ids are ignored.
        b.move_column_to(ids[2], 99);
        assert_eq!(order(&b), [ids[1], ids[0], ids[2]]);
        b.move_column_to(9999, 0);
        assert_eq!(order(&b), [ids[1], ids[0], ids[2]]);
    }

    #[test]
    fn move_task_between_columns() {
        let (mut b, todo, doing, [a, c, d]) = board_with_tasks();
        assert!(b.move_task(c, doing, 0));
        assert_eq!(b.columns[0].tasks, [a, d]);
        assert_eq!(b.columns[1].tasks, [c]);
        assert_eq!(b.column_of(c), Some(1));
        assert!(b.move_task(a, doing, 99));
        assert_eq!(b.columns[1].tasks, [c, a]);
        assert_eq!(b.columns[b.column_index(todo).unwrap()].tasks, [d]);
    }

    #[test]
    fn move_task_within_column_uses_pre_removal_index() {
        let (mut b, todo, _, [a, c, d]) = board_with_tasks();
        // Drop `a` below `c` (gap index 2 in the original list).
        b.move_task(a, todo, 2);
        assert_eq!(b.columns[0].tasks, [c, a, d]);
        // Drop `d` at the very top.
        b.move_task(d, todo, 0);
        assert_eq!(b.columns[0].tasks, [d, c, a]);
        // Dropping directly on its own slot (before or after itself) changes nothing.
        b.move_task(c, todo, 1);
        assert_eq!(b.columns[0].tasks, [d, c, a]);
        b.move_task(c, todo, 2);
        assert_eq!(b.columns[0].tasks, [d, c, a]);
        // Past the end is clamped.
        b.move_task(d, todo, 100);
        assert_eq!(b.columns[0].tasks, [c, a, d]);
    }

    #[test]
    fn move_task_rejects_subtasks_and_unknown_targets() {
        let (mut b, todo, _, [a, ..]) = board_with_tasks();
        let sub = b.add_subtask(a, "sub").unwrap();
        assert!(!b.move_task(sub, todo, 0));
        assert!(!b.move_task(a, 9999, 0));
        assert!(!b.move_task(9999, todo, 0));
    }

    #[test]
    fn nesting_to_arbitrary_depth() {
        let (mut b, _, _, [a, ..]) = board_with_tasks();
        let s1 = b.add_subtask(a, "s1").unwrap();
        let s2 = b.add_subtask(s1, "s2").unwrap();
        let s3 = b.add_subtask(s2, "s3").unwrap();
        assert_eq!(b.descendants(a), [s1, s2, s3]);
        assert_eq!(b.path(s3), "a / s1 / s2 / s3");
        assert!(b.add_subtask(9999, "x").is_none());
    }

    #[test]
    fn progress_counts_all_descendants() {
        let (mut b, _, _, [a, ..]) = board_with_tasks();
        let s1 = b.add_subtask(a, "s1").unwrap();
        let s2 = b.add_subtask(s1, "s2").unwrap();
        b.tasks.get_mut(&s2).unwrap().done = true;
        assert_eq!(b.progress(a), (1, 2));
        assert_eq!(b.progress(s2), (0, 0));
    }

    #[test]
    fn remove_task_removes_subtree_and_dangling_dependencies() {
        let (mut b, _, _, [a, c, d]) = board_with_tasks();
        let s1 = b.add_subtask(a, "s1").unwrap();
        let s2 = b.add_subtask(s1, "s2").unwrap();
        assert!(b.add_dependency(c, s2));
        assert!(b.add_dependency(d, a));
        b.remove_task(a);
        assert!(!b.tasks.contains_key(&a));
        assert!(!b.tasks.contains_key(&s1));
        assert!(!b.tasks.contains_key(&s2));
        assert!(b.tasks[&c].depends_on.is_empty());
        assert!(b.tasks[&d].depends_on.is_empty());
        assert_eq!(b.columns[0].tasks, [c, d]);
    }

    #[test]
    fn remove_subtask_detaches_from_parent() {
        let (mut b, _, _, [a, ..]) = board_with_tasks();
        let s = b.add_subtask(a, "s").unwrap();
        b.remove_task(s);
        assert!(b.tasks[&a].children.is_empty());
    }

    #[test]
    fn move_child_reorders_siblings() {
        let (mut b, _, _, [a, ..]) = board_with_tasks();
        let s1 = b.add_subtask(a, "1").unwrap();
        let s2 = b.add_subtask(a, "2").unwrap();
        let s3 = b.add_subtask(a, "3").unwrap();
        b.move_child(s3, -1);
        assert_eq!(b.tasks[&a].children, [s1, s3, s2]);
        b.move_child(s1, 5);
        assert_eq!(b.tasks[&a].children, [s3, s2, s1]);
        b.move_child(s3, -1);
        assert_eq!(b.tasks[&a].children, [s3, s2, s1]);
    }

    #[test]
    fn set_parent_nests_and_unnests() {
        let (mut b, todo, _, [a, c, d]) = board_with_tasks();
        assert!(b.set_parent(c, Some(a)));
        assert_eq!(b.tasks[&a].children, [c]);
        assert_eq!(b.tasks[&c].parent, Some(a));
        assert_eq!(b.columns[0].tasks, [a, d]);
        assert!(b.set_parent(c, None));
        assert!(b.tasks[&a].children.is_empty());
        assert_eq!(b.tasks[&c].parent, None);
        assert_eq!(b.columns[b.column_index(todo).unwrap()].tasks, [a, d, c]);
    }

    #[test]
    fn set_parent_rejects_cycles() {
        let (mut b, _, _, [a, ..]) = board_with_tasks();
        let s1 = b.add_subtask(a, "s1").unwrap();
        let s2 = b.add_subtask(s1, "s2").unwrap();
        assert!(!b.set_parent(a, Some(a)));
        assert!(!b.set_parent(a, Some(s2)));
        assert!(!b.set_parent(a, Some(9999)));
        assert!(!b.parent_candidates(a).contains(&s1));
        assert!(!b.parent_candidates(a).contains(&a));
        assert_eq!(b.descendants(a), [s1, s2]);
    }

    #[test]
    fn dependencies_reject_self_duplicates_and_cycles() {
        let (mut b, _, _, [a, c, d]) = board_with_tasks();
        assert!(!b.add_dependency(a, a));
        assert!(b.add_dependency(a, c));
        assert!(!b.add_dependency(a, c), "duplicate");
        assert!(b.add_dependency(c, d));
        assert!(!b.add_dependency(d, a), "a -> c -> d -> a would be a cycle");
        assert!(!b.add_dependency(c, a), "direct cycle");
        assert!(!b.add_dependency(a, 9999));
        assert!(!b.dependency_candidates(d).contains(&a));
        assert!(b.dependency_candidates(a).contains(&d));
    }

    #[test]
    fn blockers_and_dependents() {
        let (mut b, _, _, [a, c, d]) = board_with_tasks();
        b.add_dependency(a, c);
        b.add_dependency(a, d);
        assert_eq!(b.blockers(a), [c, d]);
        b.tasks.get_mut(&c).unwrap().done = true;
        assert_eq!(b.blockers(a), [d]);
        assert_eq!(b.dependents(c), [a]);
        b.remove_dependency(a, d);
        assert!(b.blockers(a).is_empty());
    }

    #[test]
    fn dependencies_work_on_subtasks() {
        let (mut b, _, _, [a, c, _]) = board_with_tasks();
        let s = b.add_subtask(a, "s").unwrap();
        assert!(b.add_dependency(c, s));
        assert_eq!(b.blockers(c), [s]);
    }

    #[test]
    fn ordered_tasks_follows_board_layout() {
        let (mut b, _, doing, [a, c, d]) = board_with_tasks();
        let s = b.add_subtask(a, "s").unwrap();
        b.move_task(a, doing, 0);
        assert_eq!(b.ordered_tasks(), [c, d, a, s]);
    }

    #[test]
    fn repair_fixes_dangling_references() {
        let (mut b, todo, _, [a, c, _]) = board_with_tasks();
        b.tasks.get_mut(&a).unwrap().depends_on = vec![c, c, a, 777];
        b.tasks.get_mut(&a).unwrap().children = vec![888];
        b.tasks.get_mut(&c).unwrap().parent = Some(999);
        let i = b.column_index(todo).unwrap();
        b.columns[i].tasks.push(555);
        b.columns[i].tasks.retain(|t| *t != c); // `c` is now an orphan
        b.next_id = 1;
        b.repair();
        assert_eq!(b.tasks[&a].depends_on, [c]);
        assert!(b.tasks[&a].children.is_empty());
        assert_eq!(b.tasks[&c].parent, None);
        assert!(b.columns[i].tasks.contains(&c));
        assert!(!b.columns[i].tasks.contains(&555));
        let new = b.add_task(todo, "fresh").unwrap();
        assert!(!b.tasks.values().any(|t| t.id != new && t.id == new));
        assert!(new > *b.tasks.keys().filter(|k| **k != new).max().unwrap());
    }

    #[test]
    fn headers_are_top_level_cards_that_reorder_like_any_other() {
        let (mut b, todo, doing, [a, c, _]) = board_with_tasks();
        let h = b.add_header(todo, "Waiting").unwrap();
        assert!(b.tasks[&h].is_header);
        assert_eq!(b.columns[0].tasks.last(), Some(&h));
        assert!(b.move_task(h, todo, 1));
        assert_eq!(b.columns[0].tasks[..3], [a, h, c]);
        assert!(b.move_task(h, doing, 0));
        assert_eq!(b.columns[1].tasks, [h]);
        assert!(b.add_header(9999, "x").is_none());
    }

    #[test]
    fn headers_take_no_part_in_nesting_or_dependencies() {
        let (mut b, todo, _, [a, c, _]) = board_with_tasks();
        let h = b.add_header(todo, "Waiting").unwrap();
        assert!(b.add_subtask(h, "nope").is_none());
        assert!(!b.set_parent(h, Some(a)), "header cannot become a subtask");
        assert!(
            !b.set_parent(c, Some(h)),
            "task cannot be nested in a header"
        );
        assert!(!b.add_dependency(a, h));
        assert!(!b.add_dependency(h, a));
        assert!(!b.ordered_tasks().contains(&h));
        assert!(!b.dependency_candidates(a).contains(&h));
        assert!(!b.parent_candidates(a).contains(&h));
        assert!(b.columns[0].tasks.contains(&h));
    }

    #[test]
    fn removing_a_header_leaves_other_cards_alone() {
        let (mut b, todo, _, [a, c, d]) = board_with_tasks();
        let h = b.add_header(todo, "Waiting").unwrap();
        b.remove_task(h);
        assert_eq!(b.columns[0].tasks, [a, c, d]);
        assert!(!b.tasks.contains_key(&h));
    }

    #[test]
    fn boards_saved_before_headers_existed_still_load() {
        let json = r#"{"next_id":5,"columns":[{"id":1,"name":"A","tasks":[2]}],
            "tasks":{"2":{"id":2,"title":"old"}}}"#;
        let b: Board = serde_json::from_str(json).unwrap();
        assert!(!b.tasks[&2].is_header);
    }

    #[test]
    fn json_round_trip() {
        let (mut b, _, _, [a, c, _]) = board_with_tasks();
        b.add_subtask(a, "s").unwrap();
        b.add_dependency(c, a);
        let json = serde_json::to_string(&b).unwrap();
        let back: Board = serde_json::from_str(&json).unwrap();
        assert_eq!(b, back);
    }
}
