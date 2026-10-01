//! A tiny task list, for checking how a new file renders.

use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Low,
    Medium,
    High,
    Urgent,
}

#[derive(Debug, Clone)]
pub struct Task {
    pub id: u32,
    pub title: String,
    pub priority: Priority,
    pub done: bool,
    pub tags: Vec<String>,
}

impl fmt::Display for Task {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mark = if self.done { "✓" } else { " " };
        write!(f, "[{mark}] {} ({:?})", self.title, self.priority)?;
        if !self.tags.is_empty() {
            write!(f, " #{}", self.tags.join(" #"))?;
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum TaskError {
    EmptyTitle,
    Duplicate(String),
}

#[derive(Default)]
pub struct TaskList {
    next_id: u32,
    tasks: BTreeMap<u32, Task>,
}

impl TaskList {
    pub fn add(&mut self, title: &str, priority: Priority) -> Result<u32, TaskError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(TaskError::EmptyTitle);
        }
        if self.tasks.values().any(|t| t.title == title) {
            return Err(TaskError::Duplicate(title.to_owned()));
        }
        self.next_id += 1;
        let id = self.next_id;
        self.tasks.insert(
            id,
            Task { id, title: title.to_owned(), priority, done: false, tags: Vec::new() },
        );
        Ok(id)
    }

    pub fn toggle(&mut self, id: u32) -> Option<bool> {
        let task = self.tasks.get_mut(&id)?;
        task.done = !task.done;
        Some(task.done)
    }

    pub fn tag(&mut self, id: u32, tag: &str) -> Option<()> {
        let task = self.tasks.get_mut(&id)?;
        if !task.tags.iter().any(|t| t == tag) {
            task.tags.push(tag.to_owned());
        }
        Some(())
    }

    pub fn remove(&mut self, id: u32) -> Option<Task> {
        self.tasks.remove(&id)
    }

    /// Open tasks first, then highest priority, then by title.
    pub fn sorted(&self) -> Vec<&Task> {
        let mut out: Vec<_> = self.tasks.values().collect();
        out.sort_by(|a, b| {
            a.done
                .cmp(&b.done)
                .then(b.priority.cmp(&a.priority))
                .then_with(|| a.title.cmp(&b.title))
        });
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

    #[test]
    fn duplicate_titles_are_rejected() {
        let mut list = TaskList::default();
        list.add("ship release", Priority::High).unwrap();
        assert_eq!(
            list.add("  ship release ", Priority::Low),
            Err(TaskError::Duplicate("ship release".into()))
        );
    }
}
