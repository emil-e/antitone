//! AppKit's deferred termination decision without replacing winit's delegate.
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::OnceLock,
};

use objc2::{
    ClassType, MainThreadMarker, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, ClassBuilder, Imp, Method, NSObject, ProtocolObject, Sel},
    sel,
};
use objc2_app_kit::{NSApplication, NSApplicationDelegate, NSApplicationTerminateReply};
use objc2_foundation::{NSArray, NSObjectNSDelayedPerforming, NSRunLoopCommonModes};

thread_local! {
    static ACTIVE: RefCell<Option<Rc<Request>>> = const { RefCell::new(None) };
    static NATIVE_MODAL_DEPTH: Cell<usize> = const { Cell::new(0) };
}

pub(crate) struct NativeModal {
    _private: (),
}

pub(crate) fn native_modal() -> NativeModal {
    NATIVE_MODAL_DEPTH.with(|depth| depth.set(depth.get() + 1));
    NativeModal { _private: () }
}

impl Drop for NativeModal {
    fn drop(&mut self) {
        NATIVE_MODAL_DEPTH.with(|depth| depth.set(depth.get() - 1));
    }
}

struct Request {
    app: Retained<NSApplication>,
    delegate: Retained<ProtocolObject<dyn NSApplicationDelegate>>,
    notify: Box<dyn Fn() -> bool>,
    pending: Cell<bool>,
    delivered: Cell<bool>,
    prepared: Cell<bool>,
}

impl Request {
    fn reply(&self, allow: bool) {
        if self.pending.replace(false) {
            self.delivered.set(false);
            self.prepared.set(false);
            if allow {
                // AppKit may exit here without running the executable's stack destructors.
                log::info!(
                    target: "antitone::logging",
                    reason = "native_quit";
                    "session_finished"
                );
                log::logger().flush();
            }
            self.app.replyToApplicationShouldTerminate(allow);
        }
    }
}

/// Release AppKit's deferred wait before winit catches a handler panic and
/// tries to stop its outer run loop; otherwise the modal termination loop stays alive.
pub(crate) struct CancelOnUnwind(Option<Rc<Request>>);

pub(crate) fn cancel_on_unwind() -> CancelOnUnwind {
    CancelOnUnwind(ACTIVE.with(|active| active.borrow().clone()))
}

impl Drop for CancelOnUnwind {
    fn drop(&mut self) {
        if std::thread::panicking()
            && let Some(request) = &self.0
        {
            request.reply(false);
        }
    }
}

extern "C-unwind" fn should_terminate(
    delegate: *mut AnyObject,
    _selector: Sel,
    app: *mut NSApplication,
) -> NSApplicationTerminateReply {
    let request = ACTIVE.with(|active| active.borrow().clone());
    let Some(request) = request else {
        return NSApplicationTerminateReply::TerminateNow;
    };
    let registered: &AnyObject = (*request.delegate).as_ref();
    if !std::ptr::eq(delegate, registered) || !std::ptr::eq(app, &*request.app) {
        return NSApplicationTerminateReply::TerminateNow;
    }
    if !request.pending.get() && NATIVE_MODAL_DEPTH.with(|depth| depth.get() > 0) {
        // A synchronous file panel holds winit's handler borrow. Do not start a
        // second deferred modal wait whose editor decision cannot run until it returns.
        return NSApplicationTerminateReply::TerminateCancel;
    }
    if !request.pending.replace(true) {
        request.delivered.set(false);
        request.prepared.set(false);
        if !(request.notify)() {
            request.pending.set(false);
            return NSApplicationTerminateReply::TerminateCancel;
        }
    }
    NSApplicationTerminateReply::TerminateLater
}

extern "C-unwind" fn complete_termination(_: *mut AnyObject, _: Sel, _: *mut AnyObject) {
    let request = ACTIVE.with(|active| active.borrow().clone());
    if let Some(request) = request
        && request.prepared.get()
    {
        request.reply(true);
    }
}

type Terminate = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject);
static ORIGINAL_TERMINATE: OnceLock<Terminate> = OnceLock::new();

extern "C-unwind" fn terminate(app: *mut AnyObject, selector: Sel, sender: *mut AnyObject) {
    let request = ACTIVE.with(|active| active.borrow().clone());
    if let Some(request) = request
        && request.pending.get()
        && std::ptr::eq(app, Retained::as_ptr(&request.app).cast())
    {
        // AppKit can bypass applicationShouldTerminate on a second terminate:
        // during its deferred wait. Coalesce at entry, before that native path.
        return;
    }
    let original = ORIGINAL_TERMINATE
        .get()
        .expect("installed native termination entry");
    // EntryGate verifies the native void(id, SEL, id) ABI before installing us.
    // Forward the actual receiver, selector and nullable sender unchanged.
    unsafe { original(app, selector, sender) };
}

struct EntryGate {
    method: &'static Method,
    original: Imp,
}

impl EntryGate {
    fn install(app: &NSApplication) -> Result<Self, Box<dyn std::error::Error>> {
        let object: &AnyObject = app.as_ref();
        let class = object.class();
        let selector = sel!(terminate:);
        class.verify_sel::<(*mut AnyObject,), ()>(selector)?;
        let method = class
            .instance_method(selector)
            .ok_or("native termination entry is missing")?;
        let original = method.implementation();
        // Objective-C encodings were verified above, including the nullable object
        // sender. The IMP and typed C-unwind function pointer have the same ABI.
        let function = unsafe { std::mem::transmute::<Imp, Terminate>(original) };
        let saved = ORIGINAL_TERMINATE.get_or_init(|| function);
        if !std::ptr::fn_addr_eq(*saved, function) {
            return Err("native termination entry changed; revisit termination integration".into());
        }
        let replacement = methods()
            .instance_method(selector)
            .unwrap()
            .implementation();
        // Only the main thread installs this gate. Keep the original implementation
        // for forwarding/restoration; no delegate callbacks or object layouts change.
        let previous = unsafe { method.set_implementation(replacement) };
        if !std::ptr::fn_addr_eq(previous, original) {
            unsafe { method.set_implementation(previous) };
            return Err("native termination entry changed during installation".into());
        }
        Ok(Self { method, original })
    }
}

impl Drop for EntryGate {
    fn drop(&mut self) {
        let replacement = methods()
            .instance_method(sel!(terminate:))
            .unwrap()
            .implementation();
        if std::ptr::fn_addr_eq(self.method.implementation(), replacement) {
            // Restore only our own gate; never overwrite another integration.
            unsafe { self.method.set_implementation(self.original) };
        }
    }
}

fn methods() -> &'static AnyClass {
    static METHODS: OnceLock<&'static AnyClass> = OnceLock::new();
    METHODS.get_or_init(|| {
        let mut builder = ClassBuilder::new(c"AntitoneTerminationMethods", NSObject::class())
            .expect("unique Antitone termination method class");
        // AppKit's optional delegate method takes self, _cmd and one nonnull
        // NSApplication, returning its NSUInteger-backed termination reply.
        unsafe {
            builder.add_method(
                sel!(applicationShouldTerminate:),
                should_terminate
                    as extern "C-unwind" fn(
                        *mut AnyObject,
                        Sel,
                        *mut NSApplication,
                    ) -> NSApplicationTerminateReply,
            );
            builder.add_method(
                sel!(completeTermination:),
                complete_termination as extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject),
            );
            builder.add_method(
                sel!(terminate:),
                terminate as extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject),
            );
        }
        builder.register()
    })
}

/// One host owns the decision. AppKit's YES reply can exit without returning,
/// so approval belongs after fullscreen restoration, on_exit and state destruction.
pub(crate) struct Guard {
    request: Rc<Request>,
    _entry: EntryGate,
}

impl Guard {
    pub(crate) fn install(
        notify: impl Fn() -> bool + 'static,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let main = MainThreadMarker::new().ok_or("termination guard requires the main thread")?;
        if ACTIVE.with(|active| active.borrow().is_some()) {
            return Err("a native termination guard is already active".into());
        }
        let app = NSApplication::sharedApplication(main);
        // AppKit's delegate is weak; retain it through host destruction and the
        // final reply so winit's existing applicationWillTerminate callback survives.
        let delegate = app
            .delegate()
            .ok_or("winit application delegate is missing")?;
        let object: &AnyObject = (*delegate).as_ref();
        let class = object.class();
        if class.name() != c"WinitApplicationDelegate" {
            return Err(
                "native termination integration requires winit's application delegate".into(),
            );
        }
        let source = methods()
            .instance_method(sel!(applicationShouldTerminate:))
            .unwrap();
        let selector = sel!(applicationShouldTerminate:);
        if let Some(existing) = class.instance_method(selector) {
            if !std::ptr::fn_addr_eq(existing.implementation(), source.implementation()) {
                return Err("winit already implements applicationShouldTerminate; revisit native termination integration".into());
            }
        } else {
            // Winit 0.30.13 owns/type-checks this delegate despite its custom-delegate
            // documentation. Add only the missing optional method, using objc2's
            // typed implementation/encoding; never replace a winit callback or change isa.
            let added = unsafe {
                objc2::ffi::class_addMethod(
                    (class as *const objc2::runtime::AnyClass).cast_mut(),
                    selector,
                    source.implementation(),
                    objc2::ffi::method_getTypeEncoding(source),
                )
            };
            if !added.as_bool() {
                return Err("could not register the native termination decision".into());
            }
        }
        let entry = EntryGate::install(&app)?;
        let request = Rc::new(Request {
            app,
            delegate,
            notify: Box::new(notify),
            pending: Cell::new(false),
            delivered: Cell::new(false),
            prepared: Cell::new(false),
        });
        ACTIVE.with(|active| *active.borrow_mut() = Some(request.clone()));
        Ok(Self {
            request,
            _entry: entry,
        })
    }

    pub(crate) fn delivered(&self) -> bool {
        let pending = self.request.pending.get();
        if pending {
            self.request.delivered.set(true);
        }
        pending
    }

    pub(crate) fn awaiting_decision(&self) -> bool {
        self.request.pending.get() && self.request.delivered.get()
    }

    pub(crate) fn reply(&self, allow: bool) {
        self.request.reply(allow);
    }

    pub(crate) fn approve_after_cleanup(&self) {
        self.request.prepared.set(true);
        // AppKit's termination wait must be resolved while the run loop is still
        // running. Reply outside winit's borrowed event handler: YES invokes its
        // original applicationWillTerminate callback synchronously.
        unsafe {
            let target: Retained<NSObject> = msg_send![methods(), new];
            let modes = NSArray::from_slice(&[NSRunLoopCommonModes]);
            target.performSelector_withObject_afterDelay_inModes(
                sel!(completeTermination:),
                None,
                0.,
                &modes,
            );
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.reply(false);
        ACTIVE.with(|active| {
            let mut active = active.borrow_mut();
            if active
                .as_ref()
                .is_some_and(|request| Rc::ptr_eq(request, &self.request))
            {
                *active = None;
            }
        });
    }
}
