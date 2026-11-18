//! The Text Services Framework: the text store COM object TSF edits, and the thread manager,
//! document managers and contexts that connect windows to input methods. The editing logic
//! is the safe [`TextStore`]; this module only translates COM calls and locks to it.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;

use remedy_ui_host::{ImeEvent, TextInputState};
use windows::Win32::Foundation::{E_FAIL, E_INVALIDARG, E_NOTIMPL, HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, FORMATETC, IDataObject,
};
use windows::Win32::UI::TextServices::{
    CLSID_TF_ThreadMgr, ITextStoreACP, ITextStoreACP_Impl, ITextStoreACPSink, ITfCompositionView, ITfContext,
    ITfContextOwnerCompositionServices, ITfContextOwnerCompositionSink, ITfContextOwnerCompositionSink_Impl,
    ITfDocumentMgr, ITfRange, ITfRangeACP, ITfThreadMgr, TEXT_STORE_LOCK_FLAGS, TEXT_STORE_TEXT_CHANGE_FLAGS,
    TF_IAS_QUERYONLY, TS_AE_END, TS_AE_START, TS_ATTRVAL, TS_DEFAULT_SELECTION, TS_E_INVALIDPOINT,
    TS_E_INVALIDPOS, TS_E_NOLOCK, TS_E_SYNCHRONOUS, TS_LC_CHANGE, TS_LF_READ, TS_LF_READWRITE, TS_LF_SYNC,
    TS_RT_PLAIN, TS_RUNINFO, TS_S_ASYNC, TS_SELECTION_ACP, TS_SELECTIONSTYLE, TS_SS_NOHIDDENTEXT, TS_STATUS,
    TS_TEXTCHANGE,
};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows_core::{BOOL, GUID, HRESULT, IUnknown, Interface, PCWSTR, PWSTR, Ref, Result, implement};

use crate::text_store::{AppUpdate, StoreError, TextChange, TextStore};

fn store_error(e: StoreError) -> windows_core::Error {
    match e {
        StoreError::InvalidPosition => TS_E_INVALIDPOS.into(),
        StoreError::OutsideEditableArea => windows::Win32::UI::TextServices::TS_E_READONLY.into(),
    }
}

fn change(c: TextChange) -> TS_TEXTCHANGE {
    let i = |v: usize| i32::try_from(v).unwrap_or(i32::MAX);
    TS_TEXTCHANGE { acpStart: i(c.start), acpOldEnd: i(c.old_end), acpNewEnd: i(c.new_end) }
}

fn pos(v: i32) -> Result<usize> {
    usize::try_from(v).map_err(|_| TS_E_INVALIDPOS.into())
}

/// State shared by a window's COM text store and the bridge.
pub(crate) struct Shared {
    store: RefCell<TextStore>,
    sink: RefCell<Option<(ITextStoreACPSink, IUnknown)>>,
    /// Lock flags held right now (0 for none).
    lock: Cell<u32>,
    /// An asynchronous lock requested while another was held.
    queued: Cell<u32>,
    /// App changes to tell TSF once the current lock ends.
    pending: Cell<AppUpdate>,
    hwnd: isize,
    scale: Cell<f32>,
}

impl Shared {
    fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut c_void)
    }

    fn need(&self, flags: TEXT_STORE_LOCK_FLAGS) -> Result<()> {
        if self.lock.get() & flags.0 == flags.0 { Ok(()) } else { Err(TS_E_NOLOCK.into()) }
    }

    /// Tells TSF what the app changed, now or when the current lock ends.
    fn notify(&self, up: AppUpdate) {
        if self.lock.get() != 0 {
            let p = self.pending.get();
            self.pending.set(AppUpdate {
                text: up.text.or(p.text),
                selection: up.selection || p.selection,
                layout: up.layout || p.layout,
            });
            return;
        }
        let sink = self.sink.borrow().as_ref().map(|(s, _)| s.clone());
        let Some(sink) = sink else { return };
        // SAFETY: plain COM calls on a live sink, on the thread that owns it.
        unsafe {
            if let Some(c) = up.text {
                let _ = sink.OnTextChange(TEXT_STORE_TEXT_CHANGE_FLAGS(0), &change(c));
            }
            if up.selection {
                let _ = sink.OnSelectionChange();
            }
            if up.layout {
                let _ = sink.OnLayoutChange(TS_LC_CHANGE, 0);
            }
        }
    }

    /// The caret rectangle in screen pixels.
    fn caret_on_screen(&self) -> RECT {
        let c = self.store.borrow().caret();
        let s = self.scale.get();
        let mut a = POINT { x: (c.min.x * s).floor() as i32, y: (c.min.y * s).floor() as i32 };
        let mut b = POINT { x: (c.max.x * s).ceil() as i32, y: (c.max.y * s).ceil() as i32 };
        // SAFETY: the window is alive while its store is; the points are valid out-params.
        unsafe {
            let _ = ClientToScreen(self.hwnd(), &mut a);
            let _ = ClientToScreen(self.hwnd(), &mut b);
        }
        RECT { left: a.x, top: a.y, right: b.x.max(a.x + 1), bottom: b.y }
    }
}

#[implement(ITextStoreACP, ITfContextOwnerCompositionSink)]
struct StoreCom(Rc<Shared>);

/// The ACP range a TSF range covers.
fn extent(range: &ITfRange) -> Result<std::ops::Range<usize>> {
    let acp: ITfRangeACP = range.cast()?;
    let (mut start, mut len) = (0, 0);
    // SAFETY: two valid out-params.
    unsafe { acp.GetExtent(&mut start, &mut len)? };
    let start = pos(start)?;
    Ok(start..start + pos(len)?)
}

impl ITfContextOwnerCompositionSink_Impl for StoreCom_Impl {
    fn OnStartComposition(&self, composition: Ref<'_, ITfCompositionView>) -> Result<BOOL> {
        let view = composition.ok()?;
        // SAFETY: a COM call on a live view.
        let r = extent(&unsafe { view.GetRange()? })?;
        self.0.store.borrow_mut().start_composition(r).map_err(store_error)?;
        Ok(true.into())
    }

    fn OnUpdateComposition(
        &self,
        composition: Ref<'_, ITfCompositionView>,
        range: Ref<'_, ITfRange>,
    ) -> Result<()> {
        let r = match range.ok() {
            Ok(range) => extent(range)?,
            // SAFETY: a COM call on a live view.
            Err(_) => extent(&unsafe { composition.ok()?.GetRange()? })?,
        };
        self.0.store.borrow_mut().update_composition(r).map_err(store_error)
    }

    fn OnEndComposition(&self, _composition: Ref<'_, ITfCompositionView>) -> Result<()> {
        self.0.store.borrow_mut().end_composition();
        Ok(())
    }
}

impl ITextStoreACP_Impl for StoreCom_Impl {
    fn AdviseSink(&self, riid: *const GUID, punk: Ref<'_, IUnknown>, _mask: u32) -> Result<()> {
        // SAFETY: TSF passes a valid IID pointer.
        if unsafe { riid.as_ref() } != Some(&ITextStoreACPSink::IID) {
            return Err(E_INVALIDARG.into());
        }
        let unk = punk.ok()?.clone();
        let identity: IUnknown = unk.cast()?;
        let mut sink = self.0.sink.borrow_mut();
        match sink.as_ref() {
            Some((_, id)) if *id == identity => Ok(()),
            Some(_) => Err(windows::Win32::System::Ole::CONNECT_E_ADVISELIMIT.into()),
            None => {
                *sink = Some((unk.cast()?, identity));
                Ok(())
            }
        }
    }

    fn UnadviseSink(&self, punk: Ref<'_, IUnknown>) -> Result<()> {
        let identity: IUnknown = punk.ok()?.cast()?;
        let mut sink = self.0.sink.borrow_mut();
        match sink.as_ref() {
            Some((_, id)) if *id == identity => {
                *sink = None;
                Ok(())
            }
            _ => Err(windows::Win32::System::Ole::CONNECT_E_NOCONNECTION.into()),
        }
    }

    fn RequestLock(&self, flags: u32) -> Result<HRESULT> {
        let shared = &self.0;
        let want = flags & TS_LF_READWRITE.0;
        if shared.lock.get() != 0 {
            if flags & TS_LF_SYNC != 0 {
                return Ok(TS_E_SYNCHRONOUS);
            }
            shared.queued.set(shared.queued.get() | want);
            return Ok(TS_S_ASYNC);
        }
        let sink =
            shared.sink.borrow().as_ref().map(|(s, _)| s.clone()).ok_or(windows_core::Error::from(E_FAIL))?;
        let grant = |f: u32| -> HRESULT {
            shared.lock.set(f);
            // SAFETY: a COM call on the advised sink; it calls back into this store, which
            // holds no borrow across the call.
            let hr = unsafe { sink.OnLockGranted(TEXT_STORE_LOCK_FLAGS(f)) }
                .map_or_else(|e| e.code(), |()| HRESULT(0));
            shared.lock.set(0);
            hr
        };
        let hr = grant(want);
        // Locks requested asynchronously during this one run now, in order.
        loop {
            let q = shared.queued.replace(0);
            if q == 0 {
                break;
            }
            let _ = grant(q);
        }
        shared.notify(shared.pending.take());
        Ok(hr)
    }

    fn GetStatus(&self) -> Result<TS_STATUS> {
        Ok(TS_STATUS { dwDynamicFlags: 0, dwStaticFlags: TS_SS_NOHIDDENTEXT })
    }

    fn QueryInsert(&self, start: i32, end: i32, _cch: u32, rstart: *mut i32, rend: *mut i32) -> Result<()> {
        let len = self.0.store.borrow().text().len();
        let (s, e) = (pos(start)?, pos(end)?);
        if s > e || e > len {
            return Err(E_INVALIDARG.into());
        }
        // SAFETY: TSF passes valid out-params.
        unsafe {
            *rstart = start;
            *rend = end;
        }
        Ok(())
    }

    fn GetSelection(
        &self,
        index: u32,
        count: u32,
        sel: *mut TS_SELECTION_ACP,
        fetched: *mut u32,
    ) -> Result<()> {
        self.0.need(TS_LF_READ)?;
        if index != TS_DEFAULT_SELECTION && index != 0 {
            return Err(E_INVALIDARG.into());
        }
        if count == 0 || sel.is_null() || fetched.is_null() {
            return Err(E_INVALIDARG.into());
        }
        let (r, start_active) = self.0.store.borrow().selection();
        let i = |v: usize| i32::try_from(v).unwrap_or(i32::MAX);
        // SAFETY: TSF provides room for `count` (at least one) selections and a counter.
        unsafe {
            *sel = TS_SELECTION_ACP {
                acpStart: i(r.start),
                acpEnd: i(r.end),
                style: TS_SELECTIONSTYLE {
                    ase: if start_active { TS_AE_START } else { TS_AE_END },
                    fInterimChar: false.into(),
                },
            };
            *fetched = 1;
        }
        Ok(())
    }

    fn SetSelection(&self, count: u32, sel: *const TS_SELECTION_ACP) -> Result<()> {
        self.0.need(TS_LF_READWRITE)?;
        if count == 0 || sel.is_null() {
            return Err(E_INVALIDARG.into());
        }
        // SAFETY: TSF passes `count` (at least one) selections.
        let s = unsafe { *sel };
        let r = pos(s.acpStart)?..pos(s.acpEnd)?;
        self.0.store.borrow_mut().set_selection(r, s.style.ase == TS_AE_START).map_err(store_error)
    }

    fn GetText(
        &self,
        start: i32,
        end: i32,
        plain: PWSTR,
        plain_req: u32,
        plain_ret: *mut u32,
        runs: *mut TS_RUNINFO,
        runs_req: u32,
        runs_ret: *mut u32,
        next: *mut i32,
    ) -> Result<()> {
        self.0.need(TS_LF_READ)?;
        let store = self.0.store.borrow();
        let text = store.text();
        let s = pos(start)?;
        let e = if end == -1 { text.len() } else { pos(end)? };
        if s > e || e > text.len() {
            return Err(TS_E_INVALIDPOS.into());
        }
        let n = (e - s).min(plain_req as usize);
        let part = text.get(s..s + n).unwrap_or_default();
        // SAFETY: TSF provides `plain_req` units at `plain` and `runs_req` run infos at `runs`;
        // at most that many are written. Counters are valid out-params.
        unsafe {
            if !plain.is_null() && n > 0 {
                std::ptr::copy_nonoverlapping(part.as_ptr(), plain.0, n);
            }
            *plain_ret = n as u32;
            if !runs.is_null() && runs_req > 0 && n > 0 {
                *runs = TS_RUNINFO { uCount: n as u32, r#type: TS_RT_PLAIN };
                *runs_ret = 1;
            } else {
                *runs_ret = 0;
            }
            *next = i32::try_from(s + n).unwrap_or(i32::MAX);
        }
        Ok(())
    }

    fn SetText(&self, _flags: u32, start: i32, end: i32, text: &PCWSTR, cch: u32) -> Result<TS_TEXTCHANGE> {
        self.0.need(TS_LF_READWRITE)?;
        // SAFETY: TSF passes `cch` UTF-16 units at `text`.
        let units =
            if cch == 0 { &[][..] } else { unsafe { std::slice::from_raw_parts(text.0, cch as usize) } };
        let c = self.0.store.borrow_mut().set_text(pos(start)?..pos(end)?, units).map_err(store_error)?;
        Ok(change(c))
    }

    fn GetFormattedText(&self, _start: i32, _end: i32) -> Result<IDataObject> {
        Err(E_NOTIMPL.into())
    }

    fn GetEmbedded(&self, _pos: i32, _service: *const GUID, _riid: *const GUID) -> Result<IUnknown> {
        Err(E_NOTIMPL.into())
    }

    fn QueryInsertEmbedded(&self, _service: *const GUID, _format: *const FORMATETC) -> Result<BOOL> {
        // Plain-text fields hold no embedded objects.
        Ok(false.into())
    }

    fn InsertEmbedded(
        &self,
        _flags: u32,
        _start: i32,
        _end: i32,
        _data: Ref<'_, IDataObject>,
    ) -> Result<TS_TEXTCHANGE> {
        Err(E_NOTIMPL.into())
    }

    fn InsertTextAtSelection(
        &self,
        flags: u32,
        text: &PCWSTR,
        cch: u32,
        rstart: *mut i32,
        rend: *mut i32,
        changed: *mut TS_TEXTCHANGE,
    ) -> Result<()> {
        let query = flags & TF_IAS_QUERYONLY.0 != 0;
        self.0.need(if query { TS_LF_READ } else { TS_LF_READWRITE })?;
        // SAFETY: TSF passes `cch` UTF-16 units at `text`.
        let units =
            if cch == 0 { &[][..] } else { unsafe { std::slice::from_raw_parts(text.0, cch as usize) } };
        let (r, c) = self.0.store.borrow_mut().insert_at_selection(units, query).map_err(store_error)?;
        let i = |v: usize| i32::try_from(v).unwrap_or(i32::MAX);
        // SAFETY: optional out-params, written only when present.
        unsafe {
            if !rstart.is_null() {
                *rstart = i(r.start);
            }
            if !rend.is_null() {
                *rend = i(r.end);
            }
            if let (false, Some(c)) = (changed.is_null(), c) {
                *changed = change(c);
            }
        }
        Ok(())
    }

    fn InsertEmbeddedAtSelection(
        &self,
        _flags: u32,
        _data: Ref<'_, IDataObject>,
        _start: *mut i32,
        _end: *mut i32,
        _changed: *mut TS_TEXTCHANGE,
    ) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn RequestSupportedAttrs(&self, _flags: u32, _count: u32, _attrs: *const GUID) -> Result<()> {
        // No text attributes are exposed; RetrieveRequestedAttrs then returns none.
        Ok(())
    }

    fn RequestAttrsAtPosition(&self, _pos: i32, _count: u32, _attrs: *const GUID, _flags: u32) -> Result<()> {
        Ok(())
    }

    fn RequestAttrsTransitioningAtPosition(
        &self,
        _pos: i32,
        _count: u32,
        _attrs: *const GUID,
        _flags: u32,
    ) -> Result<()> {
        Ok(())
    }

    fn FindNextAttrTransition(
        &self,
        _start: i32,
        halt: i32,
        _count: u32,
        _attrs: *const GUID,
        _flags: u32,
        next: *mut i32,
        found: *mut BOOL,
        offset: *mut i32,
    ) -> Result<()> {
        // SAFETY: valid out-params.
        unsafe {
            *next = halt;
            *found = false.into();
            *offset = 0;
        }
        Ok(())
    }

    fn RetrieveRequestedAttrs(&self, _count: u32, _vals: *mut TS_ATTRVAL, fetched: *mut u32) -> Result<()> {
        // SAFETY: a valid out-param.
        unsafe { *fetched = 0 };
        Ok(())
    }

    fn GetEndACP(&self) -> Result<i32> {
        self.0.need(TS_LF_READ)?;
        Ok(i32::try_from(self.0.store.borrow().text().len()).unwrap_or(i32::MAX))
    }

    fn GetActiveView(&self) -> Result<u32> {
        Ok(0)
    }

    fn GetACPFromPoint(&self, _view: u32, _point: *const POINT, _flags: u32) -> Result<i32> {
        // Hit testing lives in the app's text layout, which the store does not have.
        Err(TS_E_INVALIDPOINT.into())
    }

    fn GetTextExt(
        &self,
        _view: u32,
        _start: i32,
        _end: i32,
        rect: *mut RECT,
        clipped: *mut BOOL,
    ) -> Result<()> {
        self.0.need(TS_LF_READ)?;
        // Candidate windows anchor to the caret the app reported.
        let r = self.0.caret_on_screen();
        // SAFETY: valid out-params.
        unsafe {
            *rect = r;
            *clipped = false.into();
        }
        Ok(())
    }

    fn GetScreenExt(&self, _view: u32) -> Result<RECT> {
        let mut r = RECT::default();
        // SAFETY: a live window and valid out-params.
        unsafe {
            GetClientRect(self.0.hwnd(), &mut r)?;
            let mut a = POINT { x: r.left, y: r.top };
            let mut b = POINT { x: r.right, y: r.bottom };
            let _ = ClientToScreen(self.0.hwnd(), &mut a);
            let _ = ClientToScreen(self.0.hwnd(), &mut b);
            Ok(RECT { left: a.x, top: a.y, right: b.x, bottom: b.y })
        }
    }

    fn GetWnd(&self, _view: u32) -> Result<HWND> {
        Ok(self.0.hwnd())
    }
}

struct Doc {
    docmgr: ITfDocumentMgr,
    context: ITfContext,
    shared: Rc<Shared>,
    _store: ITextStoreACP,
}

/// TSF for one UI thread: one document per window with a text field focused.
pub(crate) struct Tsf {
    thread: ITfThreadMgr,
    client: u32,
    docs: HashMap<isize, Doc>,
}

impl Tsf {
    /// Activates TSF on this thread.
    pub(crate) fn new() -> Result<Tsf> {
        // SAFETY: COM initialization on this (UI) thread; "already initialized" results are
        // fine, a different apartment model is not, and surfaces as the CoCreateInstance error.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let thread: ITfThreadMgr = CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)?;
            let client = thread.Activate()?;
            Ok(Tsf { thread, client, docs: HashMap::new() })
        }
    }

    fn doc(&mut self, hwnd: isize) -> Result<&Doc> {
        if !self.docs.contains_key(&hwnd) {
            let shared = Rc::new(Shared {
                store: RefCell::new(TextStore::new()),
                sink: RefCell::new(None),
                lock: Cell::new(0),
                queued: Cell::new(0),
                pending: Cell::new(AppUpdate::default()),
                hwnd,
                scale: Cell::new(1.0),
            });
            let store: ITextStoreACP = StoreCom(shared.clone()).into();
            // SAFETY: COM calls on live objects; out-params are valid.
            let (docmgr, context) = unsafe {
                let docmgr = self.thread.CreateDocumentMgr()?;
                let mut context = None;
                let mut cookie = 0;
                docmgr.CreateContext(self.client, 0, &store, &mut context, &mut cookie)?;
                let context = context.ok_or(windows_core::Error::from(E_FAIL))?;
                docmgr.Push(&context)?;
                (docmgr, context)
            };
            self.docs.insert(hwnd, Doc { docmgr, context, shared, _store: store });
        }
        self.docs.get(&hwnd).ok_or(E_FAIL.into())
    }

    /// Starts or updates the session for a window's focused field.
    pub(crate) fn focus(&mut self, hwnd: isize, scale: f32, state: &TextInputState) -> Result<()> {
        let thread = self.thread.clone();
        let doc = self.doc(hwnd)?;
        doc.shared.scale.set(scale);
        let up = doc.shared.store.borrow_mut().set_from_app(state);
        doc.shared.notify(up);
        // SAFETY: COM calls on live objects.
        unsafe {
            // Returns the previously associated document, null the first time (which the
            // binding reports as an error with S_OK); either way the association is made.
            let _ = thread.AssociateFocus(HWND(hwnd as *mut c_void), &doc.docmgr);
            thread.SetFocus(&doc.docmgr)?;
        }
        Ok(())
    }

    /// Ends the session: a running composition is committed, and the window stops talking
    /// to input methods.
    pub(crate) fn blur(&mut self, hwnd: isize) {
        let Some(doc) = self.docs.get(&hwnd) else { return };
        // SAFETY: COM calls on live objects.
        unsafe {
            if doc.shared.store.borrow().composition().is_some()
                && let Ok(services) = doc.context.cast::<ITfContextOwnerCompositionServices>()
            {
                let _ = services.TerminateComposition(None);
            }
            let _ = self.thread.AssociateFocus(HWND(hwnd as *mut c_void), None);
        }
    }

    /// Releases a closing window's document.
    pub(crate) fn close(&mut self, hwnd: isize) {
        self.blur(hwnd);
        if let Some(doc) = self.docs.remove(&hwnd) {
            // SAFETY: a COM call on a live document manager.
            let _ = unsafe { doc.docmgr.Pop(windows::Win32::UI::TextServices::TF_POPF_ALL) };
        }
    }

    /// Events for the app since the last call.
    pub(crate) fn take_events(&mut self) -> Vec<(isize, ImeEvent)> {
        let mut out = Vec::new();
        for (hwnd, doc) in &self.docs {
            out.extend(doc.shared.store.borrow_mut().take_events().into_iter().map(|e| (*hwnd, e)));
        }
        out
    }
}

impl Drop for Tsf {
    fn drop(&mut self) {
        let hwnds: Vec<isize> = self.docs.keys().copied().collect();
        for h in hwnds {
            self.close(h);
        }
        // SAFETY: deactivating the thread manager this object activated.
        let _ = unsafe { self.thread.Deactivate() };
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use super::*;
    use remedy_ui_core::Rect;
    use windows::Win32::UI::TextServices::ITextStoreACPSink_Impl;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WINDOW_STYLE,
    };
    use windows_core::w;

    /// What the test sink saw, in order.
    type Log = Rc<RefCell<Vec<String>>>;
    /// One scripted edit session.
    type Step = Box<dyn Fn(&ITextStoreACP, &Log)>;

    /// A stand-in for TSF's own sink: inside each granted lock it runs the next scripted step
    /// against the store, the way an input method's edit session does.
    #[implement(ITextStoreACPSink)]
    struct Sink {
        store: RefCell<Option<ITextStoreACP>>,
        steps: RefCell<Vec<Step>>,
        log: Log,
    }

    impl ITextStoreACPSink_Impl for Sink_Impl {
        fn OnTextChange(&self, _flags: TEXT_STORE_TEXT_CHANGE_FLAGS, c: *const TS_TEXTCHANGE) -> Result<()> {
            let c = unsafe { *c };
            self.log.borrow_mut().push(format!("text {}..{}->{}", c.acpStart, c.acpOldEnd, c.acpNewEnd));
            Ok(())
        }
        fn OnSelectionChange(&self) -> Result<()> {
            self.log.borrow_mut().push("selection".into());
            Ok(())
        }
        fn OnLayoutChange(
            &self,
            _code: windows::Win32::UI::TextServices::TsLayoutCode,
            _view: u32,
        ) -> Result<()> {
            self.log.borrow_mut().push("layout".into());
            Ok(())
        }
        fn OnStatusChange(&self, _flags: u32) -> Result<()> {
            Ok(())
        }
        fn OnAttrsChange(&self, _s: i32, _e: i32, _n: u32, _a: *const GUID) -> Result<()> {
            Ok(())
        }
        fn OnLockGranted(&self, flags: TEXT_STORE_LOCK_FLAGS) -> Result<()> {
            self.log.borrow_mut().push(format!("lock {}", flags.0));
            let store = self.store.borrow().clone().unwrap();
            let step = self.steps.borrow_mut().remove(0);
            step(&store, &self.log);
            Ok(())
        }
        fn OnStartEditTransaction(&self) -> Result<()> {
            Ok(())
        }
        fn OnEndEditTransaction(&self) -> Result<()> {
            Ok(())
        }
    }

    fn text_of(store: &ITextStoreACP) -> String {
        let mut buf = [0u16; 64];
        let (mut got, mut runs_got, mut next) = (0u32, 0u32, 0i32);
        let mut run = TS_RUNINFO::default();
        unsafe {
            store
                .GetText(0, -1, &mut buf, &mut got, std::slice::from_mut(&mut run), &mut runs_got, &mut next)
                .unwrap();
        }
        String::from_utf16_lossy(&buf[..got as usize])
    }

    fn shared(hwnd: isize) -> Rc<Shared> {
        Rc::new(Shared {
            store: RefCell::new(TextStore::new()),
            sink: RefCell::new(None),
            lock: Cell::new(0),
            queued: Cell::new(0),
            pending: Cell::new(AppUpdate::default()),
            hwnd,
            scale: Cell::new(1.0),
        })
    }

    fn field(text: &str, caret: usize) -> TextInputState {
        {
            let mut v_ = TextInputState::default();
            v_.surrounding = text.into();
            v_.selection = caret..caret;
            v_.caret = Rect::new(5.0, 5.0, 6.0, 20.0);
            v_.private = false;
            v_.multiline = false;
            v_
        }
    }

    #[test]
    fn the_com_store_honors_locks_and_edits_like_tsf_expects() {
        let sh = shared(0);
        sh.store.borrow_mut().set_from_app(&field("Hi ", 3));
        let store: ITextStoreACP = StoreCom(sh.clone()).into();
        let log: Log = Rc::default();
        let steps: Vec<Step> = vec![
            // A read-write session: read, try a nested sync lock, queue an async one, insert.
            Box::new(|s, log| {
                log.borrow_mut().push(format!("read {}", text_of(s)));
                let nested = unsafe { s.RequestLock(TS_LF_READ.0 | TS_LF_SYNC) }.unwrap();
                log.borrow_mut().push(format!("nested sync {}", nested == TS_E_SYNCHRONOUS));
                let queued = unsafe { s.RequestLock(TS_LF_READ.0) }.unwrap();
                log.borrow_mut().push(format!("queued async {}", queued == TS_S_ASYNC));
                let text: Vec<u16> = "there".encode_utf16().collect();
                let (mut a, mut b) = (0, 0);
                let mut ch = TS_TEXTCHANGE::default();
                unsafe { s.InsertTextAtSelection(0, &text, &mut a, &mut b, &mut ch) }.unwrap();
                log.borrow_mut().push(format!("inserted {a}..{b}"));
            }),
            // The queued read lock runs after the first ends.
            Box::new(|s, log| log.borrow_mut().push(format!("read {}", text_of(s)))),
        ];
        let sink: ITextStoreACPSink =
            Sink { store: RefCell::new(Some(store.clone())), steps: RefCell::new(steps), log: log.clone() }
                .into();
        unsafe {
            store.AdviseSink(&ITextStoreACPSink::IID, &sink, 0).unwrap();
            assert!(store.AdviseSink(&ITextStoreACPSink::IID, &store, 0).is_err(), "one sink at a time");
            // Outside a lock, reads are refused.
            let mut buf = [0u16; 4];
            let (mut g, mut rg, mut n) = (0, 0, 0);
            let mut run = TS_RUNINFO::default();
            let e = store
                .GetText(0, -1, &mut buf, &mut g, std::slice::from_mut(&mut run), &mut rg, &mut n)
                .unwrap_err();
            assert_eq!(e.code(), TS_E_NOLOCK);
            let hr = store.RequestLock(TS_LF_READWRITE.0 | TS_LF_SYNC).unwrap();
            assert_eq!(hr, HRESULT(0));
        }
        assert_eq!(
            *log.borrow(),
            vec![
                "lock 6",
                "read Hi ",
                "nested sync true",
                "queued async true",
                "inserted 3..8",
                "lock 2",
                "read Hi there"
            ]
        );
        assert_eq!(sh.store.borrow_mut().take_events(), vec![ImeEvent::Commit("there".into())]);

        // An app change outside a lock reaches TSF right away.
        log.borrow_mut().clear();
        let up = sh.store.borrow_mut().set_from_app(&field("Hi there!", 9));
        sh.notify(up);
        assert_eq!(*log.borrow(), vec!["text 8..8->9", "selection"]);
        unsafe { store.UnadviseSink(&sink).unwrap() };
        assert!(sh.sink.borrow().is_none());
    }

    #[test]
    fn tsf_connects_to_the_store_on_a_real_window() {
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                100,
                40,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap();
        let h = hwnd.0 as isize;
        let mut tsf = Tsf::new().unwrap();
        tsf.focus(h, 1.0, &field("abc", 3)).unwrap();
        let doc = tsf.docs.get(&h).unwrap();
        assert!(doc.shared.sink.borrow().is_some(), "TSF advised its sink on our store");
        tsf.focus(h, 1.0, &field("abcd", 4)).unwrap();
        tsf.blur(h);
        tsf.close(h);
        assert!(tsf.docs.is_empty());
        drop(tsf);
        unsafe { DestroyWindow(hwnd).unwrap() };
    }
}
