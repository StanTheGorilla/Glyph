// glyph-asr-sidecar: a resident ASR engine process wrapping parakeet.dll (ggml/Vulkan).
//
// The stock parakeet-cli --stream only reads WAV files, so for live mic dictation we
// dlopen parakeet.dll and drive its cache-aware streaming C-API (ABI v5) ourselves.
//
// Protocol
//   stdin  : binary frames  [tag:u8][len:u32 LE][payload]
//              tag 1 AUDIO   payload = raw f32 LE PCM, 16 kHz mono
//              tag 2 END     finalize the current utterance (model stays loaded)
//              tag 3 QUIT    exit
//   stdout : one JSON object per line
//              {"type":"ready","abi":5}
//              {"type":"partial","text":"...","words":[...]}
//              {"type":"final","text":"...","eou":0,"eob":0}
//              {"type":"error","msg":"..."}
//   stderr : human diagnostics + ggml/Vulkan device logs (never on stdout)
//
// One-shot mode: --file <wav> transcribes a WAV via the offline path and exits.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::io::{Read, Write};
use std::path::PathBuf;

use libloading::{Library, Symbol};
use serde_json::{json, Value};

// ---- C-API function signatures (parakeet_capi.h, ABI v5) ----
type LoadFn = unsafe extern "C" fn(*const c_char) -> *mut c_void;
type FreeFn = unsafe extern "C" fn(*mut c_void);
type LastErrFn = unsafe extern "C" fn(*mut c_void) -> *const c_char;
type AbiFn = unsafe extern "C" fn() -> c_int;
type TranscribePathLangFn =
    unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *const c_char) -> *mut c_char;
type StreamBeginLangFn = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void;
type StreamFeedJsonFn = unsafe extern "C" fn(*mut c_void, *const f32, c_int) -> *mut c_char;
type StreamFinalizeJsonFn = unsafe extern "C" fn(*mut c_void) -> *mut c_char;
type StreamFreeFn = unsafe extern "C" fn(*mut c_void);
type FreeStringFn = unsafe extern "C" fn(*mut c_char);

struct Pk {
    _lib: Library, // kept alive; the fn pointers below borrow from it
    load: LoadFn,
    free: FreeFn,
    last_error: LastErrFn,
    abi_version: AbiFn,
    transcribe_path_lang: TranscribePathLangFn,
    stream_begin_lang: StreamBeginLangFn,
    stream_feed_json: StreamFeedJsonFn,
    stream_finalize_json: StreamFinalizeJsonFn,
    stream_free: StreamFreeFn,
    free_string: FreeStringFn,
}

impl Pk {
    unsafe fn load_from(dll: &PathBuf) -> Result<Pk, String> {
        let lib = Library::new(dll).map_err(|e| format!("load {}: {e}", dll.display()))?;
        unsafe fn sym<T: Copy>(lib: &Library, name: &[u8]) -> Result<T, String> {
            let s: Symbol<T> = lib
                .get(name)
                .map_err(|e| format!("symbol {}: {e}", String::from_utf8_lossy(name)))?;
            Ok(*s)
        }
        let pk = Pk {
            load: sym(&lib, b"parakeet_capi_load\0")?,
            free: sym(&lib, b"parakeet_capi_free\0")?,
            last_error: sym(&lib, b"parakeet_capi_last_error\0")?,
            abi_version: sym(&lib, b"parakeet_capi_abi_version\0")?,
            transcribe_path_lang: sym(&lib, b"parakeet_capi_transcribe_path_lang\0")?,
            stream_begin_lang: sym(&lib, b"parakeet_capi_stream_begin_lang\0")?,
            stream_feed_json: sym(&lib, b"parakeet_capi_stream_feed_json\0")?,
            stream_finalize_json: sym(&lib, b"parakeet_capi_stream_finalize_json\0")?,
            stream_free: sym(&lib, b"parakeet_capi_stream_free\0")?,
            free_string: sym(&lib, b"parakeet_capi_free_string\0")?,
            _lib: lib,
        };
        Ok(pk)
    }

    /// Take ownership of a malloc'd C string from the lib, copy to Rust, free it.
    unsafe fn take_string(&self, p: *mut c_char) -> Option<String> {
        if p.is_null() {
            return None;
        }
        let s = CStr::from_ptr(p).to_string_lossy().into_owned();
        (self.free_string)(p);
        Some(s)
    }
}

fn emit(v: Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

struct Args {
    model: PathBuf,
    dll: PathBuf,
    lang: String,
    device: Option<String>,
    file: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut model = None;
    let mut dll = None;
    let mut lang = "auto".to_string();
    let mut device = None;
    let mut file = None;
    let a: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < a.len() {
        let need = |i: usize| -> Result<String, String> {
            a.get(i + 1).cloned().ok_or_else(|| format!("missing value for {}", a[i]))
        };
        match a[i].as_str() {
            "--model" => model = Some(PathBuf::from(need(i)?)),
            "--dll" => dll = Some(PathBuf::from(need(i)?)),
            "--lang" => lang = need(i)?,
            "--device" => device = Some(need(i)?),
            "--file" => file = Some(PathBuf::from(need(i)?)),
            other => return Err(format!("unknown arg {other}")),
        }
        i += 2;
    }
    Ok(Args {
        model: model.ok_or("--model required")?,
        dll: dll.ok_or("--dll required")?,
        lang,
        device,
        file,
    })
}

fn main() {
    if let Err(e) = run() {
        emit(json!({"type":"error","msg":e}));
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    if let Some(dev) = &args.device {
        // PARAKEET_DEVICE selects the ggml backend: "Vulkan0", "cpu", "CUDA0", ...
        std::env::set_var("PARAKEET_DEVICE", dev);
    }

    let pk = unsafe { Pk::load_from(&args.dll)? };
    let abi = unsafe { (pk.abi_version)() };
    eprintln!("[sidecar] parakeet ABI v{abi}, device={:?}", args.device);

    let model_c = CString::new(args.model.to_string_lossy().as_bytes())
        .map_err(|_| "model path has NUL")?;
    let ctx = unsafe { (pk.load)(model_c.as_ptr()) };
    if ctx.is_null() {
        return Err(format!("load model failed: {}", args.model.display()));
    }
    let lang_c = CString::new(args.lang.as_bytes()).map_err(|_| "lang has NUL")?;

    // One-shot file mode.
    if let Some(wav) = &args.file {
        let wav_c =
            CString::new(wav.to_string_lossy().as_bytes()).map_err(|_| "wav path has NUL")?;
        let txt = unsafe {
            pk.take_string((pk.transcribe_path_lang)(ctx, wav_c.as_ptr(), 0, lang_c.as_ptr()))
        };
        match txt {
            Some(text) => emit(json!({"type":"final","text":text})),
            None => {
                let err = unsafe { CStr::from_ptr((pk.last_error)(ctx)).to_string_lossy().into_owned() };
                unsafe { (pk.free)(ctx) };
                return Err(format!("transcribe failed: {err}"));
            }
        }
        unsafe { (pk.free)(ctx) };
        return Ok(());
    }

    // Streaming mode.
    emit(json!({"type":"ready","abi":abi}));
    let mut stream: *mut c_void = std::ptr::null_mut();
    let mut stdin = std::io::stdin().lock();

    loop {
        let mut hdr = [0u8; 5];
        match stdin.read_exact(&mut hdr) {
            Ok(()) => {}
            Err(_) => break, // EOF / pipe closed
        }
        let tag = hdr[0];
        let len = u32::from_le_bytes([hdr[1], hdr[2], hdr[3], hdr[4]]) as usize;
        let mut payload = vec![0u8; len];
        if len > 0 && stdin.read_exact(&mut payload).is_err() {
            break;
        }

        match tag {
            1 => {
                // AUDIO: bytes -> f32 samples
                if stream.is_null() {
                    stream = unsafe { (pk.stream_begin_lang)(ctx, lang_c.as_ptr()) };
                    if stream.is_null() {
                        let err = unsafe {
                            CStr::from_ptr((pk.last_error)(ctx)).to_string_lossy().into_owned()
                        };
                        emit(json!({"type":"error","msg":format!("stream_begin: {err}")}));
                        continue;
                    }
                }
                let n = len / 4;
                let pcm: Vec<f32> = payload
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                let js = unsafe {
                    pk.take_string((pk.stream_feed_json)(stream, pcm.as_ptr(), n as c_int))
                };
                if let Some(js) = js {
                    forward_event("partial", &js);
                }
            }
            2 => {
                // END: finalize current utterance, keep model loaded
                if !stream.is_null() {
                    let js = unsafe { pk.take_string((pk.stream_finalize_json)(stream)) };
                    unsafe { (pk.stream_free)(stream) };
                    stream = std::ptr::null_mut();
                    match js {
                        Some(js) => forward_event("final", &js),
                        None => emit(json!({"type":"final","text":""})),
                    }
                } else {
                    emit(json!({"type":"final","text":""}));
                }
            }
            3 => break, // QUIT
            other => emit(json!({"type":"error","msg":format!("bad tag {other}")})),
        }
    }

    if !stream.is_null() {
        unsafe { (pk.stream_free)(stream) };
    }
    unsafe { (pk.free)(ctx) };
    Ok(())
}

/// Re-emit the lib's streaming JSON as one of our events, stamped with `kind`.
/// The lib returns {"text":..,"eou":..,"eob":..,"words":[..],...}. We pass through
/// text/eou/eob/words and add "type". Empty-text partials are suppressed.
fn forward_event(kind: &str, lib_json: &str) {
    let v: Value = match serde_json::from_str(lib_json) {
        Ok(v) => v,
        Err(_) => json!({ "text": lib_json }),
    };
    let text = v.get("text").and_then(|t| t.as_str()).unwrap_or("");
    if kind == "partial" && text.is_empty() {
        return; // nothing newly finalized this feed
    }
    let mut out = json!({ "type": kind, "text": text });
    for k in ["eou", "eob", "words"] {
        if let Some(val) = v.get(k) {
            out[k] = val.clone();
        }
    }
    emit(out);
}
