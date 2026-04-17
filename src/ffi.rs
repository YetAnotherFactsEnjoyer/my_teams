use std::{
    ffi::{CString, c_char},
    sync::atomic::{AtomicBool, Ordering},
};

pub static RUNNING: AtomicBool = AtomicBool::new(true);

extern "C" fn handle_sigint(_sig: i32) {
    RUNNING.store(false, Ordering::SeqCst);
}

unsafe extern "C" {
    fn signal(sig: i32, handler: extern "C" fn(i32)) -> usize;
}

pub fn setup_signal_handler() {
    unsafe {
        signal(2, handle_sigint);
    }
}

unsafe extern "C" {
    pub fn server_event_team_created(
        team_uuid: *const c_char,
        team_name: *const c_char,
        user_uuid: *const c_char,
    ) -> i32;
    pub fn server_event_user_loaded(user_uuid: *const c_char, user_name: *const c_char) -> i32;
    pub fn server_event_user_created(user_uuid: *const c_char, user_name: *const c_char) -> i32;
    pub fn server_event_user_logged_in(user_uuid: *const c_char) -> i32;
    pub fn server_event_user_logged_out(user_uuid: *const c_char) -> i32;
    pub fn server_event_private_message_sended(
        s_uuid: *const c_char,
        r_uuid: *const c_char,
        c_body: *const c_char,
    ) -> i32;
}

pub fn call_user_loaded(uuid: &str, name: &str) {
    if let (Ok(c_uuid), Ok(c_name)) = (CString::new(uuid), CString::new(name)) {
        unsafe { server_event_user_loaded(c_uuid.as_ptr(), c_name.as_ptr()) };
    }
}

pub fn call_user_logged_in(uuid: &str) {
    if let Ok(c_uuid) = CString::new(uuid) {
        unsafe { server_event_user_logged_in(c_uuid.as_ptr()) };
    }
}

pub fn call_user_created(uuid: &str, name: &str) {
    if let (Ok(c_uuid), Ok(c_name)) = (CString::new(uuid), CString::new(name)) {
        unsafe { crate::ffi::server_event_user_created(c_uuid.as_ptr(), c_name.as_ptr()) };
    }
}

pub fn call_user_logged_out(uuid: &str) {
    if let Ok(c_uuid) = CString::new(uuid) {
        unsafe { crate::ffi::server_event_user_logged_out(c_uuid.as_ptr()) };
    }
}

pub fn call_private_message_sended(sender_uuid: &str, receiver_uuid: &str, body: &str) {
    if let (Ok(s_uuid), Ok(r_uuid), Ok(c_body)) = (
        CString::new(sender_uuid),
        CString::new(receiver_uuid),
        CString::new(body),
    ) {
        unsafe {
            crate::ffi::server_event_private_message_sended(
                s_uuid.as_ptr(),
                r_uuid.as_ptr(),
                c_body.as_ptr(),
            )
        };
    }
}
