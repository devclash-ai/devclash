//! Background reads run in an isolated process: legacy Keychain UI policy is process-wide.
use core_foundation::{
    base::{CFType, TCFType},
    boolean::CFBoolean,
    data::CFData,
    dictionary::CFDictionary,
    string::{CFString, CFStringRef},
};
use security_framework_sys::{
    item::*,
    keychain::{SecKeychainFindGenericPassword, SecKeychainSetUserInteractionAllowed},
    keychain_item::{SecItemCopyMatching, SecKeychainItemFreeContent},
};
use std::{
    io::{IsTerminal, Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const FLAG: &str = "--devclash-read-provider-keychain";
const MAX_SECRET: usize = 1024 * 1024;
const HEADER: usize = 9;
const DEADLINE: Duration = Duration::from_secs(5);
const ACCESS: &str =
    "Keychain access is required. Open Usage Sources and authorize this provider, then Refresh.";
const FAILURE: &str =
    "The provider's Keychain login could not be read. Unlock your login Keychain, then Refresh.";
const INVALID: &str =
    "The provider's Keychain login is empty or unsupported. Sign in to the provider again.";
const TIMED_OUT: &str =
    "The provider's Keychain login could not be read in time. Unlock your Mac, then Refresh.";

#[link(name = "Security", kind = "framework")]
extern "C" {
    static kSecMatchLimitOne: CFStringRef;
}

fn permitted(service: &str, account: Option<&str>) -> bool {
    match (service, account) {
        ("Claude Safe Storage", Some("Claude Key"))
        | ("cursor-access-token", Some("cursor-user")) => true,
        ("Claude Code-credentials", None) => true,
        (service, None) => service
            .strip_prefix("Claude Code-credentials-")
            .is_some_and(|suffix| {
                suffix.len() == 8 && suffix.bytes().all(|b| b.is_ascii_hexdigit())
            }),
        ("Codex Auth", Some(account)) => account.strip_prefix("cli|").is_some_and(|suffix| {
            suffix.len() == 16 && suffix.bytes().all(|b| b.is_ascii_hexdigit())
        }),
        _ => false,
    }
}

/// Called before Tauri initializes. Invalid helper invocations exit without opening a GUI.
pub(super) fn handle_helper() -> bool {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if !args.iter().any(|arg| arg == FLAG) {
        return false;
    }
    if args.first().is_none_or(|arg| arg != FLAG) || !(2..=3).contains(&args.len()) {
        std::process::exit(2);
    }
    // The protocol carries a credential, so never emit it to an interactive terminal.
    if std::io::stdout().is_terminal() {
        std::process::exit(2);
    }
    let service = args[1].to_str();
    let account = args.get(2).and_then(|arg| arg.to_str());
    if service.is_none()
        || (args.len() == 3 && account.is_none())
        || !permitted(service.unwrap_or_default(), account)
    {
        std::process::exit(2);
    }
    let service = service.unwrap();
    // Only this short-lived helper changes legacy UI policy; the app and cloud Keychain do not.
    let mut length = 0u32;
    let mut data = std::ptr::null_mut();
    let status = unsafe {
        let ui_status = SecKeychainSetUserInteractionAllowed(0);
        if ui_status != 0 {
            ui_status
        } else {
            SecKeychainFindGenericPassword(
                std::ptr::null(),
                service.len() as u32,
                service.as_ptr().cast(),
                account.map_or(0, |value| value.len() as u32),
                account.map_or(std::ptr::null(), |value| value.as_ptr().cast()),
                &mut length,
                &mut data,
                std::ptr::null_mut(),
            )
        }
    };
    let content = NativeContent { data, length };
    let code = match status {
        0 if !data.is_null() && length > 0 && length as usize <= MAX_SECRET => 0,
        0 => 3,
        -25300 => 1,
        -25293 | -25308 | -128 => 2,
        _ => 4,
    };
    let payload = if code == 0 {
        unsafe { std::slice::from_raw_parts(content.data.cast::<u8>(), content.length as usize) }
    } else {
        &[]
    };
    let mut header = [0u8; HEADER];
    header[..4].copy_from_slice(b"DCK1");
    header[4] = code;
    header[5..].copy_from_slice(&(payload.len() as u32).to_be_bytes());
    let mut stdout = std::io::stdout().lock();
    let written = stdout
        .write_all(&header)
        .and_then(|_| stdout.write_all(payload))
        .and_then(|_| stdout.flush());
    drop(content);
    if written.is_err() {
        std::process::exit(1);
    }
    true
}

struct NativeContent {
    data: *mut std::ffi::c_void,
    length: u32,
}
impl Drop for NativeContent {
    fn drop(&mut self) {
        if !self.data.is_null() {
            unsafe {
                // Clear the externally allocated password before returning its buffer to Security.
                for offset in 0..self.length as usize {
                    self.data.cast::<u8>().add(offset).write_volatile(0);
                }
                SecKeychainItemFreeContent(std::ptr::null_mut(), self.data);
            }
        }
    }
}

fn helper_executable() -> Result<PathBuf, String> {
    // Test harness binaries do not run Tauri's helper dispatch. Production ignores this variable.
    #[cfg(test)]
    if let Some(path) = std::env::var_os("DEVCLASH_MAC_KEYCHAIN_HELPER") {
        return Ok(path.into());
    }
    std::env::current_exe().map_err(|_| FAILURE.into())
}

pub(super) fn password(
    service: &str,
    account: Option<&str>,
) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    if !permitted(service, account) {
        return Err(FAILURE.into());
    }
    let mut command = Command::new(helper_executable()?);
    command.arg(FLAG).arg(service);
    if let Some(account) = account {
        command.arg(account);
    }
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| FAILURE.to_owned())?;
    decode(collect_child(child, DEADLINE)?)
}

/// A concurrent bounded reader drains the pipe while the helper runs, avoiding pipe deadlocks.
fn collect_child(mut child: Child, timeout: Duration) -> Result<Zeroizing<Vec<u8>>, String> {
    let started = Instant::now();
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(FAILURE.into());
    };
    let (send, receive) = mpsc::channel();
    let reader = std::thread::Builder::new()
        .name("provider-keychain-pipe".into())
        .spawn(move || {
            // Reserve once so growing the buffer cannot leave freed copies of secret prefixes.
            let mut bytes = Zeroizing::new(Vec::with_capacity(MAX_SECRET + HEADER + 2));
            let result = stdout
                .take((MAX_SECRET + HEADER + 1) as u64)
                .read_to_end(&mut bytes);
            let result = if result.is_ok() && bytes.len() <= MAX_SECRET + HEADER {
                Ok(bytes)
            } else {
                Err(FAILURE.to_owned())
            };
            let _ = send.send(result);
        });
    if reader.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Err(FAILURE.into());
    }
    let mut output = None;
    let result = loop {
        if started.elapsed() >= timeout {
            break Err(TIMED_OUT.into());
        }
        if output.is_none() {
            match receive.try_recv() {
                Ok(Ok(bytes)) => output = Some(bytes),
                Ok(Err(error)) => break Err(error),
                Err(mpsc::TryRecvError::Disconnected) => break Err(FAILURE.into()),
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        match child.try_wait() {
            Ok(Some(status)) if !status.success() => break Err(FAILURE.into()),
            Ok(Some(_)) if output.is_some() => break Ok(output.take().unwrap()),
            Ok(_) => {}
            Err(_) => break Err(FAILURE.into()),
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // Reap on success and every error path, including deadlines and oversized output.
    if result.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    // A terminated helper closes the pipe; never wait indefinitely on inherited descriptors.
    if reader.as_ref().is_ok_and(|thread| thread.is_finished()) {
        let _ = reader.unwrap().join();
    }
    result
}

fn decode(mut bytes: Zeroizing<Vec<u8>>) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    if bytes.len() < HEADER || bytes.len() > MAX_SECRET + HEADER || &bytes[..4] != b"DCK1" {
        return Err(FAILURE.into());
    }
    let length = u32::from_be_bytes(bytes[5..9].try_into().unwrap()) as usize;
    if length > MAX_SECRET || bytes.len() != HEADER + length {
        return Err(FAILURE.into());
    }
    match bytes[4] {
        0 if length > 0 => {
            // Erase the removed tail too: Vec::truncate alone leaves duplicate secret bytes.
            bytes.copy_within(HEADER.., 0);
            bytes[length..].fill(0);
            bytes.truncate(length);
            Ok(Some(bytes))
        }
        1 if length == 0 => Ok(None),
        2 if length == 0 => Err(ACCESS.into()),
        3 if length == 0 => Err(INVALID.into()),
        _ => Err(FAILURE.into()),
    }
}

pub(super) fn read(
    service: &str,
    account: Option<&str>,
    allow_interaction: bool,
) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    if !allow_interaction {
        return password(service, account);
    }
    if !permitted(service, account) {
        return Err(FAILURE.into());
    }
    // Interactive reads only occur after an explicit user action. No global UI policy changes.
    let mut pairs = unsafe {
        vec![
            (
                CFString::wrap_under_get_rule(kSecClass),
                CFString::wrap_under_get_rule(kSecClassGenericPassword).into_CFType(),
            ),
            (
                CFString::wrap_under_get_rule(kSecAttrService),
                CFString::new(service).into_CFType(),
            ),
            (
                CFString::wrap_under_get_rule(kSecReturnData),
                CFBoolean::true_value().into_CFType(),
            ),
            (
                CFString::wrap_under_get_rule(kSecMatchLimit),
                CFString::wrap_under_get_rule(kSecMatchLimitOne).into_CFType(),
            ),
        ]
    };
    if let Some(account) = account {
        unsafe {
            pairs.push((
                CFString::wrap_under_get_rule(kSecAttrAccount),
                CFString::new(account).into_CFType(),
            ));
        }
    }
    let query = CFDictionary::from_CFType_pairs(&pairs);
    let mut result = std::ptr::null();
    let status = unsafe { SecItemCopyMatching(query.as_concrete_TypeRef(), &mut result) };
    let result = (!result.is_null()).then(|| unsafe { CFType::wrap_under_create_rule(result) });
    match status {
        -25300 => Ok(None),
        -25293 | -25308 | -128 => Err(ACCESS.into()),
        0 => {
            let data = result
                .and_then(|value| value.downcast::<CFData>())
                .ok_or(FAILURE)?;
            if data.len() == 0 || data.len() > MAX_SECRET as isize {
                return Err(INVALID.into());
            }
            Ok(Some(Zeroizing::new(data.bytes().to_vec())))
        }
        _ => Err(FAILURE.into()),
    }
}

#[cfg(test)]
mod tests;
