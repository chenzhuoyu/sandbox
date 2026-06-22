use std::{
    borrow::Cow,
    ffi::{CString, OsStr},
    fmt::{Debug, Formatter, Result as FmtResult},
    fs::File,
    io::{Error as IoError, Read, Result as IoResult},
    mem::MaybeUninit,
    ops::Range,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::Path,
    sync::LazyLock,
};

use anyhow::{Context, anyhow, ensure};
use object::{
    LittleEndian,
    macho::{
        CPU_TYPE_ARM64, CPU_TYPE_X86_64, FAT_CIGAM, FAT_CIGAM_64, FatArch32, FatArch64,
        MH_CIGAM_64, MH_MAGIC, MH_MAGIC_64, MachHeader64, SegmentCommand64,
    },
    read::macho::{FatArch, LoadCommandVariant, MachHeader, Segment},
};

use crate::{
    Maybe,
    utils::{
        io::{MappedFile, MemoryIo, ValueExt},
        path::LibPathNormalizeExt,
        ptr::Uintptr,
        str::Sz,
    },
};

#[repr(C)]
#[derive(Debug)]
struct DyldImageInfo {
    addr: Uintptr,
    path: Sz,
    time: i64,
}

static LLDB_IMAGE_NOTIFIER: LazyLock<
    Option<unsafe extern "C" fn(mode: u32, count: usize, info: *const DyldImageInfo)>,
> = LazyLock::new(|| unsafe {
    std::mem::transmute(libc::dlsym(
        libc::dlopen(c"dyld".as_ptr(), libc::RTLD_LAZY),
        c"lldb_image_notifier".as_ptr(),
    ))
});

pub struct Image {
    pub size: usize,
    pub base: Uintptr,
    pub main: Uintptr,
}

impl Image {
    const CPU_TYPE: u32 = CPU_TYPE_ARM64;
    const DYLD_PATH: &str = "/usr/lib/dyld";
    const DYLD_BASE: usize = 0x10000018000;
}

impl Image {
    fn read<T>(file: &mut File) -> Maybe<T> {
        unsafe {
            let mut ret = MaybeUninit::<T>::uninit();
            file.read_exact(ret.as_bytes_mut().assume_init_mut())?;
            Ok(ret.assume_init())
        }
    }

    fn map_image(path: &Path) -> Maybe<MappedFile> {
        let mut file = File::open(path)?;
        let magic = Self::read::<u32>(&mut file)?;

        /* check the file magic, and remap the file at correct offset */
        let offset = match magic {
            MH_MAGIC => return Err(anyhow!("32-bit binaries are not supported: {path:?}")),
            MH_MAGIC_64 => 0usize,
            FAT_CIGAM => Self::find_binary(&mut file, false)?,
            FAT_CIGAM_64 => Self::find_binary(&mut file, true)?,
            _ => return Err(anyhow!("not a valid Mach-O binary: {path:?}")),
        };

        /* map the image */
        let size = file.metadata()?.len() as usize;
        let size = size - offset;
        MappedFile::map(file, size, offset)
    }

    fn find_binary(file: &mut File, is_fat64: bool) -> Maybe<usize> {
        for _ in 0..Self::read::<u32>(file)? {
            let (cpu, offset) = {
                if is_fat64 {
                    let arch = Self::read::<FatArch64>(file)?;
                    (arch.cputype(), arch.offset() as usize)
                } else {
                    let arch = Self::read::<FatArch32>(file)?;
                    (arch.cputype(), arch.offset() as usize)
                }
            };
            if cpu == Self::CPU_TYPE {
                return Ok(offset);
            }
        }
        Err(anyhow!("cannot find valid architecture in fat binary"))
    }

    fn load_segment(
        fd: i32,
        seg: &SegmentCommand64<LittleEndian>,
        slide: usize,
        offset: usize,
    ) -> IoResult<Range<Uintptr>> {
        let vma_addr = Uintptr::new(seg.vmaddr.usize() + slide);
        let vma_next = vma_addr + seg.vmsize.usize();

        /* nothing to load, just protect the segment */
        if seg.filesize.value() == 0 {
            return Ok(vma_addr..vma_next);
        }

        /* set segment protection to RW */
        let ret = unsafe {
            libc::mprotect(
                vma_addr.as_ptr(),
                seg.vmsize.usize(),
                libc::PROT_READ | libc::PROT_WRITE,
            )
        };

        /* check for errors */
        if ret != 0 {
            return Err(IoError::last_os_error());
        }

        /* map the segment */
        let ret = unsafe {
            libc::mmap(
                vma_addr.as_ptr(),
                seg.filesize.usize(),
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_FIXED,
                fd,
                (offset + seg.fileoff.usize()) as libc::off_t,
            )
        };

        /* check map result */
        if !std::ptr::eq(ret, vma_addr.as_ptr()) {
            Err(IoError::last_os_error())
        } else {
            Ok(vma_addr..vma_next)
        }
    }

    fn notify_debugger(path: &Path, base: Uintptr) {
        if let Some(lldb_image_notifier) = *LLDB_IMAGE_NOTIFIER {
            let name = CString::new(path.as_os_str().as_bytes())
                .map_or(Cow::Borrowed(c"(???)"), Cow::Owned);
            let info = DyldImageInfo {
                addr: base,
                path: Sz::from(name.as_ptr()),
                time: path.metadata().map_or(0, |m| m.mtime()),
            };
            unsafe {
                lldb_image_notifier(0, 1, &raw const info);
            }
        }
    }
}

impl Image {
    pub fn load<P: AsRef<Path>>(path: P) -> Maybe<Self> {
        let path = path.as_ref().normalize()?;
        let file = Self::map_image(&path)?;

        /* read the mach header */
        let mio = &mut MemoryIo(&file);
        let hdr = mio.read::<MachHeader64<LittleEndian>>()?;

        /* validate the Mach-O magic again */
        if hdr.magic() != MH_CIGAM_64 {
            return Err(anyhow!("must be 64-bit little-endian executables"));
        }

        /* virtual address range & fixup types */
        let mut max_addr = 0u64;
        let mut min_addr = u64::MAX;
        let mut segments = Vec::with_capacity(8);
        let mut entry_point = None;

        /* helper macro to read NULL-terminated strings */
        macro_rules! read_cstr {
            ($cmd:ident $(.$field:ident)+) => {{
                unsafe {
                    let field: &object::macho::LcStr<_> = &$cmd $(.$field)+;
                    let data = (&raw const *$cmd as *const u8).add(field.offset.usize());
                    let len = file.as_ptr().add(file.len()).offset_from(data) as usize;
                    std::ffi::CStr::from_bytes_until_nul(std::slice::from_raw_parts(data, len))
                }
            }};
        }

        /* verify & collect load commands */
        for cmd in hdr.load_commands(LittleEndian, file.data(), 0)? {
            match cmd?.variant()? {
                LoadCommandVariant::Thread(.., data) => {
                    if entry_point.is_none() {
                        entry_point = Some(match hdr.cputype.value() {
                            CPU_TYPE_X86_64 => MemoryIo(data).read_at(136)?,
                            CPU_TYPE_ARM64 => MemoryIo(data).read_at(264)?,
                            _ => unreachable!(),
                        })
                    }
                }
                LoadCommandVariant::LoadDylinker(cmd) => {
                    let dyld = read_cstr!(cmd.name)?;
                    ensure!(dyld == c"/usr/lib/dyld", "unknown loader: {dyld:?}");
                }
                LoadCommandVariant::Segment64(seg, ..) => {
                    if seg.name() != b"__PAGEZERO" {
                        min_addr = min_addr.min(seg.vmaddr.value());
                        max_addr = max_addr.max(seg.vmaddr.value() + seg.vmsize.value());
                        segments.push(seg);
                    }
                }
                LoadCommandVariant::EntryPoint(cmd) => {
                    if entry_point.is_none() {
                        entry_point = Some(cmd.entryoff.usize());
                    } else {
                        tracing::warn!("Found multiple entry points");
                    }
                }
                _ => {}
            }
        }

        /* __TEXT segment should start at fileoff 0 and have the lowest vmaddr */
        for &seg in &segments {
            if seg.name() == b"__TEXT" {
                if seg.fileoff.value() != 0 || seg.vmaddr.value() != min_addr {
                    return Err(anyhow!("malformed Mach-O file: misplaced __TEXT segment"));
                } else {
                    break;
                }
            }
        }

        /* calculate the reservation size */
        let page_size = page_size::get();
        let addr_size = (max_addr - min_addr) as usize;
        let load_size = (addr_size + page_size - 1) & !(page_size - 1);

        /* disable ASLR under debug mode */
        let (map_addr, map_flags) = {
            if cfg!(debug_assertions) && path.as_path() == Self::DYLD_PATH {
                (Self::DYLD_BASE, libc::MAP_FIXED)
            } else {
                (0, 0)
            }
        };

        /* reserve the virtual address range */
        let base = unsafe {
            Uintptr::from_ptr(libc::mmap(
                map_addr as *mut libc::c_void,
                load_size,
                libc::PROT_NONE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | map_flags,
                -1,
                0,
            ))
        };

        /* check the map result */
        if std::ptr::eq(base.as_ptr(), libc::MAP_FAILED) {
            return Err(IoError::last_os_error()).context("cannot reserve virtual address range");
        }

        /* calculate ASLR slide */
        let lower = min_addr as usize;
        let slide = base.addr() - lower;

        /* load the segments */
        for &seg in &segments {
            let vma = {
                Self::load_segment(file.fd(), seg, slide, file.offset()).with_context(|| {
                    format!(
                        "cannot load segment {:?} at 0x{:x}-0x{:x}",
                        OsStr::from_bytes(seg.name()),
                        seg.vmaddr.value(),
                        seg.vmaddr.value() + seg.vmsize.value()
                    )
                })?
            };
            tracing::debug!(
                "Segment {:?} is loaded at 0x{:x}-0x{:x} ({:p}-{:p})",
                OsStr::from_bytes(seg.name()),
                seg.vmaddr.value(),
                seg.vmaddr.value() + seg.vmsize.value(),
                vma.start,
                vma.end
            );
        }

        /* set the segments with correct protection */
        for &seg in &segments {
            let ret = unsafe {
                libc::mprotect(
                    (seg.vmaddr.usize() + slide) as *mut libc::c_void,
                    seg.vmsize.usize(),
                    seg.initprot.value() as libc::vm_prot_t,
                )
            };
            if ret != 0 {
                return Err(IoError::last_os_error()).context("cannot set segment protections");
            }
        }

        /* get the entry point */
        let size = load_size;
        let main = entry_point.map_or(Uintptr::NIL, |entry| base + entry);

        /* notify the debugger, then construct the image */
        Self::notify_debugger(&path, base);
        Ok(Self { base, size, main })
    }
}

impl Debug for Image {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(
            f,
            "Image({:p}-{:p},entry={:p})",
            self.base,
            self.base + self.size,
            self.main
        )
    }
}
