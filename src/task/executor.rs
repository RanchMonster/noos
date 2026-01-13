use alloc::{boxed::Box, vec::Vec};
use spin::{Mutex, MutexGuard, RwLock};

use crate::task::{TaskStack, cpu_funcs::get_cpu_data, switch_to_task_context};

use super::Task;
lazy_static::lazy_static! {
    static ref GLOBAL_QUEUE: Mutex<Vec<Box<Task>>> = Mutex::new(Vec::new());
}
pub struct Executor {
    tasks: RwLock<Vec<Box<Task>>>,
    current_task: Mutex<Option<Box<Task>>>,
}
impl Executor {
    pub fn new() -> Self {
        Executor {
            tasks: RwLock::new(Vec::new()),
            current_task: Mutex::new(None),
        }
    }
    pub fn next_task(&self) -> Option<Box<Task>> {
        if let Some(task) = self.tasks.write().pop() {
            Some(task)
        } else {
            GLOBAL_QUEUE.lock().pop()
        }
    }
    pub fn current_task<'a>(&'a self) -> MutexGuard<'a, Option<Box<Task>>> {
        self.current_task.lock()
    }
    pub fn spawn(&self, func: Box<dyn FnOnce()>) {
        let task = Task::new(func, TaskStack::new());
        GLOBAL_QUEUE.lock().push(task);
    }
    pub fn bound_spawn(&self, func: Box<dyn FnOnce()>) {
        let core_id = unsafe { get_cpu_data().core_id };
        let task = Task::new_with_core(func, TaskStack::new(), core_id as usize);
        self.tasks.write().push(task);
    }
    pub fn run(&self) -> ! {
        loop {
            if let Some(task) = self.next_task() {
                // if there is a current task, switch from it to the new task
                if let Some(mut current_task) = self.current_task.lock().take() {
                    switch_to_task_context(&mut current_task.task_context, &task.task_context);
                } else {
                    // if there is no current task, switch to the new task
                    let kernel_ctx = unsafe { &mut *get_cpu_data().kernel_ctx };
                    switch_to_task_context(kernel_ctx, &task.task_context);
                }
            }
        }
    }
}
