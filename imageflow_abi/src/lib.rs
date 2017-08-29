//! # Purpose
//!
//! This module contains the functions exported for use by other languages.
//!
//!
//! If you're writing bindings, you're in the right place. Don't use imageflow_core::ffi
//!
//! Don't call functions against the same context from multiple threads. You can create contexts
//! from as many threads as you like, but you are responsible for synchronizing API calls
//! on a per-context basis if you want to use one context from multiple threads. No use
//! case for multithreaded Context access has been presented, so it is out of scope for API design.
//!
//!
//!
//! # Memory Lifetimes
//!
//! In order to prevent dangling pointers, we must be correct about memory lifetimes.
//!
//! ## ... when allocated by Imageflow, assume the lifetime of the `context`
//!
//! **In Imageflow, by default, all things created with a context will be destroyed when the
//! context is destroyed.** Don't try to access ANYTHING imageflow has provided after the context is gone.
//!
//! This is very nice, as it means that a client's failure to clean up
//! will have limited impact on the process as a whole - as long as the client at minimum
//! calls `flow_context_destroy` at the end of all possible code paths.
//!
//! However, waiting to free memory and run destructors until the context is destroyed is not ideal;
//! it increases our peak memory usage/needs and may cause operations
//! to fail that would otherwise succeed.
//!
//! There are two ways to mitigate this.
//!
//! 1. When creating an I/O object, request it be cleaned up when the first job it is assigned to is cleaned up (instead of the context).
//! 2. Manually invoke the corresponding destroy function when you're done with the thing.
//!
//! ### Destroying things
//!
//! * An `imageflow_context` should ALWAYS be destroyed with `imageflow_context_destroy`
//! * JsonResponse structures should be released with `imageflow_json_response_destroy`
//! * An `imageflow_job` can be destroyed early with `imageflow_job_destroy`
//!
//! ## ... when allocated by the client, Imageflow only borrows it for the `invocation`
//!
//! **Imageflow assumes that, at minimum, all pointers that you provide to it will, at minimum,
//! remain valid for the duration of the API call.** We'll call this 'borrowing'. Imageflow is
//! just borrowing it for a bit; not taking ownership of the thing.
//!
//! This may seem obvious, but it is not, in fact, guaranteed by garbage-collected languages. They
//! are oblivious to pointers, and cannot track what data is and is not referenced.
//! Therefore, we suggest that you ensure every allocation made (and handed to Imageflow) is
//! referenced *after* the imageflow API call, preferably in a way that will not be optimized away
//! at runtime. Many languages and FFI libraries offer a utility method just for this purpose.
//!
//! ## ... and it should be very clear when Imageflow is taking ownership of something you created!
//!
//! When Imageflow needs continued access to data that is NOT highly likely to be static, it
//! will be documented.
//!
//! * If you give Imageflow a buffer to read an image from, it will need to access that buffer
//!   much longer than the initial io_create call.
//!
//! ## What if I need something to outlive the `context`?
//!
//! Copy it before the context is destroyed.
//!
//! # Data types
//!
//! Reference for those creating bindings in other languages
//!
//! Two types are platform-specific - use the corresponding pointer or size type that varies with
//! your platform.
//!
//! * libc::c_void (or anything *mut or *const): Platform-sized pointer. 32 or 64 bits.
//! * The above includes *mut Context, *mut Job, *mut JobIo, etc.
//! * libc::size_t (or usize): Unsigned integer, platform-sized. 32 or 64 bits.
//!
//!
//! Treat *mut Context, *mut Job, *mut JobIo, *mut JsonResponse ALL as opaque pointers.
//!
//! ## Strings
//!
//! ASCII is a safe subset of UTF-8; therefore wherever Imageflow asks for UTF-8 encoded bytes, you may provide ASCII instead.
//!
//! You will provide Imageflow with strings in one of 3 ways:
//! * UTF-8 null-terminated. You'll see something like `libc::char`, but no length parameter. Short and likely static strings are usually transmitted this way.
//! * Operating system null-terminated. Only applicable to `imageflow_io_create_for_file`.
//! * UTF-8 buffer with length. You'll usually see *const u8 and a length parameter. This is common for buffers of UTF-8 encoded json.
//!
//! filename: *const libc::c_char
//! function_name: *const libc::c_char
//!
//! Fixed size
//!
//! * u8 (1 unsigned byte)
//! * bool (C99 style, 1 byte, value 0 or 1)
//! * The rest seem self-explanatory.
//! * `i` prefixes signed ints
//! * `u` prefixes unsigned ints.
//! * `f` prefixes floating point
//!
//! Structs
//!
//! Consider all structures to be opaque. Do not attempt to access fields by offsets; rather,
//! use the accessor functions provided.
//!
//!
//! ## Failure behavior
//!
//! If you provide a null pointer for `imageflow_context`, then the process will terminate.
//! This "fail fast" behavior offers the best opportunity for a useful stacktrace, and it's not a
//! recoverable error.
//!
//! If you try to continue using an errored imageflow_context, the process will terminate.
//! Some errors can be recovered from, but you *must* do that before trying to use the context again.
//!
//!
//!
#![crate_type = "cdylib"]
#![feature(alloc_system)]
#![feature(core_intrinsics)]
#[macro_use]
extern crate imageflow_core as c;
extern crate alloc_system;
extern crate libc;
extern crate smallvec;
extern crate backtrace;
use c::ffi;

pub use c::{Context, Job, FlowError, ErrorCategory, ErrorKind, CodeLocation};
pub use c::IoProxy as JobIo;
pub use c::ffi::ImageflowJsonResponse as JsonResponse;
use std::any::Any;
use std::ptr;
use std::io::Write;
use std::ffi::CStr;
use std::panic::{catch_unwind, AssertUnwindSafe};
#[cfg(test)]
use std::str;


///
/// What is possible with the IO object
#[repr(C)]
pub enum IoMode {
    None = 0,
    ReadSequential = 1,
    WriteSequential = 2,
    ReadSeekable = 5, // 1 | 4,
    WriteSeekable = 6, // 2 | 4,
    ReadWriteSeekable = 15, // 1 | 2 | 4 | 8
}

///
/// Input or output?
#[repr(C)]
#[derive(Copy,Clone)]
pub enum Direction {
    Out = 8,
    In = 4,
}

///
/// When a resource should be closed/freed/cleaned up
///
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CleanupWith{
    /// When the context is destroyed
    Context = 0,
    /// When the first job that the item is associated with is destroyed. (Not yet implemented)
    FirstJob = 1
}

///
/// How long the provided pointer/buffer will remain valid.
/// Callers must prevent the memory from being freed or moved until this contract expires.
///
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Lifetime{
    /// Pointer will outlive function call. If the host language has a garbage collector, call the appropriate method to ensure the object pointed to will not be collected or moved until the call returns. You may think host languages do this automatically in their FFI system. Most do not.
    OutlivesFunctionCall = 0,
    /// Pointer will outlive context. If the host language has a GC, ensure that you are using a data type guaranteed to neither be moved or collected automatically.
    OutlivesContext = 1,
}



/// Creates a static, null-terminated Rust string, and
/// returns a ` *const libc::c_char` pointer to it.
///
/// Useful for API invocations that require a static C string
macro_rules! static_char {
    ($lit:expr) => {
        concat!($lit, "\0").as_ptr() as *const libc::c_char
    }
}

fn type_name_of<T>(_: T) -> &'static str {
    extern crate core;
    unsafe { core::intrinsics::type_name::<T>() }
}

/// Returns a reference to a static string containing the parent function name
macro_rules! short_function {
    () => {{
        fn f() {}
        let name = type_name_of(f);
        &name[..name.len() - 4].rsplit_terminator(":").next().unwrap_or("[function name not found]")
    }}
}


macro_rules! context {
    ($ptr:ident) => {{
        if $ptr.is_null() {
            fn f() {}
            let name = type_name_of(f);
            let shortname = &name[..name.len() - 4].rsplit_terminator(":").next().unwrap_or("[function name not found]");
            eprintln!("Null context pointer provided to {}. Terminating process.", shortname);
            let bt = ::backtrace::Backtrace::new();
            eprintln!("{:?}", bt);
            ::std::process::abort();
        }
        (&mut *$ptr)
    }}
}

macro_rules! context_ready {
    ($ptr:ident) => {{
        if $ptr.is_null() {
            fn f() {}
            let name = type_name_of(f);
            let shortname = &name[..name.len() - 4].rsplit_terminator(":").next().unwrap_or("[function name not found]");
            eprintln!("Null context pointer provided to {}. Terminating process.", shortname);
            let bt = ::backtrace::Backtrace::new();
            eprintln!("{:?}", bt);
            ::std::process::abort();
        }else if (&*$ptr).outward_error().has_error(){
            fn f() {}
            let name = type_name_of(f);
            let shortname = &name[..name.len() - 4].rsplit_terminator(":").next().unwrap_or("[function name not found]");
            eprintln!("The Context passed to {} is in an error state and cannot be used. Terminating process.", shortname);
            eprintln!("{}",(&*$ptr).outward_error());

            let bt = ::backtrace::Backtrace::new();
            eprintln!("{} was invoked by: \n{:?}", shortname, bt);
            ::std::process::abort();
        }
        (&mut *$ptr)
    }}
}
macro_rules! handle_result {
    ($context:ident, $result:expr, $failure_value:expr) => {{
        match $result{
            Ok(Ok(v)) => v,
            Err(p) => {
                $context.outward_error_mut().try_set_panic_error(p); $failure_value
            },
            Ok(Err(error)) => {
                $context.outward_error_mut().try_set_error(error); $failure_value
            }
        }
        }}
}

/// Creates and returns an imageflow context.
/// An imageflow context is required for all other imageflow API calls.
///
/// An imageflow context tracks
/// * error state
/// * error messages
/// * stack traces for errors (in C land, at least)
/// * context-managed memory allocations
/// * performance profiling information
///
/// **Contexts are not thread-safe!** Once you create a context, *you* are responsible for ensuring that it is never involved in two overlapping API calls.
///
/// Returns a null pointer if allocation fails.
#[no_mangle]
pub unsafe extern "C" fn imageflow_context_create() -> *mut Context {
    Context::create_cant_panic().map(|b| Box::into_raw(b)).unwrap_or(std::ptr::null_mut())
}

/// Begins the process of destroying the context, yet leaves error information intact
/// so that any errors in the tear-down process can be
/// debugged with imageflow_context_error_and_stacktrace.
///
/// Returns true if no errors occurred. Returns false if there were tear-down issues.
///
/// *Behavior is undefined if context is a null or invalid ptr.*
#[no_mangle]
pub unsafe extern "C" fn imageflow_context_begin_terminate(context: *mut Context) -> bool {
    let c: &mut Context = context!(context);
    c.abi_begin_terminate()
}

/// Destroys the imageflow context and frees the context object.
/// Only use this with contexts created using imageflow_context_create
///
/// Behavior is undefined if context is a null or invalid ptr; may segfault on free(NULL);
#[no_mangle]
pub unsafe extern "C" fn imageflow_context_destroy(context: *mut Context) {
    let _ = Box::from_raw(context);

}


#[test]
fn test_create_destroy() {
    exercise_create_destroy();
}

pub fn exercise_create_destroy() {
    unsafe {
        let c = imageflow_context_create();
        assert!(!c.is_null());
        assert!(imageflow_context_begin_terminate(c));
        imageflow_context_destroy(c);
    }
}

/// Returns true if the context is in an error state. You must immediately deal with the error,
/// as subsequent API calls will fail or cause undefined behavior until the error state is cleared
///
/// Behavior is undefined if `context` is a dangling or invalid ptr; segfault likely.
#[no_mangle]
pub unsafe extern "C" fn imageflow_context_has_error(context: *mut Context) -> bool {
    context!(context).outward_error_mut().has_error()
}

/// Returns true if the context is "ok" or in an error state that is recoverable.
/// You must immediately deal with the error,
/// as subsequent API calls will fail or cause undefined behavior until the error state is cleared
///
/// Behavior is undefined if `context` is a dangling or invalid ptr; segfault likely.
#[no_mangle]
pub unsafe extern "C" fn imageflow_context_error_recoverable(context: *mut Context) -> bool {
    context!(context).outward_error_mut().recoverable()
}

/// Returns true if the context is "ok" or in an error state that is recoverable.
/// You must immediately deal with the error,
/// as subsequent API calls will fail or cause undefined behavior until the error state is cleared
///
/// Behavior is undefined if `context` is a dangling or invalid ptr; segfault likely.
#[no_mangle]
pub unsafe extern "C" fn imageflow_context_error_try_clear(context: *mut Context) -> bool {
    context!(context).outward_error_mut().try_clear()
}


/// Prints the error messages and stacktrace to the given buffer in UTF-8 form; writes a null
/// character to terminate the string, and *ALSO* returns the number of bytes written.
///
///
/// Happy(ish) path: Returns the length of the error message written to the buffer.
/// Sad path: Returns -1 if buffer_length was too small or buffer was nullptr.
/// full_file_path, if true, will display the directory associated with the files in each stack frame.
///
/// Please be accurate with the buffer length, or a buffer overflow will occur.
///
/// Behavior is undefined if `context` is a dangling or invalid ptr; segfault likely.
#[no_mangle]
pub unsafe extern "C" fn imageflow_context_error_and_stacktrace(context: *mut Context,
                                                                buffer: *mut libc::c_char,
                                                                buffer_length: libc::size_t,
                                                                full_file_path: bool)
                                                                -> i64 {
    if buffer.is_null(){
        -1
    }else {
        use c::errors::writing_to_slices::WriteResult;
        let c = context!(context);
        let result = c.outward_error_mut().get_buffer_writer().write_and_write_errors_to_cstring(buffer as *mut u8, buffer_length, Some("\n[truncated]\n"));
        match result {
            WriteResult::AllWritten(v) => v as i64,
            _ => -1
        }
    }
}
/// Prints the error messages (and optional stack frames) to the given buffer in UTF-8 form; writes a null
/// character to terminate the string, and *ALSO* provides the number of bytes written (excluding the null terminator)
///
/// Returns false if the buffer was too small (or null) and the output was truncated.
/// Returns true if all data was written OR if there was a bug in error serialization (that gets written, too).
///
/// If the data is truncated, "\n[truncated]\n" is written to the buffer
///
/// Please be accurate with the buffer length, or a buffer overflow will occur.
///
/// Behavior is undefined if `context` is a dangling or invalid ptr; segfault likely.
#[no_mangle]
pub unsafe extern "C" fn imageflow_context_error_write_to_buffer(context: *mut Context,
                                                                buffer: *mut libc::c_char,
                                                                buffer_length: libc::size_t,
                                                                bytes_written: *mut libc::size_t) -> bool {
    if buffer.is_null(){
        false
    }else {
        use c::errors::writing_to_slices::WriteResult;
        let c = context!(context);
        let result = c.outward_error_mut().get_buffer_writer().write_and_write_errors_to_cstring(buffer as *mut u8, buffer_length, Some("\n[truncated]\n"));
        *bytes_written = result.bytes_written();
        match result {
            WriteResult::AllWritten(_) |
            WriteResult::Error { .. } => true,
            WriteResult::TruncatedAt(_) => false,
        }
    }
}

/// Returns the numeric code associated with the error.
///
/// ## Error categories
///
/// * 0 - No error condition.
///
///
/// Behavior is undefined if `context` is a dangling or invalid ptr; segfault likely.
#[no_mangle]
pub unsafe extern "C" fn imageflow_context_error_code(context: *mut Context) -> i32 {
    context!(context).outward_error_mut().category().to_c_error_code()
}

/// Prints the error to stderr and exits the process if an error has been raised on the context.
/// If no error is present, the function returns false.
///
/// Behavior is undefined if `context` is a dangling or invalid ptr; segfault likely.
///
/// THIS PRINTS DIRECTLY TO STDERR! Do not use in any kind of service! Command-line usage only!
#[no_mangle]
pub unsafe extern "C" fn imageflow_context_print_and_exit_if_error(context: *mut Context) -> bool {
    let e = context!(context).outward_error();
    if e.has_error(){
        eprintln!("{}",e);
        true
    }else{
        false
    }

}



///
/// Writes fields from the given imageflow_json_response to the locations referenced.
/// The buffer pointer sent out will be a UTF-8 byte array of the given length (not null-terminated). It will
/// also become invalid if the JsonResponse associated is freed, or if the context is destroyed.
///
#[no_mangle]
pub unsafe extern fn imageflow_json_response_read(context: *mut Context,
                                                  response_in: *const JsonResponse,
                                                  status_code_out: *mut i64,
                                                  buffer_utf8_no_nulls_out: *mut *const u8,
                                                  buffer_size_out: *mut libc::size_t) -> bool {
    let mut c = context_ready!(context);

    if response_in.is_null() {
        c.outward_error_mut().try_set_error(nerror!(ErrorKind::NullArgument, "The argument response_in (* JsonResponse) is null."));
        return false;
    }

    if !status_code_out.is_null() {
        *status_code_out = (*response_in).status_code;
    }
    if !buffer_utf8_no_nulls_out.is_null() {
        *buffer_utf8_no_nulls_out = (*response_in).buffer_utf8_no_nulls;
    }
    if !buffer_size_out.is_null() {
        *buffer_size_out = (*response_in).buffer_size;
    }
    return true;
}


/// Frees memory associated with the given object (and owned objects) after
/// running any owned or attached destructors. Returns false if something went wrong during tear-down.
///
/// Returns true if the object to destroy is a null pointer, or if tear-down was successful.
///
/// Behavior is undefined if the pointer is dangling or not a valid memory reference.
/// Although certain implementations catch
/// some kinds of invalid pointers, a segfault is likely in future revisions).
///
/// Behavior is undefined if the context provided does not match the context with which the
/// object was created.
///
/// Behavior is undefined if `context` is a dangling or invalid ptr; segfault likely.
///
#[no_mangle]
pub unsafe extern "C" fn imageflow_json_response_destroy(context: *mut Context,
                                                         response: *mut JsonResponse)
                                                         -> bool {
    imageflow_context_memory_free(context, response as *mut libc::c_void, ptr::null(), 0)
}

///
/// Sends a JSON message to the imageflow_context
///
/// The context is provided `method`, which determines which code path will be used to
/// process the provided JSON data and compose a response.
///
/// * `method` and `json_buffer` are only borrowed for the duration of the function call. You are
///    responsible for their cleanup (if necessary - static strings are handy for things like
///    `method`).
/// * `method` should be a UTF-8 null-terminated string.
///   `json_buffer` should be a UTF-8 encoded buffer (not null terminated) of length json_buffer_size.
///
/// You should call `imageflow_context_has_error()` to see if this succeeded.
///
/// A JsonResponse is returned for success and most error conditions.
/// Call `imageflow_json_response_destroy` when you're done with it (or dispose the context).
///
/// Behavior is undefined if `context` is a dangling or invalid ptr; segfault likely.
#[no_mangle]
#[allow(unused_variables)]
pub unsafe extern "C" fn imageflow_context_send_json(context: *mut Context,
                                                     method: *const libc::c_char,
                                                     json_buffer: *const u8,
                                                     json_buffer_size: libc::size_t)
                                                     -> *const JsonResponse {
    imageflow_send_json(context, None, method, json_buffer, json_buffer_size)
}

///
/// Sends a JSON message to the imageflow_job
///
/// The recipient is provided `method`, which determines which code path will be used to
/// process the provided JSON data and compose a response.
///
/// * `method` and `json_buffer` are only borrowed for the duration of the function call. You are
///    responsible for their cleanup (if necessary - static strings are handy for things like
///    `method`).
/// * `method` should be a UTF-8 null-terminated string.
///   `json_buffer` should be a UTF-8 encoded buffer (not null terminated) of length json_buffer_size.
///
/// You should call `imageflow_context_has_error()` to see if this succeeded.
///
/// A JsonResponse is returned for success and most error conditions.
/// Call `imageflow_json_response_destroy` when you're done with it (or dispose the context).
///
/// Behavior is undefined if `context` is a dangling or invalid ptr; segfault likely.
#[no_mangle]
#[allow(unused_variables)]
pub unsafe extern "C" fn imageflow_job_send_json(context: *mut Context,
                                                 job: *mut Job,
                                                 method: *const libc::c_char,
                                                 json_buffer: *const libc::uint8_t,
                                                 json_buffer_size: libc::size_t)
                                                 -> *const JsonResponse {
    imageflow_send_json(context, Some(job), method, json_buffer, json_buffer_size)
}

///
/// Sends a JSON message to one of 2 recipients.
///
/// 1. `imageflow_context`, If both `job` and `io` are both null. Required.
/// 2. `imageflow_job`, if only `io` is null.
///
/// The recipient is then provided `method`, which determines which code path will be used to
/// process the provided JSON data and compose a response.
///
/// * `method` and `json_buffer` are only borrowed for the duration of the function call. You are
///    responsible for their cleanup (if necessary - static strings are handy for things like
///    `method`).
///
///
/// You should call `imageflow_context_has_error()` to see if this succeeded.
///
/// A JsonResponse is returned for success and most error conditions.
/// Call `imageflow_json_response_destroy` when you're done with it (or dispose the context).
///
/// Behavior is undefined if `context` is a dangling or invalid ptr; segfault likely.
#[allow(unused_variables)]
unsafe fn imageflow_send_json(context: *mut Context,
                              job: Option<*mut Job>,
                              method: *const i8,
                              json_buffer: *const libc::uint8_t,
                              json_buffer_size: libc::size_t)
                              -> *const JsonResponse {

    let c: &mut Context = context_ready!(context);

    if let Some(j) = job {
        if j.is_null() {
            c.outward_error_mut().try_set_error(nerror!(ErrorKind::NullArgument, "The argument 'job' is null."));
            return ptr::null();
        }
    }
    if method.is_null() {
        c.outward_error_mut().try_set_error(nerror!(ErrorKind::NullArgument, "The argument 'method' is null."));
        return ptr::null();
    }
    if json_buffer.is_null() {
        c.outward_error_mut().try_set_error(nerror!(ErrorKind::NullArgument, "The argument 'json_buffer' is null."));
        return ptr::null();
    }
    let panic_result = catch_unwind(AssertUnwindSafe(|| {
        let method_str = if let Ok(str) = ::std::ffi::CStr::from_ptr(method as *const i8).to_str() {
            str
        } else {
            return (ptr::null(), Err(nerror!(ErrorKind::InvalidArgument, "The argument 'method' is invalid UTF-8.")));
        };

        let json_bytes = std::slice::from_raw_parts(json_buffer, json_buffer_size);

        // Segfault early
        let _ = (json_bytes.first(), json_bytes.last());


        let (json, result) = if let Some(j) = job {
                (&mut *j).message(method_str, json_bytes)
            } else {
                c.message(method_str, json_bytes)
            };

        // An unfortunate copy occurs here
        (create_abi_json_response(c, &json.response_json, json.status_code), result)
    }));

    match panic_result{
        Ok((json, Ok(result))) => json,
        Ok((json, Err(e))) => {
            c.outward_error_mut().try_set_error(e);
            json
        }
        Err(p) => {
         c.outward_error_mut().try_set_panic_error(p); ptr::null_mut()
        },
    }
}


pub fn create_abi_json_response(c: &mut Context,
                                json_bytes: &[u8],
                                status_code: i64)
                                -> *const JsonResponse {
    unsafe {
        let sizeof_struct = std::mem::size_of::<JsonResponse>();
        let alloc_size = sizeof_struct + json_bytes.len();

        let pointer = ::ffi::flow_context_calloc(c.flow_c(),
                                                 1,
                                                 alloc_size,
                                                 ptr::null(),
                                                 c.flow_c() as *mut libc::c_void,
                                                 ptr::null(),
                                                 line!() as i32) as *mut u8;
        if pointer.is_null() {
            c.outward_error_mut().try_set_error(nerror!(ErrorKind::AllocationFailed, "Failed to allocate JsonResponse ({} bytes)", alloc_size));
            return ptr::null();
        }
        let pointer_to_final_buffer =
            pointer.offset(sizeof_struct as isize) as *mut libc::uint8_t;
        let imageflow_response = &mut (*(pointer as *mut JsonResponse));
        imageflow_response.buffer_utf8_no_nulls = pointer_to_final_buffer;
        imageflow_response.buffer_size = json_bytes.len();
        imageflow_response.status_code = status_code;

        let mut out_json_bytes = std::slice::from_raw_parts_mut(pointer_to_final_buffer,
                                                                json_bytes.len());

        out_json_bytes.clone_from_slice(&json_bytes);

        imageflow_response as *const JsonResponse
    }
}


#[test]
fn test_message() {
    exercise_json_message();
}

pub fn exercise_json_message() {
    unsafe {
        let c = imageflow_context_create();
        assert!(!c.is_null());

        let method_in = static_char!("brew_coffee");
        let json_in = "{}";
        let expected_response = c::JsonResponse::teapot();
        let expected_json_out = ::std::str::from_utf8(
            expected_response.response_json.as_ref()).unwrap();
        let expected_reponse_status = expected_response.status_code;

        let response = imageflow_send_json(c,
                                           None,
                                           method_in,
                                           json_in.as_ptr(),
                                           json_in.len());

        assert!(response != ptr::null());

        let mut json_out_ptr: *const u8 = ptr::null_mut();
        let mut json_out_size: usize = 0;
        let mut json_status_code: i64 = 0;

        assert!(imageflow_json_response_read(c,
                                             response,
                                             &mut json_status_code,
                                             &mut json_out_ptr,
                                             &mut json_out_size));


        let json_out_str =
        ::std::str::from_utf8(std::slice::from_raw_parts(json_out_ptr, json_out_size)).unwrap();
        assert_eq!(json_out_str, expected_json_out);

        assert_eq!(json_status_code, expected_reponse_status);

        imageflow_context_destroy(c);
    }
}

///
/// Creates an imageflow_io object to wrap a filename.
///
/// The filename should be a null-terminated string. It should be written in codepage used by your operating system for handling `fopen` calls.
/// https://msdn.microsoft.com/en-us/library/yeby3zcb.aspx
///
/// If the filename is fopen compatible, you're probably OK.
///
/// As always, `mode` is not enforced except for the file open flags.
///
#[no_mangle]
#[allow(unused_variables)]
pub unsafe extern "C" fn imageflow_io_create_for_file(context: *mut Context,
                                                      mode: IoMode,
                                                      filename: *const libc::c_char,
                                                      cleanup: CleanupWith)
                                                      -> *mut JobIo {
    let mut c = context_ready!(context);
    if filename.is_null() {
        c.outward_error_mut().try_set_error(nerror!(ErrorKind::NullArgument, "The argument 'filename' is null."));
        return ptr::null_mut();
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        let s = CStr::from_ptr(filename).to_str().unwrap();
        let result = c.create_io_from_filename_with_mode(s, std::mem::transmute(mode)).map_err(|e| e.at(here!()));

        result.map(|mut io| &mut *io as *mut JobIo)
    }));

    handle_result!(c, result, ptr::null_mut())
}


///
/// Creates an imageflow_io structure for reading from the provided buffer.
/// You are ALWAYS responsible for freeing the memory provided in accordance with the Lifetime value.
/// If you specify OutlivesFunctionCall, then the buffer will be copied.
///
///
#[no_mangle]
#[allow(unused_variables)]
pub unsafe extern "C" fn imageflow_io_create_from_buffer(context: *mut Context,
                                                         buffer: *const u8,
                                                         buffer_byte_count: libc::size_t,
                                                            lifetime: Lifetime,
                                                            cleanup: CleanupWith)
                                                         -> *mut JobIo {

    let mut c = context_ready!(context);
    if buffer.is_null() {
        c.outward_error_mut().try_set_error(nerror!(ErrorKind::NullArgument, "The argument 'buffer' is null."));
        return ptr::null_mut();
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        let bytes = std::slice::from_raw_parts(buffer, buffer_byte_count);

        let result = if lifetime == Lifetime::OutlivesFunctionCall {
            c.create_io_from_copy_of_slice(bytes).map_err(|e| e.at(here!()))
        }else {
            c.create_io_from_slice(bytes).map_err(|e| e.at(here!()))
        };
        result.map(|mut io| &mut *io as *mut JobIo)
    }));
    handle_result!(c, result, ptr::null_mut())
}


///
/// Creates an imageflow_io structure for writing to an expanding memory buffer.
///
/// Reads/seeks, are, in theory, supported, but unless you've written, there will be nothing to read.
///
/// The I/O structure and buffer will be freed with the context.
///
///
/// Returns null if allocation failed; check the context for error details.
#[no_mangle]
#[allow(unused_variables)]
pub unsafe extern "C" fn imageflow_io_create_for_output_buffer(context: *mut Context)
                                                               -> *mut JobIo {
    let mut c = context_ready!(context);
    let result = catch_unwind(AssertUnwindSafe(|| {
        c.create_io_output_buffer().map(|mut v| &mut *v as *mut JobIo).map_err(|e| e.at(here!()))
    }));
    handle_result!(c, result, ptr::null_mut())

}


// Returns false if the flow_io struct is disposed or not an output buffer type (or for any other error)
//

///
/// Provides access to the underlying buffer for the given imageflow_io object.
///
/// Ensure your length variable always holds 64-bits.
///
#[no_mangle]
pub unsafe extern "C" fn imageflow_io_get_output_buffer(context: *mut Context,
                                                        io: *mut JobIo,
                                                        result_buffer: *mut *const u8,
                                                        result_buffer_length: *mut libc::size_t)
                                                        -> bool {
    let mut c = context_ready!(context);
    if io.is_null() {
        c.outward_error_mut().try_set_error(nerror!(ErrorKind::NullArgument, "The argument 'io' is null."));
        return false;
    }
    if result_buffer.is_null() {
        c.outward_error_mut().try_set_error(nerror!(ErrorKind::NullArgument, "The argument 'result_buffer' is null."));
        return false;
    }

    if result_buffer_length.is_null() {
        c.outward_error_mut().try_set_error(nerror!(ErrorKind::NullArgument, "The argument 'result_buffer_length' is null."));
        return false;
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        c.get_proxy_mut_by_pointer(io).map_err(|e| e.at(here!())).and_then(|io_proxy| {
            let s= (&mut *io).get_output_buffer_bytes().map_err(|e| e.at(here!()))?;
            (*result_buffer) = s.as_ptr();
            (*result_buffer_length) = s.len();
            Ok(true)
        })
    }));
    handle_result!(c, result, false)
}

///
/// Provides access to the underlying buffer for the given imageflow_io object.
///
/// Ensure your length variable always holds 64-bits
///
#[no_mangle]
pub unsafe extern "C" fn imageflow_job_get_output_buffer_by_id(context: *mut Context,
                                                               job: *mut Job,
                                                               io_id: i32,
                                                               result_buffer: *mut *const u8,
                                                               result_buffer_length: *mut libc::size_t)
                                                               -> bool {
    let mut c = context_ready!(context);
    if job.is_null() {
        c.outward_error_mut().try_set_error(nerror!(ErrorKind::NullArgument, "The argument 'job' is null."));
        return false;
    }
    if result_buffer.is_null() {
        c.outward_error_mut().try_set_error(nerror!(ErrorKind::NullArgument, "The argument 'result_buffer' is null."));
        return false;
    }

    if result_buffer_length.is_null() {
        c.outward_error_mut().try_set_error(nerror!(ErrorKind::NullArgument, "The argument 'result_buffer_length' is null."));
        return false;
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        (&*job).get_io(io_id).map_err(|e| e.at(here!())).and_then(|io_proxy| {
            let s = io_proxy.get_output_buffer_bytes().map_err(|e| e.at(here!()))?;
            (*result_buffer) = s.as_ptr();
            (*result_buffer_length) = s.len();
            Ok(true)
        })
    }));
    handle_result!(c, result, false)
}


///
/// Creates an imageflow_job, which permits the association of imageflow_io instances with
/// numeric identifiers and provides a 'sub-context' for job execution
///
#[no_mangle]
pub unsafe extern "C" fn imageflow_job_create(context: *mut Context) -> *mut Job {
    let mut c = context_ready!(context);
    let result = catch_unwind(AssertUnwindSafe(|| {
        Ok(&mut *c.create_job() as *mut Job)
    }));
    handle_result!(c, result, ptr::null_mut())
}


///
/// Looks up the imageflow_io pointer from the provided io_id
///
#[no_mangle]
pub unsafe extern "C" fn imageflow_job_get_io(context: *mut Context,
                                              job: *mut Job,
                                              io_id: i32)
                                              -> *mut JobIo {
    let mut c = context_ready!(context);
    if job.is_null() {
        c.outward_error_mut().try_set_error(nerror!(ErrorKind::NullArgument, "The argument 'job' is null."));
        return ptr::null_mut();
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        (&*job).get_io(io_id)
            .map(|mut io| &mut *io as *mut JobIo).map_err(|e| e.at(here!()))

    }));
    handle_result!(c, result, ptr::null_mut())
}

///
/// Associates the imageflow_io object with the job and the assigned io_id.
///
/// The io_id will correspond with io_id in the graph
///
/// direction is in or out.
#[no_mangle]
pub unsafe extern "C" fn imageflow_job_add_io(context: *mut Context,
                                              job: *mut Job,
                                              io: *mut JobIo,
                                              io_id: i32,
                                              direction: Direction)
                                              -> bool {
    let mut c = context_ready!(context);
    if job.is_null() {
        c.outward_error_mut().try_set_error(nerror!(ErrorKind::NullArgument, "The argument 'job' is null."));
        return false;
    }
    if io.is_null() {
        c.outward_error_mut().try_set_error(nerror!(ErrorKind::NullArgument, "The argument 'io' is null."));
        return false;
    }
    let result = catch_unwind(AssertUnwindSafe(|| {

        (&mut *job).add_io(&mut *io, io_id, std::mem::transmute(direction))
            .map(|_| true).map_err(|e| e.at(here!()))

    }));
    handle_result!(c, result, false)
}

///
/// Destroys the provided imageflow_job
///
#[no_mangle]
pub unsafe extern "C" fn imageflow_job_destroy(context: *mut Context, job: *mut Job) -> bool {
    let mut c = context_ready!(context);
    let result = catch_unwind(AssertUnwindSafe(|| Ok(c.abi_try_remove_job(job))));
    handle_result!(c, result, false)
}


///
/// Allocates zeroed memory that will be freed with the context.
///
/// * filename/line may be used for debugging purposes. They are optional. Provide null/-1 to skip.
/// * `filename` should be an null-terminated UTF-8 or ASCII string which will outlive the context.
///
/// Returns null(0) on failure.
///
#[no_mangle]
pub unsafe extern "C" fn imageflow_context_memory_allocate(context: *mut Context,
                                                    bytes: libc::size_t,
                                                    filename: *const libc::c_char,
                                                    line: i32) -> *mut libc::c_void {

    let mut c = context_ready!(context);
    ffi::flow_context_calloc(c.flow_c(), 1, bytes, ptr::null(), c.flow_c() as *const libc::c_void, filename, line)
}

///
/// Frees memory allocated with imageflow_context_memory_allocate early.
///
/// * filename/line may be used for debugging purposes. They are optional. Provide null/-1 to skip.
/// * `filename` should be an null-terminated UTF-8 or ASCII string which will outlive the context.
///
/// Returns false on failure.
///
#[no_mangle]
pub unsafe extern "C" fn imageflow_context_memory_free(context: *mut Context,
                                                       pointer: *mut libc::c_void,
                                                       filename: *const libc::c_char,
                                                       line: i32) -> bool {
    let flow_c_ptr = if context.is_null(){
        ptr::null_mut()
    }else {
        (&mut *context).flow_c()
    };
    ffi::flow_destroy(flow_c_ptr, pointer, filename, line)
}

#[test]
fn test_allocate_free() {
    unsafe{
        let c = imageflow_context_create();
        let bytes = 100;
        let ptr = imageflow_context_memory_allocate(c, bytes, static_char!(file!()),
                                                    line!() as i32) as *mut u8;
        assert!(ptr != ptr::null_mut());

        for x in 0..bytes{
            assert_eq!(*ptr.offset(x as isize), 0);
        }
        assert!(imageflow_context_memory_free(c, ptr as *mut libc::c_void, static_char!(file!()),
                                              line!() as i32));

        imageflow_context_destroy(c);
        //imageflow_context_destroy(c);
    }
}
