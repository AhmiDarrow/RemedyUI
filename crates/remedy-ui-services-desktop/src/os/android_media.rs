//! Android's `MediaSession` and its media notification, through the Android bridge's Java
//! (`dev.remedyui.android.MediaControls`), found with the app's class loader: the APK carries
//! the bridge's dex the way `cargo remedy package` makes it. The app's Context comes from
//! ndk-context (which android-activity fills in).

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::sys::{jint, jlong};
use jni::{JNIEnv, JavaVM, NativeMethod};

use crate::DesktopError;
use crate::media_session::{Artwork, Handler, MediaCommand, MediaState, NowPlaying};

const CLASS: &str = "dev.remedyui.android.MediaControls";

struct Java {
    vm: JavaVM,
    context: GlobalRef,
    class: GlobalRef,
}

/// The live sessions' handlers, by the handle their Java half calls back with.
fn handlers() -> &'static Mutex<HashMap<jlong, Arc<Handler>>> {
    static H: OnceLock<Mutex<HashMap<jlong, Arc<Handler>>>> = OnceLock::new();
    H.get_or_init(Mutex::default)
}

/// The pending Java exception as text (cleared), or the JNI error.
fn describe(env: &mut JNIEnv<'_>, e: jni::errors::Error) -> String {
    if !env.exception_check().unwrap_or(false) {
        return e.to_string();
    }
    let thrown = env.exception_occurred();
    let _ = env.exception_clear();
    thrown
        .and_then(|t| env.call_method(&t, "toString", "()Ljava/lang/String;", &[]))
        .and_then(|s| s.l())
        .and_then(|s| env.get_string(&JString::from(s)).map(String::from))
        .unwrap_or_else(|_| e.to_string())
}

/// Runs `f` on this thread's JNI environment, in a frame of its own.
fn with_env<T>(vm: &JavaVM, f: impl FnOnce(&mut JNIEnv<'_>) -> jni::errors::Result<T>) -> Result<T, String> {
    let mut env = vm.attach_current_thread_permanently().map_err(|e| e.to_string())?;
    let r = env.with_local_frame(16, |env| f(env));
    r.map_err(|e| describe(&mut env, e))
}

fn java() -> Result<&'static Java, String> {
    static J: OnceLock<Result<Java, String>> = OnceLock::new();
    J.get_or_init(|| {
        let ctx = std::panic::catch_unwind(ndk_context::android_context)
            .map_err(|_| "no Android context (the app is not running in an activity)".to_owned())?;
        // SAFETY: ndk-context holds the process's JavaVM, valid for the process's life.
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }.map_err(|e| e.to_string())?;
        let (context, class) = with_env(&vm, |env| {
            // SAFETY: ndk-context's context is a global reference to the app's Context (its
            // activity), alive while the app runs; it is only read here.
            let context = unsafe { JObject::from_raw(ctx.context().cast()) };
            let loader =
                env.call_method(&context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?.l()?;
            let name = env.new_string(CLASS)?;
            let class = env
                .call_method(
                    &loader,
                    "loadClass",
                    "(Ljava/lang/String;)Ljava/lang/Class;",
                    &[JValue::Object(&name)],
                )?
                .l()?;
            let class = JClass::from(class);
            let natives = [NativeMethod {
                name: "command".into(),
                sig: "(JIJ)V".into(),
                fn_ptr: command as *mut std::ffi::c_void,
            }];
            env.register_native_methods(&class, &natives)?;
            Ok((env.new_global_ref(&context)?, env.new_global_ref(class)?))
        })
        .map_err(|e| {
            format!(
                "the APK has no {CLASS} ({e}): it must carry the Android bridge's dex (cargo remedy \
                 package does)"
            )
        })?;
        Ok(Java { vm, context, class })
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// `MediaControls.command(handle, what, arg)`: a button, from the main thread.
extern "system" fn command(_env: JNIEnv<'_>, _class: JClass<'_>, handle: jlong, what: jint, arg: jlong) {
    let h = handlers().lock().unwrap_or_else(PoisonError::into_inner).get(&handle).cloned();
    let Some(h) = h else { return };
    #[allow(clippy::cast_precision_loss, reason = "milliseconds of media fit an f64")]
    let c = match what {
        0 => MediaCommand::Play,
        1 => MediaCommand::Pause,
        2 => MediaCommand::PlayPause,
        3 => MediaCommand::Stop,
        4 => MediaCommand::Next,
        5 => MediaCommand::Previous,
        6 => MediaCommand::SeekBy(arg as f64 / 1000.0),
        7 => MediaCommand::SeekTo(Duration::from_millis(u64::try_from(arg).unwrap_or(0))),
        _ => return,
    };
    h(c);
}

fn millis(d: Duration) -> jlong {
    jlong::try_from(d.as_millis()).unwrap_or(jlong::MAX)
}

pub(crate) struct Session {
    java: &'static Java,
    controls: GlobalRef,
    handle: jlong,
}

impl Session {
    pub(crate) fn new(handler: Handler) -> Result<Session, DesktopError> {
        static NEXT: AtomicI64 = AtomicI64::new(1);
        let java = java().map_err(|e| DesktopError::unsupported("Media controls", e))?;
        let handle = NEXT.fetch_add(1, Ordering::Relaxed);
        let controls = with_env(&java.vm, |env| {
            let class: &JClass<'_> = java.class.as_obj().into();
            let c = env
                .call_static_method(
                    class,
                    "open",
                    "(Landroid/content/Context;J)Ldev/remedyui/android/MediaControls;",
                    &[JValue::Object(java.context.as_obj()), JValue::Long(handle)],
                )?
                .l()?;
            env.new_global_ref(c)
        })
        .map_err(|e| DesktopError::platform(format!("MediaSession: {e}")))?;
        handlers().lock().unwrap_or_else(PoisonError::into_inner).insert(handle, Arc::new(handler));
        Ok(Session { java, controls, handle })
    }

    fn call(
        &self,
        method: &str,
        sig: &str,
        args: impl for<'a> FnOnce(&mut JNIEnv<'a>) -> jni::errors::Result<Vec<Arg<'a>>>,
    ) {
        let _ = with_env(&self.java.vm, |env| {
            let owned = args(env)?;
            let values: Vec<JValue<'_, '_>> = owned.iter().map(Arg::value).collect();
            env.call_method(self.controls.as_obj(), method, sig, &values)?;
            Ok(())
        });
    }

    pub(crate) fn now_playing(&self, now: Option<&NowPlaying>) {
        self.call("metadata", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;[B)V", |env| {
            let Some(now) = now else {
                return Ok(vec![
                    Arg::Object(JObject::null()),
                    Arg::Object(JObject::null()),
                    Arg::Object(JObject::null()),
                    Arg::Object(JObject::null()),
                ]);
            };
            let title = Arg::Object(env.new_string(&now.title)?.into());
            let subtitle = match &now.subtitle {
                Some(s) => Arg::Object(env.new_string(s)?.into()),
                None => Arg::Object(JObject::null()),
            };
            let (url, bytes) = match &now.art {
                Some(Artwork::Url(u)) => {
                    (Arg::Object(env.new_string(u)?.into()), Arg::Object(JObject::null()))
                }
                Some(Artwork::Bytes(b)) => {
                    (Arg::Object(JObject::null()), Arg::Object(env.byte_array_from_slice(b)?.into()))
                }
                None => (Arg::Object(JObject::null()), Arg::Object(JObject::null())),
            };
            Ok(vec![title, subtitle, url, bytes])
        });
    }

    pub(crate) fn playback(&self, state: MediaState, position: Duration, length: Option<Duration>) {
        let what = match state {
            MediaState::Stopped => 0,
            MediaState::Playing => 1,
            MediaState::Paused => 2,
        };
        let length = length.map_or(-1, millis);
        self.call("playback", "(IJJ)V", |_| {
            Ok(vec![Arg::Int(what), Arg::Long(millis(position)), Arg::Long(length)])
        });
    }

    pub(crate) fn can_next(&self, can: bool) {
        self.call("canNext", "(Z)V", |_| Ok(vec![Arg::Bool(can)]));
    }
}

/// A Java argument that owns its local reference until the call.
enum Arg<'a> {
    Object(JObject<'a>),
    Int(jint),
    Long(jlong),
    Bool(bool),
}

impl Arg<'_> {
    fn value(&self) -> JValue<'_, '_> {
        match self {
            Arg::Object(o) => JValue::Object(o),
            Arg::Int(i) => JValue::Int(*i),
            Arg::Long(l) => JValue::Long(*l),
            Arg::Bool(b) => JValue::Bool(u8::from(*b)),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        handlers().lock().unwrap_or_else(PoisonError::into_inner).remove(&self.handle);
        self.call("close", "()V", |_| Ok(Vec::new()));
    }
}
