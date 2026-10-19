//! Finding other Platters on the phone. iOS doesn't let an app send raw
//! multicast without a special entitlement, so `mdns-sd` stays silent there;
//! Apple's own Bonjour (`NSNetService`) is allowed for the types listed in
//! Info.plist (`_platter._tcp`). Same [`PeerDirectory`] as [`super::directory`].

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject};
use objc2::{AnyThread, define_class, msg_send};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSString};
use parking_lot::Mutex;

use super::directory::{Announce, PeerDirectory, ip_of};
use crate::remote::lan::{FIRST_SCAN, MdnsAnswer};

const TYPE: &str = "_platter._tcp.";
const DOMAIN: &str = "local.";

/// Resolved Platters, by instance name.
static SEEN: Mutex<Option<HashMap<String, MdnsAnswer>>> = Mutex::new(None);

/// The Bonjour objects, which live on the main thread.
struct Live {
    browser: Retained<AnyObject>,
    delegate: Retained<BrowseDelegate>,
    /// Services found and being resolved (Bonjour needs them kept).
    resolving: Vec<Retained<AnyObject>>,
    published: Option<Retained<AnyObject>>,
}

thread_local! {
    static LIVE: std::cell::RefCell<Option<Live>> = const { std::cell::RefCell::new(None) };
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "PlatterBonjourDelegate"]
    struct BrowseDelegate;

    #[allow(non_snake_case)]
    impl BrowseDelegate {
        #[unsafe(method(netServiceBrowser:didFindService:moreComing:))]
        fn found(&self, _browser: &AnyObject, service: &AnyObject, _more: bool) {
            // SAFETY: Bonjour on the main thread; the service is kept until resolved.
            unsafe {
                let _: () = msg_send![service, setDelegate: self];
                let _: () = msg_send![service, resolveWithTimeout: 5.0f64];
                let kept: Retained<AnyObject> = Retained::retain(service as *const AnyObject as *mut AnyObject).unwrap();
                LIVE.with(|l| {
                    if let Some(live) = l.borrow_mut().as_mut() {
                        live.resolving.push(kept);
                    }
                });
            }
        }

        #[unsafe(method(netServiceBrowser:didRemoveService:moreComing:))]
        fn removed(&self, _browser: &AnyObject, service: &AnyObject, _more: bool) {
            // SAFETY: reading the name of a Bonjour service.
            let name = unsafe { service_name(service) };
            if let Some(seen) = SEEN.lock().as_mut() {
                seen.remove(&name);
            }
        }

        #[unsafe(method(netServiceDidResolveAddress:))]
        fn resolved(&self, service: &AnyObject) {
            // SAFETY: reading a resolved service's name, port, addresses and TXT.
            let answer = unsafe { answer_of(service) };
            if let Some(a) = answer {
                SEEN.lock().get_or_insert_with(HashMap::new).insert(a.instance.clone(), a);
            }
        }
    }
);

unsafe fn service_name(service: &AnyObject) -> String {
    let name: *mut NSString = unsafe { msg_send![service, name] };
    if name.is_null() { String::new() } else { unsafe { (*name).to_string() } }
}

unsafe fn answer_of(service: &AnyObject) -> Option<MdnsAnswer> {
    unsafe {
        let instance = service_name(service);
        let port: isize = msg_send![service, port];
        let addresses: *mut NSArray<NSData> = msg_send![service, addresses];
        let mut addrs: Vec<IpAddr> = Vec::new();
        if !addresses.is_null() {
            for d in (*addresses).iter() {
                if let Some(ip) = ip_of(&d.to_vec()) {
                    addrs.push(ip);
                }
            }
        }
        addrs.sort_by_key(|a| !a.is_ipv4());
        let mut txt = HashMap::new();
        let record: *mut NSData = msg_send![service, TXTRecordData];
        if !record.is_null() {
            let class = AnyClass::get(c"NSNetService")?;
            let dict: *mut NSDictionary<NSString, NSData> = msg_send![class, dictionaryFromTXTRecordData: record];
            if !dict.is_null() {
                for k in (*dict).allKeys().iter() {
                    if let Some(v) = (*dict).objectForKey(&k) {
                        txt.insert(k.to_string().to_ascii_lowercase(), String::from_utf8_lossy(&v.to_vec()).into_owned());
                    }
                }
            }
        }
        (port > 0 && !addrs.is_empty()).then(|| MdnsAnswer {
            service: super::SERVICE.to_string(),
            instance,
            addrs,
            port: port as u16,
            txt,
        })
    }
}

/// Bonjour on the phone.
pub struct BonjourPeers {
    started: Instant,
}

impl BonjourPeers {
    pub fn new() -> Arc<Self> {
        DispatchQueue::main().exec_async(|| {
            // SAFETY: Bonjour objects made and scheduled on the main thread's
            // run loop, kept in LIVE for the app's life.
            unsafe {
                let Some(class) = AnyClass::get(c"NSNetServiceBrowser") else { return };
                let browser: Retained<AnyObject> = msg_send![class, new];
                let delegate: Retained<BrowseDelegate> = msg_send![BrowseDelegate::alloc(), init];
                let _: () = msg_send![&*browser, setDelegate: &*delegate];
                let _: () = msg_send![&*browser, searchForServicesOfType: &*NSString::from_str(TYPE), inDomain: &*NSString::from_str(DOMAIN)];
                LIVE.with(|l| *l.borrow_mut() = Some(Live { browser, delegate, resolving: Vec::new(), published: None }));
            }
        });
        Arc::new(Self { started: Instant::now() })
    }
}

#[async_trait]
impl PeerDirectory for BonjourPeers {
    fn announce(&self, a: &Announce) {
        let (name, port) = (a.id.clone(), a.port);
        let txt: Vec<(String, String)> = a.txt.clone();
        DispatchQueue::main().exec_async(move || {
            // SAFETY: NSNetService made, given its TXT record and published on
            // the main thread; the old one is stopped first.
            unsafe {
                let Some(class) = AnyClass::get(c"NSNetService") else { return };
                LIVE.with(|l| {
                    let mut l = l.borrow_mut();
                    let Some(live) = l.as_mut() else { return };
                    if let Some(old) = live.published.take() {
                        let _: () = msg_send![&*old, stop];
                    }
                    let alloc: *mut AnyObject = msg_send![class, alloc];
                    let raw: *mut AnyObject = msg_send![alloc,
                        initWithDomain: &*NSString::from_str(DOMAIN),
                        type: &*NSString::from_str(TYPE),
                        name: &*NSString::from_str(&name),
                        port: port as i32];
                    // `init…` hands back an owned (+1) object.
                    let Some(svc) = Retained::from_raw(raw) else { return };
                    let keys: Vec<Retained<NSString>> = txt.iter().map(|(k, _)| NSString::from_str(k)).collect();
                    let vals: Vec<Retained<NSData>> = txt.iter().map(|(_, v)| NSData::with_bytes(v.as_bytes())).collect();
                    let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
                    let dict = NSDictionary::from_retained_objects(&key_refs, &vals);
                    let record: *mut NSData = msg_send![class, dataFromTXTRecordDictionary: &*dict];
                    if !record.is_null() {
                        let _: bool = msg_send![&*svc, setTXTRecordData: record];
                    }
                    let _: () = msg_send![&*svc, publish];
                    live.published = Some(svc);
                });
            }
        });
    }

    fn withdraw(&self) {
        DispatchQueue::main().exec_async(|| {
            LIVE.with(|l| {
                if let Some(live) = l.borrow_mut().as_mut()
                    && let Some(old) = live.published.take()
                {
                    // SAFETY: stopping our own published service.
                    unsafe {
                        let _: () = msg_send![&*old, stop];
                    }
                }
            });
        });
    }

    async fn scan(&self) -> Vec<MdnsAnswer> {
        let waited = self.started.elapsed();
        if waited < FIRST_SCAN {
            tokio::time::sleep(FIRST_SCAN - waited).await;
        }
        SEEN.lock().as_ref().map(|m| m.values().cloned().collect()).unwrap_or_default()
    }
}
