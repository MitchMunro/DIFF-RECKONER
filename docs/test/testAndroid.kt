package com.example.tasks.ui

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.viewModels
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.*
import kotlinx.coroutines.launch
import java.util.UUID

// [- REVIEW -] [SPAN: 6 lines] fasdfaf
/** How urgent a task is. Ordinals persist to disk, so append only. */
enum class Priority(val label: String, val tint: Color) {
    LOW("Low", Color(0xFF2E7D32)),
    MEDIUM("Medium", Color(0xFFEF6C00)),
    HIGH("High", Color(0xFFC62828)),
}

// [- REVIEW -] [SPAN: 8 lines] comment
data class Task(
    val id: String = UUID.randomUUID().toString(),
    val title: String,
    val notes: String? = null,
    val priority: Priority = Priority.MEDIUM,
    val dueAtMillis: Long? = null,
    val isDone: Boolean = false,
) {
    val isOverdue: Boolean
        get() = !isDone && dueAtMillis?.let { it < System.currentTimeMillis() } == true
}

sealed interface TasksUiState {
    data object Loading : TasksUiState
    data class Ready(val tasks: List<Task>, val filter: Priority?) : TasksUiState {
        val overdueCount: Int get() = tasks.count { it.isOverdue }
    }
    data class Error(val message: String) : TasksUiState
}

interface TaskRepository {
    fun observe(): Flow<List<Task>>
    suspend fun upsert(task: Task)
    suspend fun delete(id: String)
}

class InMemoryTaskRepository : TaskRepository {
    private val tasks = MutableStateFlow<Map<String, Task>>(emptyMap())

    override fun observe(): Flow<List<Task>> = tasks.map { it.values.toList() }

    override suspend fun upsert(task: Task) {
        delay(50) // Pretend this is a database.
        tasks.update { it + (task.id to task) }
    }

    override suspend fun delete(id: String) {
        delay(50)
        tasks.update { it - id }
    }
}

// [- REVIEW -] [SPAN: 41 lines] multi line comment here
class TasksViewModel(
    private val repository: TaskRepository = InMemoryTaskRepository(),
) : ViewModel() {
    private val filter = MutableStateFlow<Priority?>(null)

    val uiState: StateFlow<TasksUiState> =
        combine(repository.observe(), filter) { tasks, filter ->
            val visible = tasks
                .filter { filter == null || it.priority == filter }
                .sortedWith(
                    compareBy<Task> { it.isDone }
                        .thenByDescending { it.priority.ordinal }
                        .thenBy { it.dueAtMillis ?: Long.MAX_VALUE },
                )
            TasksUiState.Ready(visible, filter) as TasksUiState
        }
            .catch { e -> emit(TasksUiState.Error(e.message ?: "Unknown error")) }
            .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), TasksUiState.Loading)

    fun add(title: String, priority: Priority) {
        val trimmed = title.trim()
        require(trimmed.isNotEmpty()) { "A task needs a title." }
        viewModelScope.launch { repository.upsert(Task(title = trimmed, priority = priority)) }
    }

    fun toggle(task: Task) = viewModelScope.launch {
        repository.upsert(task.copy(isDone = !task.isDone))
    }

    fun rename(task: Task, title: String) {
        val trimmed = title.trim()
        require(trimmed.isNotEmpty()) { "A task needs a title." }
        viewModelScope.launch { repository.upsert(task.copy(title = trimmed)) }
    }

    fun delete(task: Task) = viewModelScope.launch { repository.delete(task.id) }

    fun setFilter(priority: Priority?) {
        filter.value = priority
    }
}

// [- REVIEW -] a's;dkfja;sdf
class TaskActivity : ComponentActivity() {
    private val viewModel: TasksViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MaterialTheme {
                val state by viewModel.uiState.collectAsStateWithLifecycle()
                TaskScreen(
                    state = state,
                    onAdd = viewModel::add,
                    onToggle = { viewModel.toggle(it) },
                    onDelete = { viewModel.delete(it) },
                    onFilter = viewModel::setFilter,
                )
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
// [- REVIEW -] Multi line comment here
fun TaskScreen(
    state: TasksUiState,
    onAdd: (String, Priority) -> Unit,
    onToggle: (Task) -> Unit,
    onDelete: (Task) -> Unit,
    onFilter: (Priority?) -> Unit,
) {
    var title by remember { mutableStateOf("") }
    var priority by remember { mutableStateOf(Priority.MEDIUM) }
    var error by remember { mutableStateOf<String?>(null) }

    Scaffold(topBar = { TopAppBar(title = { Text("Tasks") }) }) { padding ->
        Column(Modifier.padding(padding).padding(16.dp)) {
            OutlinedTextField(
                value = title,
                onValueChange = {
                    title = it
                    error = null
                },
                label = { Text("What needs doing?") },
                isError = error != null,
                supportingText = error?.let { { Text(it) } },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Priority.entries.forEach { p ->
                    FilterChip(
                        selected = p == priority,
                        onClick = { priority = p },
                        label = { Text(p.label) },
                    )
                }
                Spacer(Modifier.weight(1f))
                Button(onClick = {
                    try {
                        onAdd(title, priority)
                        title = ""
                        error = null
                    } catch (e: IllegalArgumentException) {
                        error = e.message
                    }
                }) { Text("Add") }
            }

            when (state) {
                is TasksUiState.Loading -> CircularProgressIndicator(Modifier.align(Alignment.CenterHorizontally))
                is TasksUiState.Error -> Text(state.message, color = MaterialTheme.colorScheme.error)
                is TasksUiState.Ready -> {
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                        Text("${state.tasks.size} tasks", style = MaterialTheme.typography.labelLarge)
                        if (state.overdueCount > 0) {
                            Text(
                                "${state.overdueCount} overdue",
                                style = MaterialTheme.typography.labelLarge,
                                color = MaterialTheme.colorScheme.error,
                            )
                        }
                    }
                    LazyColumn(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        items(state.tasks, key = { it.id }) { task ->
                            TaskRow(task, onToggle = { onToggle(task) }, onDelete = { onDelete(task) })
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun TaskRow(task: Task, onToggle: () -> Unit, onDelete: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Checkbox(checked = task.isDone, onCheckedChange = { onToggle() })
        Column(Modifier.weight(1f)) {
            Text(
                text = task.title,
                color = if (task.isOverdue) MaterialTheme.colorScheme.error else task.priority.tint,
                textDecoration = if (task.isDone) TextDecoration.LineThrough else null,
            )
            task.notes?.takeIf { it.isNotBlank() }?.let { notes ->
                Text(
                    text = notes,
                    style = MaterialTheme.typography.bodySmall,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        IconButton(onClick = onDelete) {
            Icon(Icons.Default.Delete, contentDescription = "Delete ${task.title}")
        }
    }
}
