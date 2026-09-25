//
//  TaskListView.swift
//  Tasks
//

import SwiftUI
import Combine
import OSLog

private let logger = Logger(subsystem: "com.example.tasks", category: "TaskStore")

/// How urgent a task is. Raw values persist to disk.
enum Priority: Int, Codable, CaseIterable, Identifiable {
    case low = 0
    case medium
    case high

    var id: Int { rawValue }

    var label: String {
        switch self {
        case .low: return "Low"
        case .medium: return "Medium"
        case .high: return "High"
        }
    }

    var tint: Color {
        switch self {
        case .low: .green
        case .medium: .orange
        case .high: .red
        }
    }
}

struct TaskItem: Identifiable, Codable, Hashable {
    let id: UUID
    var title: String
    var notes: String?
    var priority: Priority
    var dueDate: Date?
    var isDone: Bool = false

    init(title: String, notes: String? = nil, priority: Priority = .medium, dueDate: Date? = nil) {
        self.id = UUID()
        self.title = title
        self.notes = notes
        self.priority = priority
        self.dueDate = dueDate
    }

    var isOverdue: Bool {
        guard let dueDate, !isDone else { return false }
        return dueDate < .now
    }
}

enum TaskStoreError: LocalizedError {
    case decodingFailed(underlying: Error)
    case emptyTitle

    var errorDescription: String? {
        switch self {
        case .decodingFailed(let error):
            return "Couldn't load tasks: \(error.localizedDescription)"
        case .emptyTitle:
            return "A task needs a title."
        }
    }
}

@MainActor
final class TaskStore: ObservableObject {
    @Published private(set) var tasks: [TaskItem] = []
    @Published var filter: Priority? = nil

    private let fileURL: URL
    private var cancellables = Set<AnyCancellable>()

    init(fileURL: URL = .documentsDirectory.appending(path: "tasks.json")) {
        self.fileURL = fileURL
        $tasks
            .debounce(for: .milliseconds(300), scheduler: RunLoop.main)
            .sink { [weak self] tasks in self?.save(tasks) }
            .store(in: &cancellables)
    }

    var visibleTasks: [TaskItem] {
        tasks
            .filter { filter == nil || $0.priority == filter }
            .sorted { lhs, rhs in
                if lhs.isDone != rhs.isDone { return !lhs.isDone }
                if lhs.priority != rhs.priority { return lhs.priority.rawValue > rhs.priority.rawValue }
                return (lhs.dueDate ?? .distantFuture) < (rhs.dueDate ?? .distantFuture)
            }
    }

    var overdueCount: Int {
        tasks.filter(\.isOverdue).count
    }

    func add(_ title: String, priority: Priority) throws {
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { throw TaskStoreError.emptyTitle }
        tasks.append(TaskItem(title: trimmed, priority: priority))
    }

    func toggle(_ task: TaskItem) {
        guard let index = tasks.firstIndex(where: { $0.id == task.id }) else { return }
        tasks[index].isDone.toggle()
    }

    func rename(_ task: TaskItem, to title: String) throws {
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { throw TaskStoreError.emptyTitle }
        guard let index = tasks.firstIndex(where: { $0.id == task.id }) else { return }
        tasks[index].title = trimmed
    }

    func delete(at offsets: IndexSet) {
        let ids = offsets.map { visibleTasks[$0].id }
        tasks.removeAll { ids.contains($0.id) }
    }

    func load() async throws {
        do {
            let (data, _) = try await URLSession.shared.data(from: fileURL)
            tasks = try JSONDecoder().decode([TaskItem].self, from: data)
        } catch let error as DecodingError {
            throw TaskStoreError.decodingFailed(underlying: error)
        } catch {
            tasks = [] // First launch: nothing saved yet.
        }
    }

    private func save(_ tasks: [TaskItem]) {
        do {
            let data = try JSONEncoder().encode(tasks)
            try data.write(to: fileURL, options: [.atomic, .completeFileProtection])
        } catch {
            logger.error("Save failed: \(error.localizedDescription, privacy: .public)")
        }
    }
}

struct TaskListView: View {
    @StateObject private var store = TaskStore()
    @State private var newTitle = ""
    @State private var newPriority: Priority = .medium
    @State private var errorMessage: String?

    var body: some View {
        NavigationStack {
            List {
                Section("New task") {
                    TextField("What needs doing?", text: $newTitle)
                        .onSubmit(addTask)
                    Picker("Priority", selection: $newPriority) {
                        ForEach(Priority.allCases) { Text($0.label).tag($0) }
                    }
                    .pickerStyle(.segmented)
                }

                Section {
                    ForEach(store.visibleTasks) { task in
                        TaskRow(task: task) { store.toggle(task) }
                            .swipeActions(edge: .leading) {
                                Button(task.isDone ? "Undo" : "Done") { store.toggle(task) }
                                    .tint(.blue)
                            }
                    }
                    .onDelete(perform: store.delete)
                } header: {
                    Text("\(store.visibleTasks.count) tasks")
                } footer: {
                    if store.overdueCount > 0 {
                        Text("\(store.overdueCount) overdue")
                            .foregroundStyle(.red)
                    }
                }
            }
            .navigationTitle("Tasks")
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Menu("Filter", systemImage: "line.3.horizontal.decrease.circle") {
                        Button("All") { store.filter = nil }
                        ForEach(Priority.allCases) { priority in
                            Button(priority.label) { store.filter = priority }
                        }
                    }
                }
            }
            .alert("Oops", isPresented: Binding(
                get: { errorMessage != nil },
                set: { if !$0 { errorMessage = nil } }
            )) {
                Button("OK", role: .cancel) {}
            } message: {
                Text(errorMessage ?? "")
            }
            .task {
                do {
                    try await store.load()
                } catch {
                    errorMessage = error.localizedDescription
                }
            }
        }
    }

    private func addTask() {
        do {
            try store.add(newTitle, priority: newPriority)
            newTitle = ""
        } catch {
            errorMessage = error.localizedDescription
        }
    }
}

private struct TaskRow: View {
    let task: TaskItem
    var onToggle: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: task.isDone ? "checkmark.circle.fill" : "circle")
                .foregroundStyle(task.priority.tint)
                .onTapGesture(perform: onToggle)
            VStack(alignment: .leading, spacing: 2) {
                Text(task.title)
                    .strikethrough(task.isDone)
                if let notes = task.notes, !notes.isEmpty {
                    Text(notes)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
                if let due = task.dueDate {
                    Text(due, style: .relative)
                        .font(.caption)
                        .foregroundStyle(task.isOverdue ? .red : .secondary)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .opacity(task.isDone ? 0.5 : 1.0)
        .accessibilityElement(children: .combine)
    }
}

#Preview {
    TaskListView()
}
