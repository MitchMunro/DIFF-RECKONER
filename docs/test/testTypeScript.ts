// A tiny task store, for checking how an untracked file renders.

export type Priority = "low" | "medium" | "high" | "urgent";

export interface Task {
  readonly id: number;
  title: string;
  priority: Priority;
  done: boolean;
  dueAt?: Date;
}

const RANK: Record<Priority, number> = { low: 0, medium: 1, high: 2, urgent: 3 };

export class TaskStore {
  #nextId = 0;
  #tasks = new Map<number, Task>();

  add(title: string, priority: Priority = "medium", dueAt?: Date): Task {
    const trimmed = title.trim();
    if (trimmed.length === 0) {
      throw new Error("a task needs a title");
    }
    const task: Task = { id: ++this.#nextId, title: trimmed, priority, done: false, dueAt };
    this.#tasks.set(task.id, task);
    return task;
  }

  toggle(id: number): boolean | undefined {
    const task = this.#tasks.get(id);
    if (!task) return undefined;
    task.done = !task.done;
    return task.done;
  }

  overdue(now = new Date()): Task[] {
    return [...this.#tasks.values()].filter((t) => !t.done && t.dueAt !== undefined && t.dueAt < now);
  }

  /** Open tasks first, then highest priority. */
  sorted(): Task[] {
    return [...this.#tasks.values()].sort(
      (a, b) => Number(a.done) - Number(b.done) || RANK[b.priority] - RANK[a.priority],
    );
  }
}
