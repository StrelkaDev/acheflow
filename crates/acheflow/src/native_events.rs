//! AppKit events enter the same bootstrap as CLI/IPC. No separate executable.
//!
//! Winit 0.31 documents installing a custom NSApplicationDelegate after creating
//! its event loop: https://github.com/rust-windowing/winit/tree/v0.31.0-beta.4/winit-appkit
use acheflow_extension_api::forwarding::MAX_FORWARD_URL_BYTES;
use acheflow_launcher::{ActivationError, INBOX_CAPACITY, Request};
use objc2::{
    MainThreadMarker, MainThreadOnly, define_class, extern_methods, rc::Retained,
    runtime::ProtocolObject,
};
use objc2_app_kit::{
    NSApplication, NSApplicationDelegate, NSApplicationTerminateReply, NSUserActivityRestoring,
};
use objc2_foundation::{
    NSArray, NSNotification, NSNumber, NSObject, NSObjectProtocol, NSURL, NSUserActivity, ns_string,
};
use std::{cell::RefCell, collections::VecDeque, ptr::NonNull};
use winit::event_loop::EventLoopProxy;

#[derive(Default)]
pub struct Events {
    pub requests: VecDeque<Request>,
    pub default_launch: Option<bool>,
    pub quit: bool,
    proxy: Option<EventLoopProxy>,
}
thread_local! { static EVENTS: RefCell<Events> = RefCell::new(Events::default()); }

fn send(request: Request) {
    EVENTS.with_borrow_mut(|events| {
        if events.requests.len() < INBOX_CAPACITY {
            events.requests.push_back(request);
        } else {
            eprintln!("{}", ActivationError::Busy);
        }
        if let Some(proxy) = &events.proxy {
            proxy.wake_up();
        }
    });
}
fn open(url: &NSURL) {
    if let Some(value) = url.absoluteString() {
        if value.length() > MAX_FORWARD_URL_BYTES {
            eprintln!("{}", ActivationError::InvalidLink);
            return;
        }
        send(Request::Forward {
            url: value.to_string(),
        });
    }
}

// The delegate has no ivars, overrides no initializers and is main-thread-only.
// Method signatures match objc2-app-kit's NSApplicationDelegate declarations.
// objc2's macros provide the checked bridge; the workspace unsafe-code lint stays
// unchanged. No raw pointers are dereferenced and no callback payload is logged.
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AcheflowApplicationDelegate"]
    struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(application:openURLs:))]
        fn open_urls(&self, _application: &NSApplication, urls: &NSArray<NSURL>) {
            for url in urls.iter().take(INBOX_CAPACITY) {
                open(&url);
            }
        }
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn launched(&self, notification: &NSNotification) {
            // Public AppKit userInfo key. Nondefault launches must await their
            // URL even when modern macOS delivers it after this notification.
            let default_launch = notification
                .userInfo()
                .and_then(|info| {
                    info.objectForKey(ns_string!("NSApplicationLaunchIsDefaultLaunchKey"))
                })
                .and_then(|value| value.downcast::<NSNumber>().ok())
                .is_none_or(|value| value.boolValue());
            EVENTS.with_borrow_mut(|events| {
                events.default_launch = Some(default_launch);
                if let Some(proxy) = &events.proxy {
                    proxy.wake_up();
                }
            });
        }
        // A nondefault launch can also restore state or request an untitled
        // window. Treat AppKit's explicit no-document request as Show; the
        // launch-completion flag alone does not identify URL activation.
        #[unsafe(method(applicationShouldOpenUntitledFile:))]
        fn should_open_untitled(&self, _application: &NSApplication) -> bool {
            send(Request::Show);
            false
        }
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn reopen(&self, _application: &NSApplication, _visible: bool) -> bool {
            send(Request::Show);
            false
        }
        #[unsafe(method(application:continueUserActivity:restorationHandler:))]
        fn continue_activity(
            &self,
            _application: &NSApplication,
            activity: &NSUserActivity,
            _restore: &block2::DynBlock<
                dyn Fn(NonNull<NSArray<ProtocolObject<dyn NSUserActivityRestoring>>>),
            >,
        ) -> bool {
            if activity.activityType().to_string() == "NSUserActivityTypeBrowsingWeb"
                && let Some(url) = activity.webpageURL()
            {
                open(&url);
                true
            } else {
                false
            }
        }
        #[unsafe(method(applicationShouldTerminate:))]
        fn terminate(&self, _application: &NSApplication) -> NSApplicationTerminateReply {
            EVENTS.with_borrow_mut(|events| {
                events.quit = true;
                if let Some(proxy) = &events.proxy {
                    proxy.wake_up();
                }
            });
            NSApplicationTerminateReply::TerminateCancel
        }
    }
);
impl Delegate {
    extern_methods!(
        #[unsafe(method(new))]
        fn new(marker: MainThreadMarker) -> Retained<Self>;
    );
}

pub struct Registration {
    application: Retained<NSApplication>,
    _delegate: Retained<Delegate>,
}
impl Registration {
    pub fn install(proxy: EventLoopProxy) -> Result<Self, String> {
        let marker =
            MainThreadMarker::new().ok_or("Native activation must start on the main thread")?;
        let delegate = Delegate::new(marker);
        let application = NSApplication::sharedApplication(marker);
        EVENTS.with_borrow_mut(|events| events.proxy = Some(proxy));
        application.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        Ok(Self {
            application,
            _delegate: delegate,
        })
    }
    pub fn take(&self) -> Events {
        EVENTS.with_borrow_mut(|events| Events {
            requests: std::mem::take(&mut events.requests),
            default_launch: events.default_launch.take(),
            quit: std::mem::take(&mut events.quit),
            proxy: None,
        })
    }
    pub fn show_dock(&self) {
        self.application
            .setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Regular);
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.application.setDelegate(None);
        EVENTS.with_borrow_mut(|events| *events = Events::default());
    }
}
