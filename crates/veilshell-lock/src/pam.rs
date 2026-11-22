//! Isolated Linux-PAM, POSIX user lookup, local clock, and volatile zeroization FFI.
//!
//! This is the only module in `veilshell-lock` permitted to use `unsafe`.

#![allow(unsafe_code)]

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::Path;
use std::ptr;
use std::sync::atomic::{compiler_fence, Ordering};

use crate::secret::SecretPassword;

const PAM_SUCCESS: c_int = 0;
const PAM_CONV_ERR: c_int = 19;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_ERROR_MSG: c_int = 3;
const PAM_TEXT_INFO: c_int = 4;
const PAM_DISALLOW_NULL_AUTHTOK: c_int = 0x0001;

#[repr(C)]
struct PamHandle {
    _private: [u8; 0],
}

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

type PamConvFn = unsafe extern "C" fn(
    num_msg: c_int,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata_ptr: *mut c_void,
) -> c_int;

#[repr(C)]
struct PamConv {
    conv: Option<PamConvFn>,
    appdata_ptr: *mut c_void,
}

#[link(name = "pam")]
unsafe extern "C" {
    fn pam_start(
        service_name: *const c_char,
        user: *const c_char,
        pam_conversation: *const PamConv,
        pamh: *mut *mut PamHandle,
    ) -> c_int;
    fn pam_authenticate(pamh: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_acct_mgmt(pamh: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_end(pamh: *mut PamHandle, pam_status: c_int) -> c_int;
}

/// Outcome of a single PAM authentication + account validation attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthOutcome {
    /// Both `pam_authenticate` and `pam_acct_mgmt` returned `PAM_SUCCESS`.
    Authenticated,
    /// Credentials were rejected or the account is not permitted to unlock.
    Denied,
    /// PAM initialization, conversation, or unexpected error (fail-closed).
    Error,
}

/// Overwrite every byte in `buf` with zero using volatile stores so the compiler
/// cannot elide secret cleanup.
pub fn zeroize_bytes(buf: &mut [u8]) {
    for byte in buf {
        // SAFETY: `byte` is a valid, aligned, uniquely borrowed mutable reference.
        unsafe { ptr::write_volatile(byte, 0) };
    }
    compiler_fence(Ordering::SeqCst);
}

/// Resolve the current effective UID's login name from the system passwd database.
///
/// Never trusts `USER` or `LOGNAME` environment variables.
pub fn current_username() -> Result<String, String> {
    // SAFETY: `geteuid` has no preconditions and never fails.
    let uid = unsafe { libc::geteuid() };
    let mut buf_len = 1024_usize;
    loop {
        let mut pwd = std::mem::MaybeUninit::<libc::passwd>::zeroed();
        let mut buf = vec![0_u8; buf_len];
        let mut result: *mut libc::passwd = ptr::null_mut();
        // SAFETY: pointers are valid for the duration of the call; `buf` has length `buf_len`.
        let rc = unsafe {
            libc::getpwuid_r(
                uid,
                pwd.as_mut_ptr(),
                buf.as_mut_ptr().cast::<c_char>(),
                buf.len(),
                &raw mut result,
            )
        };
        if rc == libc::ERANGE && buf_len < 65_536 {
            buf_len *= 2;
            continue;
        }
        if rc != 0 || result.is_null() {
            return Err(format!("could not resolve account name for uid {uid}"));
        }
        // SAFETY: `getpwuid_r` succeeded with non-null `result` pointing to initialized `pwd`.
        let name_ptr = unsafe { (*result).pw_name };
        if name_ptr.is_null() {
            return Err(format!("passwd entry for uid {uid} has no username"));
        }
        // SAFETY: `pw_name` is a NUL-terminated C string residing inside `buf`.
        let name = unsafe { CStr::from_ptr(name_ptr) }
            .to_str()
            .map_err(|_| "account username is not valid UTF-8".to_owned())?;
        if name.is_empty() {
            return Err("account username is empty".to_owned());
        }
        return Ok(name.to_owned());
    }
}

/// Choose the default PAM service stack on this host.
#[must_use]
pub fn default_pam_service() -> &'static str {
    if Path::new("/etc/pam.d/veilshell-lock").is_file() {
        "veilshell-lock"
    } else if Path::new("/etc/pam.d/system-auth").is_file() {
        "system-auth"
    } else {
        "login"
    }
}

/// Format local wall-clock time as `("HH:MM", "Weekday, Month DD")`.
#[must_use]
pub fn local_clock_strings() -> (String, String) {
    const WEEKDAYS: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];

    // SAFETY: `time` and `localtime_r` operate on stack-allocated pods.
    let tm = unsafe {
        let mut now: libc::time_t = 0;
        libc::time(&raw mut now);
        let mut out = std::mem::MaybeUninit::<libc::tm>::zeroed();
        if libc::localtime_r(&raw const now, out.as_mut_ptr()).is_null() {
            return ("--:--".to_owned(), String::new());
        }
        out.assume_init()
    };

    let time = format!(
        "{:02}:{:02}",
        tm.tm_hour.clamp(0, 23),
        tm.tm_min.clamp(0, 59)
    );
    let wday = usize::try_from(tm.tm_wday)
        .ok()
        .and_then(|idx| WEEKDAYS.get(idx))
        .copied()
        .unwrap_or("");
    let month = usize::try_from(tm.tm_mon)
        .ok()
        .and_then(|idx| MONTHS.get(idx))
        .copied()
        .unwrap_or("");
    let date = if wday.is_empty() || month.is_empty() {
        String::new()
    } else {
        format!("{wday}, {month} {}", tm.tm_mday.clamp(1, 31))
    };
    (time, date)
}

struct ConvState<'a> {
    password: &'a [u8],
    consumed: bool,
}

unsafe extern "C" fn pam_conversation(
    num_msg: c_int,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata_ptr: *mut c_void,
) -> c_int {
    if num_msg <= 0 || msg.is_null() || resp.is_null() || appdata_ptr.is_null() {
        return PAM_CONV_ERR;
    }
    let Ok(count) = usize::try_from(num_msg) else {
        return PAM_CONV_ERR;
    };
    // SAFETY: `appdata_ptr` points to `ConvState` alive for the duration of `pam_authenticate`.
    let state = unsafe { &mut *appdata_ptr.cast::<ConvState<'_>>() };
    // SAFETY: `calloc` returns zero-initialized heap memory that Linux-PAM frees with `free`.
    let responses =
        unsafe { libc::calloc(count, std::mem::size_of::<PamResponse>()) }.cast::<PamResponse>();
    if responses.is_null() {
        return PAM_CONV_ERR;
    }

    for idx in 0..count {
        // SAFETY: `msg` points to `count` message pointers.
        let message_ptr = unsafe { *msg.add(idx) };
        if message_ptr.is_null() {
            free_partial_responses(responses, idx);
            return PAM_CONV_ERR;
        }
        // SAFETY: non-null PAM message pointer.
        let style = unsafe { (*message_ptr).msg_style };
        match style {
            PAM_PROMPT_ECHO_OFF => {
                if state.consumed || state.password.is_empty() || state.password.contains(&0) {
                    free_partial_responses(responses, idx);
                    return PAM_CONV_ERR;
                }
                let len = state.password.len();
                // SAFETY: allocate NUL-terminated buffer with `calloc` for PAM to `free`.
                let copy = unsafe { libc::calloc(len + 1, 1) }.cast::<u8>();
                if copy.is_null() {
                    free_partial_responses(responses, idx);
                    return PAM_CONV_ERR;
                }
                // SAFETY: `copy` has `len + 1` zeroed bytes; `state.password` has `len` bytes.
                unsafe {
                    ptr::copy_nonoverlapping(state.password.as_ptr(), copy, len);
                    (*responses.add(idx)).resp = copy.cast::<c_char>();
                    (*responses.add(idx)).resp_retcode = 0;
                }
                state.consumed = true;
            }
            PAM_TEXT_INFO | PAM_ERROR_MSG => {
                // Informational messages require no response text.
            }
            _ => {
                // Fail closed on PAM_PROMPT_ECHO_ON or any unexpected prompt style.
                free_partial_responses(responses, idx);
                return PAM_CONV_ERR;
            }
        }
    }

    // SAFETY: `resp` is non-null and receives ownership of the `calloc`'d array.
    unsafe {
        *resp = responses;
    }
    PAM_SUCCESS
}

fn free_partial_responses(responses: *mut PamResponse, count: usize) {
    for idx in 0..count {
        // SAFETY: `responses` has at least `count` entries allocated via `calloc`.
        let ptr = unsafe { (*responses.add(idx)).resp };
        if !ptr.is_null() {
            // SAFETY: `ptr` was allocated with `calloc` and is NUL-terminated.
            unsafe {
                let len = libc::strlen(ptr);
                zeroize_bytes(std::slice::from_raw_parts_mut(ptr.cast::<u8>(), len));
                libc::free(ptr.cast::<c_void>());
            }
        }
    }
    // SAFETY: `responses` was allocated via `calloc`.
    unsafe { libc::free(responses.cast::<c_void>()) };
}

/// Authenticate `username` against PAM `service`, requiring both
/// `pam_authenticate` and `pam_acct_mgmt` to succeed.
///
/// Always zeroizes `password` before returning.
#[must_use]
pub fn authenticate(service: &str, username: &str, password: &mut SecretPassword) -> AuthOutcome {
    let outcome = authenticate_inner(service, username, password);
    password.clear();
    outcome
}

fn authenticate_inner(service: &str, username: &str, password: &SecretPassword) -> AuthOutcome {
    if password.is_empty() || password.as_bytes().contains(&0) {
        return AuthOutcome::Denied;
    }
    let Ok(service_c) = CString::new(service) else {
        return AuthOutcome::Error;
    };
    let Ok(user_c) = CString::new(username) else {
        return AuthOutcome::Error;
    };

    let mut conv_state = ConvState {
        password: password.as_bytes(),
        consumed: false,
    };
    let conv = PamConv {
        conv: Some(pam_conversation),
        appdata_ptr: (&raw mut conv_state).cast::<c_void>(),
    };

    let mut handle: *mut PamHandle = ptr::null_mut();
    // SAFETY: `service_c`, `user_c`, and `conv` remain valid for the PAM transaction.
    let start_rc = unsafe {
        pam_start(
            service_c.as_ptr(),
            user_c.as_ptr(),
            &raw const conv,
            &raw mut handle,
        )
    };
    if start_rc != PAM_SUCCESS || handle.is_null() {
        return AuthOutcome::Error;
    }

    // SAFETY: `handle` was initialized by `pam_start`.
    let auth_rc = unsafe { pam_authenticate(handle, PAM_DISALLOW_NULL_AUTHTOK) };
    let final_rc = if auth_rc == PAM_SUCCESS {
        // Require account validity (`pam_acct_mgmt`) in addition to password verification.
        // SAFETY: `handle` is still valid.
        unsafe { pam_acct_mgmt(handle, PAM_DISALLOW_NULL_AUTHTOK) }
    } else {
        auth_rc
    };

    // SAFETY: close the PAM transaction cleanly.
    unsafe {
        pam_end(handle, final_rc);
    }

    if auth_rc == PAM_SUCCESS && final_rc == PAM_SUCCESS && conv_state.consumed {
        AuthOutcome::Authenticated
    } else if auth_rc == PAM_CONV_ERR {
        AuthOutcome::Error
    } else {
        AuthOutcome::Denied
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_current_username_and_clock() {
        let user = current_username().expect("effective uid should resolve");
        assert_ne!(user, "");
        let (time, _date) = local_clock_strings();
        assert_eq!(time.len(), 5);
        assert_eq!(&time[2..3], ":");
    }

    #[test]
    fn empty_password_is_denied_without_touching_pam() {
        let mut empty = SecretPassword::default();
        assert_eq!(
            authenticate(default_pam_service(), "root", &mut empty),
            AuthOutcome::Denied
        );
    }

    #[test]
    fn nul_byte_in_password_is_denied_and_zeroized() {
        let mut pw = SecretPassword::from_str_for_test("bad\0pass");
        assert_eq!(
            authenticate(default_pam_service(), "root", &mut pw),
            AuthOutcome::Denied
        );
        assert!(pw.is_empty());
    }
}
