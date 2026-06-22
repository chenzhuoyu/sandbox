use std::{
    fmt::{Debug, Formatter, Pointer, Result as FmtResult},
    ops::{Add, AddAssign},
};

use bytemuck::{Pod, Zeroable};
use fn_ptr::{FnPtr, UntypedFnPtr};

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Pod, Zeroable)]
pub struct Uintptr(usize);

impl Uintptr {
    pub const NIL: Self = Self(0);
}

impl Uintptr {
    #[inline]
    pub const fn new(addr: usize) -> Self {
        Self(addr)
    }
}

impl Uintptr {
    #[inline]
    pub fn from_ptr<T>(ptr: *const T) -> Self {
        Self(ptr.addr())
    }
}

impl Uintptr {
    #[inline]
    pub const fn addr(self) -> usize {
        self.0
    }

    #[inline]
    pub const fn is_nil(self) -> bool {
        self.0 == 0
    }

    #[inline]
    pub const fn as_ptr<T>(self) -> *mut T {
        self.0 as *mut T
    }
}

impl Uintptr {
    #[inline]
    pub fn as_fn<F: FnPtr>(self) -> F {
        unsafe { F::from_ptr(self.0 as UntypedFnPtr) }
    }
}

impl Uintptr {
    #[inline]
    pub fn read<T>(self) -> T {
        unsafe { (self.0 as *const T).read() }
    }

    #[inline]
    pub fn write<T>(self, value: T) {
        unsafe { (self.0 as *mut T).write(value) }
    }
}

impl Add<u64> for Uintptr {
    type Output = Self;

    #[inline]
    fn add(self, rhs: u64) -> Self {
        Self(self.0.wrapping_add(rhs as usize))
    }
}

impl Add<i64> for Uintptr {
    type Output = Self;

    #[inline]
    fn add(self, rhs: i64) -> Self {
        Self(self.0.wrapping_add_signed(rhs as isize))
    }
}

impl Add<usize> for Uintptr {
    type Output = Self;

    #[inline]
    fn add(self, rhs: usize) -> Self {
        Self(self.0.wrapping_add(rhs))
    }
}

impl Add<isize> for Uintptr {
    type Output = Self;

    #[inline]
    fn add(self, rhs: isize) -> Self {
        Self(self.0.wrapping_add_signed(rhs))
    }
}

impl AddAssign<u64> for Uintptr {
    #[inline]
    fn add_assign(&mut self, rhs: u64) {
        *self = *self + rhs;
    }
}

impl AddAssign<i64> for Uintptr {
    #[inline]
    fn add_assign(&mut self, rhs: i64) {
        *self = *self + rhs;
    }
}

impl AddAssign<usize> for Uintptr {
    #[inline]
    fn add_assign(&mut self, rhs: usize) {
        *self = *self + rhs;
    }
}

impl AddAssign<isize> for Uintptr {
    #[inline]
    fn add_assign(&mut self, rhs: isize) {
        *self = *self + rhs;
    }
}

impl<F: FnPtr> From<F> for Uintptr {
    #[inline]
    fn from(f: F) -> Self {
        Self(f.addr())
    }
}

impl Debug for Uintptr {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        Pointer::fmt(self, f)
    }
}

impl Pointer for Uintptr {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "0x{:x}", self.0)
    }
}
