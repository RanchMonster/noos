use crate::{task::cpu_funcs::get_cpu_data, time::TimerId};
use alloc::{boxed::Box, vec};
use core::{arch::naked_asm, sync::atomic::AtomicUsize};
pub mod cpu_funcs;
pub mod executor;
const STACK_SIZE: usize = 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    Timer(TimerId),
    Interrupt,
    Waiting(TaskId),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TaskState {
    Ready,
    Running,
    Blocked(BlockReason),
    Finished,
}
pub struct TaskStack {
    pub top: *mut u8,
}
unsafe impl Send for TaskStack {}
unsafe impl Sync for TaskStack {}
impl TaskStack {
    pub fn new() -> Self {
        let stack_vec = vec![0u8; 1024 * 1024].into_boxed_slice();
        let top = Box::into_raw(stack_vec) as *mut u8;
        TaskStack { top }
    }
    pub fn top(&self) -> *mut u8 {
        self.top
    }
    pub unsafe fn from_raw(top: *mut u8) -> Self {
        TaskStack { top }
    }
    pub fn from_slice(stack: &mut [u8; 1024 * 1024]) -> Self {
        let top = stack.as_mut_ptr();
        TaskStack { top }
    }
}
impl Drop for TaskStack {
    fn drop(&mut self) {
        unsafe {
            let stack_vec = Box::from_raw(self.top as *mut [u8; 1024 * 1024]);
            drop(stack_vec);
        }
    }
}
pub type TaskId = usize;
// Task context for switching
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct TaskContext {
    // Callee-saved registers
    pub r15: usize,
    pub r14: usize,
    pub r13: usize,
    pub r12: usize,
    //Store the arg ptrs
    pub rdi: usize,
    pub rsi: usize,
    pub rdx: usize,
    pub rcx: usize,
    pub r8: usize,
    pub r9: usize,
    // Store the task state ptr
    pub rbx: usize,
    // Stack base pointer (points to the top of the stack)
    pub rbp: usize,
    // Stack pointer
    pub rsp: usize,
    // Instruction pointer (for first run)
    pub rip: usize,
}
impl TaskContext {
    /// Initialize a context for a new task
    pub fn new(
        stack: *mut u8,
        task_ptr: *mut Task,
        trampoline: extern "C" fn(*mut Task) -> (),
    ) -> Self {
        TaskContext {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            rdi: task_ptr as usize, // first argument for trampoline
            rsi: 0,
            rdx: 0,
            rcx: 0,
            r8: 0,
            r9: 0,
            rbx: 0,
            rbp: stack as usize,      // base of stack
            rsp: stack as usize,      // top of stack
            rip: trampoline as usize, // jump here first
        }
    }
    /// Initialize a empty context
    pub const fn empty() -> Self {
        TaskContext {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            rdi: 0,
            rsi: 0,
            rdx: 0,
            rcx: 0,
            r8: 0,
            r9: 0,
            rbx: 0,
            rbp: 0,
            rsp: 0,
            rip: 0,
        }
    }
}

pub struct Task {
    pub id: TaskId,
    pub stack: TaskStack,
    pub func: Option<Box<dyn FnOnce()>>,
    pub task_context: TaskContext,
    pub core_id: Option<usize>,
}
unsafe impl Send for Task {}

/// Trampoline function for starting a task
extern "C" fn trampoline(task: *mut Task) {
    let task: &mut Task = unsafe { &mut *task };
    if let Some(func) = task.func.take() {
        func();
    } else {
        panic!("Task function is None this could be a bug please report");
    }
}

static TASK_COUNTER: AtomicUsize = AtomicUsize::new(0); // I will find a better way to do this later
impl Task {
    pub fn new<F: FnOnce() + 'static>(entry: F, stack: TaskStack) -> Box<Self> {
        let mut task = Box::new(Task {
            id: TASK_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed),
            task_context: TaskContext::empty(),
            stack,
            func: Some(Box::new(entry)),
            core_id: None, // this task is not bound to a core
        });
        let task_ptr: *mut Task = &mut *task;
        task.task_context = TaskContext::new(task.stack.top(), task_ptr, trampoline);
        task
    }
    pub fn new_with_core<F: FnOnce() + 'static>(
        entry: F,
        stack: TaskStack,
        core_id: usize,
    ) -> Box<Self> {
        let mut t = Self::new(entry, stack);
        t.core_id = Some(core_id);
        t
    }
    pub fn get_state<'task>(&'task self) -> &'task TaskState {
        let state_ptr = self.task_context.rbx as *mut TaskState;
        unsafe { &*state_ptr }
    }
}
// Assembly function for context switching
#[unsafe(naked)]
pub unsafe extern "C" fn switch_to_task(from: *mut TaskContext, to: *const TaskContext) {
    naked_asm!(
        "
        // Save current context
        mov [rdi + 0x00], r15
        mov [rdi + 0x08], r14
        mov [rdi + 0x10], r13
        mov [rdi + 0x18], r12
        mov [rdi + 0x20], rbx
        mov [rdi + 0x28], rbp
        mov [rdi + 0x30], rsp
        mov [rdi + 0x38], rax
        mov [rdi + 0x40], rdi
        mov [rdi + 0x48], rsi
        mov [rdi + 0x50], rdx
        mov [rdi + 0x58], rcx
        mov [rdi + 0x60], r8
        mov [rdi + 0x68], r9

        // Load new context
        mov r15, [rsi + 0x00]
        mov r14, [rsi + 0x08]
        mov r13, [rsi + 0x10]
        mov r12, [rsi + 0x18]
        mov rdi, [rsi + 0x40]
        mov rsi, [rsi + 0x48]
        mov rdx, [rsi + 0x50]
        mov rcx, [rsi + 0x58]
        mov r8, [rsi + 0x60]
        mov r9, [rsi + 0x68]
        mov rbx, [rsi + 0x20]
        mov rbp, [rsi + 0x28]
        mov rsp, [rsi + 0x30]
        mov rax, [rsi + 0x38]  // Load rip into rax

        // Jump to new task
        jmp rax
        ",
    );
}

/// Switch to a specific task
pub fn switch_to_task_context(from: &mut TaskContext, to: &TaskContext) {
    x86_64::instructions::interrupts::without_interrupts(|| unsafe {
        switch_to_task(from as *mut TaskContext, to as *const TaskContext);
    });
}
/// Switch to the kernel context
/// This is unsafe because it assumes that the current task is not None and that the current task
/// is not the kernel task
pub unsafe fn switch_to_kernel_context() {
    x86_64::instructions::interrupts::without_interrupts(|| unsafe {
        let cpu_data = get_cpu_data();
        let mut opt = (*cpu_data.executor).current_task();
        let current_task = opt.as_mut().expect("No current task");
        let mut kernel_ctx = *cpu_data.kernel_ctx; // deref the ptr so we can pass
        // it as a reference (didn't need
        // to deref but it's cleaner)
        switch_to_task_context(&mut current_task.task_context, &mut kernel_ctx);
    });
}
