//! Windows' share window (`DataTransferManager`) over an app window.
//!
//! Desktop apps have no CoreWindow, so the manager is reached for an HWND through
//! `IDataTransferManagerInterop`. Showing the window raises `DataRequested` on this thread,
//! where the share's title, text and files go into the data package; the package then says
//! whether the person picked a target (`ShareCompleted`) or closed the window
//! (`ShareCanceled`, Windows 10 2004 and later). Files are written to a folder of this
//! process under the temporary folder and handed over as read-only storage items; a share's
//! folder is removed when it is cancelled or fails, and the folders of earlier shares when
//! the next one starts or the bridge goes away, so a target reading a file late still finds
//! it while the share is the latest.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use remedy_ui_core::host::{
    HostEvent, ShareId, ShareOptions, ShareOutcome, ShareStarted, Unsupported, Waker,
};
use windows::ApplicationModel::DataTransfer::{
    DataPackage, DataRequestedEventArgs, DataTransferManager, ShareCompletedEventArgs,
};
use windows::Foundation::TypedEventHandler;
use windows::Storage::{IStorageItem, StorageFile};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::IDataTransferManagerInterop;
use windows::core::{HSTRING, IInspectable, Interface, Ref};
use windows_collections::IIterable;

const WHAT: &str = "sharing";

/// How shares ended, from the package's events (which Windows raises on this thread, but the
/// handlers it takes must be shareable).
#[derive(Default)]
struct Reporter {
    events: Mutex<Vec<HostEvent>>,
    reported: Mutex<HashSet<u64>>,
    waker: Mutex<Option<Waker>>,
    /// Each open share's folder, removed if it is cancelled or fails.
    folders: Mutex<HashMap<u64, PathBuf>>,
}

impl Reporter {
    /// Reports how share `id` ended, once.
    fn report(&self, id: u64, outcome: ShareOutcome) {
        if !self.reported.lock().unwrap_or_else(PoisonError::into_inner).insert(id) {
            return;
        }
        let folder = self.folders.lock().unwrap_or_else(PoisonError::into_inner).remove(&id);
        if !matches!(outcome, ShareOutcome::Shared)
            && let Some(f) = folder
        {
            let _ = std::fs::remove_dir_all(f);
        }
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(HostEvent::Share(ShareId(id), outcome));
        if let Some(w) = self.waker.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            w();
        }
    }
}

/// One window's share manager and the `DataRequested` handler serving its latest share.
struct Registration {
    manager: DataTransferManager,
    token: i64,
    share: u64,
}

/// The shares of this process.
pub(crate) struct Sharing {
    reporter: Arc<Reporter>,
    next: u64,
    windows: HashMap<isize, Registration>,
    /// `<temp>/remedy-ui-share/<process id>`.
    root: PathBuf,
}

impl Default for Sharing {
    fn default() -> Self {
        Sharing {
            reporter: Arc::default(),
            next: 1,
            windows: HashMap::new(),
            root: base().join(std::process::id().to_string()),
        }
    }
}

impl Drop for Sharing {
    fn drop(&mut self) {
        for (_, r) in self.windows.drain() {
            let _ = r.manager.RemoveDataRequested(r.token);
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Where every process's share folders live.
fn base() -> PathBuf {
    std::env::temp_dir().join("remedy-ui-share")
}

fn fail(reason: impl std::fmt::Display) -> Unsupported {
    Unsupported::new(WHAT, reason.to_string())
}

impl Sharing {
    /// Wakes the event loop when a share ends.
    pub(crate) fn set_waker(&mut self, waker: Waker) {
        *self.reporter.waker.lock().unwrap_or_else(PoisonError::into_inner) = Some(waker);
    }

    /// How shares ended since the last call.
    pub(crate) fn take_events(&mut self) -> Vec<HostEvent> {
        std::mem::take(&mut *self.reporter.events.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Forgets a closed window's manager.
    pub(crate) fn window_closed(&mut self, hwnd: isize) {
        if let Some(r) = self.windows.remove(&hwnd) {
            let _ = r.manager.RemoveDataRequested(r.token);
            self.reporter.report(r.share, ShareOutcome::Cancelled);
        }
    }

    /// Shows the share window over `hwnd` for `share`.
    pub(crate) fn share(&mut self, hwnd: isize, share: &ShareOptions) -> Result<ShareStarted, Unsupported> {
        let id = self.next;
        self.next += 1;
        // Earlier shares' files go now, except those of shares still open in other windows;
        // this one's stay until the next.
        let open: HashSet<String> = self
            .reporter
            .folders
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .map(u64::to_string)
            .collect();
        if let Ok(entries) = std::fs::read_dir(&self.root) {
            for e in entries.flatten() {
                if !e.file_name().to_str().is_some_and(|n| open.contains(n)) {
                    let _ = std::fs::remove_dir_all(e.path());
                }
            }
        }
        sweep_exited(&base());
        let folder = self.root.join(id.to_string());
        let paths =
            write_files(&folder, share).map_err(|e| fail(format!("the files could not be written: {e}")))?;
        let started = self.show(hwnd, id, share, paths);
        match started {
            Ok(()) => {
                self.reporter.folders.lock().unwrap_or_else(PoisonError::into_inner).insert(id, folder);
                Ok(ShareStarted::Pending(ShareId(id)))
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&folder);
                Err(e)
            }
        }
    }

    fn show(
        &mut self,
        hwnd: isize,
        id: u64,
        share: &ShareOptions,
        paths: Vec<PathBuf>,
    ) -> Result<(), Unsupported> {
        let interop = windows::core::factory::<DataTransferManager, IDataTransferManagerInterop>()
            .map_err(|e| fail(format!("Windows has no share window here: {e}")))?;
        let window = HWND(hwnd as *mut core::ffi::c_void);
        // SAFETY: `window` is a live top-level window of this thread (the caller's).
        let manager: DataTransferManager = unsafe { interop.GetForWindow(window) }
            .map_err(|e| fail(format!("no share manager for the window: {e}")))?;
        // A window serves one share at a time: an earlier one still open is replaced.
        if let Some(old) = self.windows.remove(&hwnd) {
            let _ = old.manager.RemoveDataRequested(old.token);
            self.reporter.report(old.share, ShareOutcome::Cancelled);
        }
        let content = Content { title: heading(share), text: share.text.clone(), paths };
        let reporter = self.reporter.clone();
        let handler = TypedEventHandler::<DataTransferManager, DataRequestedEventArgs>::new(
            move |_, args: Ref<'_, DataRequestedEventArgs>| {
                let Some(args) = args.as_ref() else { return Ok(()) };
                let request = args.Request()?;
                if let Err(e) = content.fill(&request.Data()?, id, &reporter) {
                    let reason = format!("the share could not be prepared: {e}");
                    let _ = request.FailWithDisplayText(&HSTRING::from(reason.as_str()));
                    reporter.report(id, ShareOutcome::Unsupported(fail(reason)));
                }
                Ok(())
            },
        );
        let token = manager.DataRequested(&handler).map_err(|e| fail(e.message()))?;
        // SAFETY: as above.
        if let Err(e) = unsafe { interop.ShowShareUIForWindow(window) } {
            let _ = manager.RemoveDataRequested(token);
            return Err(fail(format!("the share window did not open: {}", e.message())));
        }
        self.windows.insert(hwnd, Registration { manager, token, share: id });
        Ok(())
    }
}

/// What one share puts in the package.
struct Content {
    title: String,
    text: String,
    paths: Vec<PathBuf>,
}

impl Content {
    fn fill(&self, data: &DataPackage, id: u64, reporter: &Arc<Reporter>) -> windows::core::Result<()> {
        data.Properties()?.SetTitle(&HSTRING::from(self.title.as_str()))?;
        if !self.text.is_empty() {
            data.SetText(&HSTRING::from(self.text.as_str()))?;
        }
        if !self.paths.is_empty() {
            let mut items: Vec<Option<IStorageItem>> = Vec::with_capacity(self.paths.len());
            for p in &self.paths {
                // The lookup completes on the thread pool, so waiting here cannot deadlock.
                let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(p.as_path()))?.join()?;
                items.push(Some(file.cast::<IStorageItem>()?));
            }
            data.SetStorageItemsReadOnly(&IIterable::<IStorageItem>::from(items))?;
        }
        let done = reporter.clone();
        data.ShareCompleted(&TypedEventHandler::<DataPackage, ShareCompletedEventArgs>::new(move |_, _| {
            done.report(id, ShareOutcome::Shared);
            Ok(())
        }))?;
        let cancelled = reporter.clone();
        let cancel = data.ShareCanceled(&TypedEventHandler::<DataPackage, IInspectable>::new(move |_, _| {
            cancelled.report(id, ShareOutcome::Cancelled);
            Ok(())
        }));
        if cancel.is_err() {
            // Before Windows 10 2004 the package cannot say the window was closed: the share
            // counts as handed over, as on Android.
            reporter.report(id, ShareOutcome::Shared);
        }
        Ok(())
    }
}

/// The heading the share window shows (Windows refuses a package without one): the title,
/// else the first file's name, else the text.
fn heading(share: &ShareOptions) -> String {
    if !share.title.is_empty() {
        return share.title.clone();
    }
    match share.files.first() {
        Some(f) => f.safe_name(),
        None => share.text.lines().next().unwrap_or_default().to_owned(),
    }
}

/// Writes each file to its own folder under `folder` (two files may share a name); returns
/// their paths in order.
fn write_files(folder: &Path, share: &ShareOptions) -> std::io::Result<Vec<PathBuf>> {
    let mut paths = Vec::with_capacity(share.files.len());
    for (i, f) in share.files.iter().enumerate() {
        let dir = folder.join(i.to_string());
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(f.safe_name());
        std::fs::write(&path, &f.bytes)?;
        paths.push(path);
    }
    Ok(paths)
}

/// Removes the share folders of processes that have exited.
fn sweep_exited(base: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else { return };
    let own = std::process::id();
    for e in entries.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        if pid != own && !alive(pid) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// True while process `pid` runs (or cannot be asked about, so its folder is left alone).
fn alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, GetLastError};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    /// `GetExitCodeProcess`'s answer for a running process.
    const STILL_ACTIVE: u32 = 259;
    // SAFETY: the handle is checked, read and closed here.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            // No such process; anything else (access denied) may still be one.
            return GetLastError() != ERROR_INVALID_PARAMETER;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code) != 0;
        CloseHandle(h);
        !ok || code == STILL_ACTIVE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remedy_ui_core::host::ShareFile;

    #[test]
    fn files_get_their_own_folders_and_safe_names() {
        let dir = std::env::temp_dir().join(format!("remedy-ui-share-test-{}", std::process::id()));
        let share = ShareOptions::new()
            .file(ShareFile::new("a/b.json", "application/json", b"{}".to_vec()))
            .file(ShareFile::new("a/b.json", "application/json", b"[]".to_vec()));
        let paths = write_files(&dir, &share).unwrap_or_default();
        assert_eq!(paths, vec![dir.join("0").join("a_b.json"), dir.join("1").join("a_b.json")]);
        assert_eq!(paths.get(1).and_then(|p| std::fs::read(p).ok()).as_deref(), Some(&b"[]"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_window_always_has_a_heading() {
        let file = ShareFile::new("Lavender.alex-soap.json", "application/json", Vec::new());
        assert_eq!(heading(&ShareOptions::new().title("Soap").file(file.clone())), "Soap");
        assert_eq!(heading(&ShareOptions::new().text("hi").file(file)), "Lavender.alex-soap.json");
        assert_eq!(heading(&ShareOptions::new().text("first line\nsecond")), "first line");
    }

    #[test]
    fn only_folders_of_exited_processes_are_swept() {
        let base = std::env::temp_dir().join(format!("remedy-ui-share-sweep-{}", std::process::id()));
        let own = base.join(std::process::id().to_string());
        // Process ids are multiples of four on Windows; this one is never a process.
        let gone = base.join("4294967293");
        let other = base.join("not-a-process");
        for d in [&own, &gone, &other] {
            let _ = std::fs::create_dir_all(d);
        }
        sweep_exited(&base);
        assert!(own.exists() && other.exists());
        assert!(!gone.exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
