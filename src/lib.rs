#![feature(cstr_display)]
#![feature(maybe_uninit_as_bytes)]
#![allow(internal_features)]

pub mod image;
pub mod init;
pub mod utils;

use image::Image;
use init::{InitStackFrame, KernelArgs};
use utils::str::Sz;

pub type Unit = Maybe<()>;
pub type Error = anyhow::Error;
pub type Maybe<T> = Result<T, Error>;

pub fn sandbox_main() -> Unit {
    let dyld = Image::load("/usr/lib/dyld").unwrap();
    dbg!(&dyld);
    let code = dyld.main.read::<[u32; 8]>();
    for (i, &inst) in code.iter().enumerate() {
        eprint!("{:p}: {inst:08x}  ", dyld.main + i * 4);
        if let Some(op) = disarm64::decoder::decode(inst) {
            eprintln!("{op}");
        } else {
            eprintln!("(???)");
        }
    }
    let ls = Image::load("/bin/ls").unwrap();
    dbg!(&ls);
    let mut sf = InitStackFrame::new(ls.base);
    sf.args.argc = 1;
    sf.args.args[0] = Sz::from(c"ls");
    sf.args.args[1] = Sz::NIL;
    sf.args.args[2] = Sz::NIL;
    sf.args.args[3] = Sz::from(c"executable_path=/bin/ls");
    sf.args.args[4] = Sz::NIL;
    let frame = &raw mut sf.args;
    let dyld_start = dyld.main.as_fn::<extern "C" fn(*mut KernelArgs)>();
    unsafe {
        std::arch::asm! {
            "mov x0, #0",
            "mov x1, #0",
            "mov x2, #0",
            "mov x3, #0",
            "mov x4, #0",
            "mov x5, #0",
            "mov x6, #0",
            "mov x7, #0",
            "mov sp, {frame}",
            "mov x8, {dyld_start}",
            "brk #0",
            "br  x8",
            frame = in(reg) frame,
            dyld_start = in(reg) dyld_start,
        }
    }
    Ok(())
}
