//! A tiny task list, for checking how a new file renders.

use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone)]
pub struct Task {
    pub id: u32,
    pub title: String,
    pub priority: Priority,
    pub done: bool,
}

impl fmt::Display for Task {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mark = if self.done { "x" } else { " " };
        write!(f, "[{mark}] {} ({:?})", self.title, self.priority)
    }
}

#[derive(Default)]
pub struct TaskList {
    next_id: u32,
    tasks: BTreeMap<u32, Task>,
}

impl TaskList {
    pub fn add(&mut self, title: &str, priority: Priority) -> Result<u32, &'static str> {
        let title = title.trim();
        if title.is_empty() {
            return Err("a task needs a title");
        }
        self.next_id += 1;
        let id = self.next_id;
        self.tasks.insert(
            id,
            Task { id, title: title.to_owned(), priority, done: false },
        );
        Ok(id)
    }

    pub fn toggle(&mut self, id: u32) -> Option<bool> {
        let task = self.tasks.get_mut(&id)?;
        task.done = !task.done;
        Some(task.done)
    }

    /// Open tasks first, then highest priority.
    pub fn sorted(&self) -> Vec<&Task> {
        let mut out: Vec<_> = self.tasks.values().collect();
        out.sort_by_key(|t| (t.done, std::cmp::Reverse(t.priority)));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_high_priority_sorts_first() {
        let mut list = TaskList::default();
        let low = list.add("water plants", Priority::Low).unwrap();
        list.add("ship release", Priority::High).unwrap();
        list.toggle(low);
        assert_eq!(list.sorted()[0].title, "ship release");
    }
}
