use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    path::Path,
    ptr::NonNull,
};

use anyhow::{Context, Result, ensure};
use libloading::os::windows::Library;
use serde::Deserialize;
use windows_sys::Win32::System::LibraryLoader::{
    LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
};

type Handle = NonNull<c_void>;
type Free = unsafe extern "C" fn(Handle);
type ReadTranscript = unsafe extern "C" fn(Handle) -> *const c_char;

struct Api {
    model_new: unsafe extern "C" fn(*const c_char) -> Option<Handle>,
    model_free: Free,
    model_find_word: unsafe extern "C" fn(Handle, *const c_char) -> c_int,
    recognizer_new: unsafe extern "C" fn(Handle, f32) -> Option<Handle>,
    recognizer_new_grm: unsafe extern "C" fn(Handle, f32, *const c_char) -> Option<Handle>,
    recognizer_free: Free,
    accept: unsafe extern "C" fn(Handle, *const i16, c_int) -> c_int,
    result: ReadTranscript,
    final_result: ReadTranscript,
    _library: Library,
}

impl Api {
    fn load(directory: &Path) -> Result<Self> {
        let file = directory.join("libvosk.dll");
        let library = unsafe {
            Library::load_with_flags(
                &file,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            )
        }
        .with_context(|| {
            format!(
                "Could not load {}. Keep Wiki's speech files beside wiki.exe.",
                file.display()
            )
        })?;
        // These signatures come from vosk_api.h in the pinned Windows release.
        unsafe {
            let log: unsafe extern "C" fn(c_int) = *library.get(b"vosk_set_log_level\0")?;
            log(-1);
            Ok(Self {
                model_new: *library.get(b"vosk_model_new\0")?,
                model_free: *library.get(b"vosk_model_free\0")?,
                model_find_word: *library.get(b"vosk_model_find_word\0")?,
                recognizer_new: *library.get(b"vosk_recognizer_new\0")?,
                recognizer_new_grm: *library.get(b"vosk_recognizer_new_grm\0")?,
                recognizer_free: *library.get(b"vosk_recognizer_free\0")?,
                accept: *library.get(b"vosk_recognizer_accept_waveform_s\0")?,
                result: *library.get(b"vosk_recognizer_result\0")?,
                final_result: *library.get(b"vosk_recognizer_final_result\0")?,
                _library: library,
            })
        }
    }
}

pub struct Model {
    api: Api,
    handle: Handle,
}

impl Model {
    pub fn load(directory: &Path) -> Result<Self> {
        let api = Api::load(directory)?;
        let path = directory.join("model");
        ensure!(
            path.join("am/final.mdl").is_file(),
            "Wiki's English speech model is missing. Keep the model folder beside wiki.exe."
        );
        let model_path = CString::new(path.to_str().context("The speech model path is invalid.")?)?;
        let handle = unsafe { (api.model_new)(model_path.as_ptr()) }
            .context("Could not load Wiki's English speech model.")?;
        Ok(Self { api, handle })
    }

    pub fn contains_word(&self, word: &str) -> bool {
        CString::new(word).is_ok_and(|word| unsafe {
            (self.api.model_find_word)(self.handle, word.as_ptr()) >= 0
        })
    }

    pub fn recognizer(&self, sample_rate: u32, grammar: Option<&CStr>) -> Result<Recognizer<'_>> {
        let handle = unsafe {
            if let Some(grammar) = grammar {
                (self.api.recognizer_new_grm)(self.handle, sample_rate as f32, grammar.as_ptr())
            } else {
                (self.api.recognizer_new)(self.handle, sample_rate as f32)
            }
        }
        .context("Could not initialize speech recognition.")?;
        Ok(Recognizer {
            api: &self.api,
            handle,
        })
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        unsafe { (self.api.model_free)(self.handle) };
    }
}

pub struct Recognizer<'a> {
    api: &'a Api,
    handle: Handle,
}

impl Recognizer<'_> {
    pub fn accept(&mut self, samples: &[i16]) -> Result<Option<String>> {
        let result =
            unsafe { (self.api.accept)(self.handle, samples.as_ptr(), samples.len().try_into()?) };
        ensure!(
            result >= 0,
            "Speech recognition could not read microphone audio."
        );
        if result == 0 {
            return Ok(None);
        }

        read_transcript(self.handle, self.api.result).map(Some)
    }

    pub fn finish(&mut self) -> Result<String> {
        read_transcript(self.handle, self.api.final_result)
    }
}

fn read_transcript(handle: Handle, result: ReadTranscript) -> Result<String> {
    #[derive(Deserialize)]
    struct Transcript {
        text: String,
    }

    let json = unsafe { result(handle) };
    ensure!(
        !json.is_null(),
        "Speech recognition did not return a transcript."
    );
    let text: Transcript = serde_json::from_slice(unsafe { CStr::from_ptr(json) }.to_bytes())?;
    Ok(text.text)
}

impl Drop for Recognizer<'_> {
    fn drop(&mut self) {
        unsafe { (self.api.recognizer_free)(self.handle) };
    }
}
