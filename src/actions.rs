use std::{
    ffi::{OsStr, c_void},
    os::windows::ffi::OsStrExt,
    path::PathBuf,
    process::Command,
    ptr::null_mut,
};

use anyhow::{Context, Result, ensure};
use url::Url;
use wiki::commands::{Action, Match};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
    System::Threading::CreateMutexW,
    UI::{
        Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
            SendInput, VK_MEDIA_PLAY_PAUSE, VK_MENU, VK_TAB,
        },
        Shell::ShellExecuteW,
        WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW, SW_SHOWNORMAL},
    },
};

pub fn perform(resolved: Match<'_>) -> Result<()> {
    match &resolved.command.action {
        Action::OpenChrome => open("chrome.exe", None).context("Could not open Google Chrome."),
        Action::OpenApp(path) => {
            Command::new(path)
                .spawn()
                .context("Could not start the app.")?;
            Ok(())
        }
        Action::OpenWebsite(url) => open(url, None).context("Could not open the website."),
        Action::MediaToggle => send_keys(&[
            (VK_MEDIA_PLAY_PAUSE, KEYEVENTF_EXTENDEDKEY),
            (VK_MEDIA_PLAY_PAUSE, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP),
        ]),
        Action::SwitchApps => send_keys(&[
            (VK_MENU, 0),
            (VK_TAB, 0),
            (VK_TAB, KEYEVENTF_KEYUP),
            (VK_MENU, KEYEVENTF_KEYUP),
        ]),
        Action::SearchChrome => {
            let url = Url::parse_with_params(
                "https://www.google.com/search",
                [("q", &resolved.argument)],
            )?;
            open("chrome.exe", Some(&format!("\"{url}\""))).context("Could not search in Chrome.")
        }
    }
}

fn open(target: &str, arguments: Option<&str>) -> Result<()> {
    let target = wide(target);
    let verb = wide("open");
    let arguments = arguments.map(wide);
    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            arguments
                .as_ref()
                .map_or(std::ptr::null(), |args| args.as_ptr()),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    ensure!(
        result as isize > 32,
        "Windows returned error {}.",
        result as isize
    );
    Ok(())
}

fn send_keys(keys: &[(u16, u32)]) -> Result<()> {
    let inputs: Vec<_> = keys
        .iter()
        .map(|&(key, flags)| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    dwFlags: flags,
                    ..Default::default()
                },
            },
        })
        .collect();
    let inserted = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    ensure!(
        inserted as usize == inputs.len(),
        "Windows rejected the keyboard input."
    );
    Ok(())
}

pub fn config_file() -> Result<PathBuf> {
    let root = std::env::var_os("LOCALAPPDATA").context("Windows did not provide LocalAppData.")?;
    Ok(PathBuf::from(root).join("Wiki").join("commands.json"))
}

pub struct Instance(HANDLE);

impl Instance {
    pub fn take() -> Result<Option<Self>> {
        let name = wide("Local\\WikiVoiceAssistant-0.1");
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        let error = unsafe { GetLastError() };
        ensure!(
            !handle.is_null(),
            "Could not start Wiki (Windows error {error})."
        );
        let instance = Self(handle);
        if error == ERROR_ALREADY_EXISTS {
            Ok(None)
        } else {
            Ok(Some(instance))
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

pub fn show_error(text: &str) {
    unsafe {
        MessageBoxW(
            null_mut::<c_void>(),
            wide(text).as_ptr(),
            wide("Wiki").as_ptr(),
            MB_OK | MB_ICONERROR,
        )
    };
}

fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().chain([0]).collect()
}
