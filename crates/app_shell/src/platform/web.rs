//! A browser page's files: the ones its user picked, held in memory under
//! a path of their own; the ones the app keeps (documents, autosaved
//! copies), under [`KEPT`], held in memory and in the page's database
//! (IndexedDB), read back before the app starts; and downloads for
//! anything else the app writes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;

/// Where picked files appear to live: a folder no desktop path starts with.
const ROOT: &str = "/browser";

/// The folder of the files the page keeps between visits.
pub(crate) const KEPT: &str = "/printcad";

#[wasm_bindgen(inline_js = r#"
const DB = "printcad-files";
const STORE = "files";

function open() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB, 1);
    request.onupgradeneeded = () => request.result.createObjectStore(STORE, { keyPath: "path" });
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

async function within(mode, work) {
  const db = await open();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, mode);
    const request = work(tx.objectStore(STORE));
    tx.oncomplete = () => resolve(request.result);
    tx.onerror = () => reject(tx.error);
  });
}

export function kept_files() {
  return within("readonly", (store) => store.getAll());
}

export function keep_file(path, bytes) {
  return within("readwrite", (store) => store.put({ path, bytes }));
}

export function drop_file(path) {
  return within("readwrite", (store) => store.delete(path));
}
"#)]
extern "C" {
    fn kept_files() -> js_sys::Promise;
    fn keep_file(path: &str, bytes: &js_sys::Uint8Array) -> js_sys::Promise;
    fn drop_file(path: &str) -> js_sys::Promise;
}

thread_local! {
    /// Every file picked this session, by the path it was given.
    static FILES: RefCell<HashMap<PathBuf, Vec<u8>>> = RefCell::new(HashMap::new());
}

/// Hold `bytes` as the file `name`; the path it is read back by.
pub(crate) fn keep(name: &str, bytes: Vec<u8>) -> PathBuf {
    let path = Path::new(ROOT).join(name);
    FILES.with(|files| files.borrow_mut().insert(path.clone(), bytes));
    path
}

/// Read the files the page kept back into memory; before the app starts,
/// so its reads find them at once.
pub(crate) async fn load_kept() {
    let kept = match wasm_bindgen_futures::JsFuture::from(kept_files()).await {
        Ok(kept) => kept,
        Err(e) => {
            tracing::warn!("the page's kept files did not read: {e:?}");
            return;
        }
    };
    FILES.with(|files| {
        let mut files = files.borrow_mut();
        for record in js_sys::Array::from(&kept).iter() {
            let field = |key: &str| js_sys::Reflect::get(&record, &key.into()).unwrap_or_default();
            if let Some(path) = field("path").as_string() {
                let bytes = js_sys::Uint8Array::new(&field("bytes")).to_vec();
                files.insert(PathBuf::from(path), bytes);
            }
        }
    });
}

/// Whether `path` is one the page keeps between visits.
pub(crate) fn is_kept(path: &Path) -> bool {
    path.starts_with(KEPT)
}

/// Keep `bytes` as the file at `path`, under [`KEPT`]: at once in memory,
/// and in the page's database as soon as it takes them.
pub(crate) fn keep_at(path: &Path, bytes: &[u8]) {
    keep_owned(path, bytes.to_vec());
}

/// [`keep_at`], taking the bytes: the page's copy is these, the database's
/// the one the browser makes.
pub(crate) fn keep_owned(path: &Path, bytes: Vec<u8>) {
    let written = keep_file(
        &path.to_string_lossy(),
        &js_sys::Uint8Array::from(&bytes[..]),
    );
    FILES.with(|files| files.borrow_mut().insert(path.to_path_buf(), bytes));
    let path = path.to_path_buf();
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = wasm_bindgen_futures::JsFuture::from(written).await {
            crate::log_panel::warn(format!("The page could not keep {}: {e:?}", path.display()));
        }
    });
}

/// Take the file at `path` away.
pub(crate) fn remove(path: &Path) {
    FILES.with(|files| files.borrow_mut().remove(path));
    if is_kept(path) {
        let dropped = drop_file(&path.to_string_lossy());
        wasm_bindgen_futures::spawn_local(async move {
            let _ = wasm_bindgen_futures::JsFuture::from(dropped).await;
        });
    }
}

/// The files directly in folder `dir`.
pub(crate) fn list(dir: &Path) -> Vec<PathBuf> {
    FILES.with(|files| {
        files
            .borrow()
            .keys()
            .filter(|path| path.parent() == Some(dir))
            .cloned()
            .collect()
    })
}

/// The size of the file at `path`, without copying it.
pub(crate) fn size(path: &Path) -> Option<u64> {
    FILES.with(|files| files.borrow().get(path).map(|bytes| bytes.len() as u64))
}

/// Run `look` on the bytes of the file at `path` where they lie: a large
/// document is several hundred megabytes, which a copy would double.
pub(crate) fn with_bytes<R>(path: &Path, look: impl FnOnce(&[u8]) -> R) -> Option<R> {
    FILES.with(|files| files.borrow().get(path).map(|bytes| look(bytes)))
}

/// Whether there is a file at `path`.
pub(crate) fn exists(path: &Path) -> bool {
    FILES.with(|files| files.borrow().contains_key(path))
}

thread_local! {
    /// Whether a document has edits not saved, as the app last said.
    static UNSAVED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Ask before the page is closed or left while a document has edits not
/// saved: the browser's own question, which is all a page may ask then.
pub(crate) fn guard_unsaved() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let ask = Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(
        |event: web_sys::BeforeUnloadEvent| {
            if UNSAVED.with(std::cell::Cell::get) {
                event.prevent_default();
                event.set_return_value("You have unsaved changes");
            }
        },
    );
    let _ = window.add_event_listener_with_callback("beforeunload", ask.as_ref().unchecked_ref());
    // The page keeps the handler for its whole life.
    ask.forget();
}

pub(crate) fn set_unsaved(any: bool) {
    UNSAVED.with(|unsaved| unsaved.set(any));
}

/// The bytes of a picked file.
pub(crate) fn read(path: &Path) -> std::io::Result<Vec<u8>> {
    FILES
        .with(|files| files.borrow().get(path).cloned())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{} was not picked in this page", path.display()),
            )
        })
}

fn js_error(what: &str, error: wasm_bindgen::JsValue) -> std::io::Error {
    std::io::Error::other(format!("{what}: {error:?}"))
}

/// Hand `bytes` to the browser as a download named after `path`'s file
/// name. The file a later read asks for by the same path is these bytes.
pub(crate) fn download(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "printcad-file".to_string());
    FILES.with(|files| {
        files
            .borrow_mut()
            .insert(path.to_path_buf(), bytes.to_vec())
    });
    let window = web_sys::window().ok_or_else(|| std::io::Error::other("no window"))?;
    let document = window
        .document()
        .ok_or_else(|| std::io::Error::other("no document"))?;
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("application/octet-stream");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)
        .map_err(|e| js_error("could not make the file", e))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)
        .map_err(|e| js_error("could not offer the file", e))?;
    let link: web_sys::HtmlAnchorElement = document
        .create_element("a")
        .map_err(|e| js_error("could not offer the file", e))?
        .unchecked_into();
    link.set_href(&url);
    link.set_download(&name);
    link.click();
    // The browser has started the download; the address may go.
    let _ = web_sys::Url::revoke_object_url(&url);
    Ok(())
}

/// Ask the user for files through the page's own picker: `accept` lists
/// the extensions offered (`.step,.stp`), `many` allows several. `done`
/// gets the paths they are held under once read, or nothing when the
/// picker was closed.
pub(crate) fn pick(accept: &str, many: bool, done: impl FnOnce(Vec<PathBuf>) + 'static) {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        done(Vec::new());
        return;
    };
    let Ok(input) = document.create_element("input") else {
        done(Vec::new());
        return;
    };
    let input: web_sys::HtmlInputElement = input.unchecked_into();
    input.set_type("file");
    input.set_accept(accept);
    input.set_multiple(many);

    // The picker answers once: with files (`change`) or without (`cancel`).
    let done = std::rc::Rc::new(RefCell::new(Some(done)));
    let finish = {
        let done = done.clone();
        move |paths: Vec<PathBuf>| {
            if let Some(done) = done.borrow_mut().take() {
                done(paths);
            }
        }
    };
    let on_change = {
        let input = input.clone();
        let finish = finish.clone();
        Closure::<dyn FnMut()>::new(move || {
            let files = input.files();
            let finish = finish.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let mut paths = Vec::new();
                if let Some(files) = files {
                    for i in 0..files.length() {
                        let Some(file) = files.get(i) else { continue };
                        let Ok(buffer) =
                            wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await
                        else {
                            tracing::warn!("could not read {}", file.name());
                            continue;
                        };
                        let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
                        paths.push(keep(&file.name(), bytes));
                    }
                }
                finish(paths);
            });
        })
    };
    let on_cancel = Closure::<dyn FnMut()>::new(move || finish(Vec::new()));
    input.set_onchange(Some(on_change.as_ref().unchecked_ref()));
    let _ = input.add_event_listener_with_callback("cancel", on_cancel.as_ref().unchecked_ref());
    // The page keeps the handlers for as long as the input can call them.
    on_change.forget();
    on_cancel.forget();
    input.click();
}

/// Hold `bytes` as the file at `path`: a picked file handed to a kernel
/// worker, which has a store of its own.
pub(crate) fn put(path: &Path, bytes: Vec<u8>) {
    FILES.with(|files| files.borrow_mut().insert(path.to_path_buf(), bytes));
}
