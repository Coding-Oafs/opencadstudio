//! Windows stack-overflow diagnostics: a vectored exception handler that
//! captures the crashing call stack and prints module-relative offsets to
//! stderr. Rust cannot catch a stack overflow, so without this the GUI dies
//! with nothing but an exit code; with it, the printed offsets can be mapped
//! to functions with `scripts/symbolicate.ps1` and the PDB.

#![cfg(all(windows, debug_assertions))]

use std::sync::atomic::{AtomicBool, Ordering};

const EXCEPTION_CONTINUE_SEARCH: i32 = 0;
const EXCEPTION_STACK_OVERFLOW: u32 = 0xC000_00FD;

static REPORTED: AtomicBool = AtomicBool::new(false);

#[repr(C)]
struct ExceptionRecord {
    code: u32,
    _flags: u32,
    _record: *mut u8,
    _address: *mut u8,
    _parameters: [usize; 13],
    __pad: [usize; 2],
}

#[repr(C)]
struct ExceptionPointers {
    record: *mut ExceptionRecord,
    _context: *mut u8,
}

extern "system" {
    fn AddVectoredExceptionHandler(first: u32, handler: extern "system" fn(*mut ExceptionPointers) -> i32) -> *mut u8;
    fn GetModuleHandleExW(flags: u32, name: *const u16, module: *mut *mut u8) -> i32;
    fn RtlCaptureStackBackTrace(skip: u32, count: u32, buffer: *mut *mut u8, hash: *mut u32) -> u16;
    fn GetStdHandle(which: i32) -> *mut u8;
    fn WriteFile(file: *mut u8, bytes: *const u8, length: u32, written: *mut u32, overlapped: *mut u8) -> i32;
}

extern "system" fn on_exception(pointers: *mut ExceptionPointers) -> i32 {
    // Runs on the faulting thread with almost no stack left: touch nothing
    // heavy, and never format with the standard library here.
    unsafe {
        let record = (*pointers).record;
        if (*record).code != EXCEPTION_STACK_OVERFLOW
            || REPORTED.swap(true, Ordering::SeqCst)
        {
            return EXCEPTION_CONTINUE_SEARCH;
        }
        let mut frames: [*mut u8; 64] = [std::ptr::null_mut(); 64];
        let captured = RtlCaptureStackBackTrace(0, 64, frames.as_mut_ptr(), std::ptr::null_mut());
        let mut image_base: *mut u8 = std::ptr::null_mut();
        let mut this_module = std::ptr::null_mut();
        // Address of `on_exception` itself lives in this module.
        let probe = on_exception as *mut u8;
        // MODULE-RELATIVE offsets need the runtime image base; find it from
        // any known address via GetModuleHandleExW(RELATIVE|UNCHANGED).
        GetModuleHandleExW(
            0x6, // GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | ..._UNCHANGED_REFCOUNT (4|2)
            probe as *const u16,
            &mut this_module,
        );
        image_base = this_module;
        // Pre-formatted ASCII message buffer (no allocator, no fmt).
        let mut message = [0_u8; 2048];
        let mut cursor = 0_usize;
        const PREFIX: &[u8] = b"\nSTACK-OVERFLOW frame offsets (module+hex):\n";
        for byte in PREFIX {
            message[cursor] = *byte;
            cursor += 1;
        }
        for index in 0..captured as usize {
            let offset = frames[index] as usize - image_base as usize;
            message[cursor] = b' ';
            message[cursor + 1] = b'0';
            message[cursor + 2] = b'x';
            cursor += 3;
            let mut shift = 60;
            let mut printed = false;
            while shift > 0 {
                let nibble = (offset >> shift) & 0xF;
                if nibble != 0 || printed {
                    message[cursor] = match nibble {
                        0..=9 => b'0' + nibble as u8,
                        _ => b'a' + (nibble as u8 - 10),
                    };
                    cursor += 1;
                    printed = true;
                }
                shift -= 4;
            }
            let nibble = offset & 0xF;
            message[cursor] = match nibble {
                0..=9 => b'0' + nibble as u8,
                _ => b'a' + (nibble as u8 - 10),
            };
            cursor += 1;
            message[cursor] = b'\n';
            cursor += 1;
        }
        let stderr = GetStdHandle(-12); // STD_ERROR_HANDLE
        let mut written = 0_u32;
        WriteFile(stderr, message.as_ptr(), cursor as u32, &mut written, std::ptr::null_mut());
    }
    EXCEPTION_CONTINUE_SEARCH
}

/// Installs the handler. Safe to call once at startup.
pub unsafe fn install() {
    AddVectoredExceptionHandler(1, on_exception);
}
