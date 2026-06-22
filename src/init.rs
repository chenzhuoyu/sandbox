use crate::utils::{ptr::Uintptr, str::Sz};

pub const MAX_KERNEL_ARGS: usize = 128;
pub const INIT_STACK_SIZE: usize = 1048576;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KernelArgs {
    pub base: Uintptr,
    pub argc: usize,
    pub args: [Sz; MAX_KERNEL_ARGS],
}

const KARGS_SIZE: usize = std::mem::size_of::<KernelArgs>();
const ZEROS_SIZE: usize = INIT_STACK_SIZE - KARGS_SIZE;

#[repr(C)]
pub struct InitStackFrame {
    pub zero: [u8; ZEROS_SIZE],
    pub args: KernelArgs,
}

impl InitStackFrame {
    pub fn new(base: Uintptr) -> Box<Self> {
        let mut frame = unsafe { Box::<Self>::new_zeroed().assume_init() };
        frame.args.base = base;
        frame
    }
}
