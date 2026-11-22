//! The Android APIs the bridge calls: Java through JNI on the activity, the bridge's own Java
//! half (the app's own code when the APK carries it, else loaded from memory, so apps on a
//! plain native activity ship no Java), the C library's time zone and the system log. Every
//! unsafe call of the bridge is here.

use std::ffi::{CString, c_char, c_int, c_void};
use std::sync::{Arc, OnceLock};

use jni::objects::{GlobalRef, JClass, JObject, JObjectArray, JString, JValue};
use jni::sys::{jint, jlong};
use jni::{JNIEnv, JavaVM, NativeMethod};
use winit::platform::android::activity::AndroidApp;

use crate::android::{Notice, notify};
use crate::convert::{self, AndroidSettings};

/// The bridge's Java half (`java/`, rebuilt by `tools/build-dex.sh`).
static DEX: &[u8] = include_bytes!("../dex/classes.dex");
const BRIDGE_CLASS: &str = "dev.remedyui.android.Bridge";
/// The id native callbacks carry (one activity per process).
const HANDLE: jlong = 1;

/// The Java side of the activity.
pub(crate) struct Java {
    vm: Arc<JavaVM>,
    activity: GlobalRef,
}

type JResult<T> = jni::errors::Result<T>;

/// What a failed Java call threw, as text, with the exception cleared so later calls work.
fn describe(env: &mut JNIEnv<'_>, error: jni::errors::Error) -> String {
    if !matches!(error, jni::errors::Error::JavaException) {
        return error.to_string();
    }
    let Ok(thrown) = env.exception_occurred() else { return "a Java exception".into() };
    let _ = env.exception_clear();
    let text = env
        .call_method(&thrown, "toString", "()Ljava/lang/String;", &[])
        .and_then(|v| v.l())
        .and_then(|s| env.get_string(&JString::from(s)).map(String::from));
    let _ = env.exception_clear();
    text.unwrap_or_else(|_| "a Java exception".into())
}

fn string(env: &mut JNIEnv<'_>, obj: JObject<'_>) -> JResult<Option<String>> {
    if obj.is_null() {
        return Ok(None);
    }
    let s = JString::from(obj);
    Ok(Some(env.get_string(&s)?.into()))
}

impl Java {
    /// The activity's Java VM and object, from android-activity.
    pub(crate) fn new(app: &AndroidApp) -> Result<Java, String> {
        // SAFETY: android-activity hands out the process's JavaVM pointer, which stays valid
        // for the life of the process.
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }.map_err(|e| e.to_string())?;
        let env = vm.attach_current_thread().map_err(|e| e.to_string())?;
        // SAFETY: the pointer is android-activity's global reference to the activity, alive
        // while the `AndroidApp` is (it outlives this call); the bridge takes its own global
        // reference from it and keeps only that.
        let obj = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
        let activity = env.new_global_ref(&obj).map_err(|e| e.to_string())?;
        drop(env);
        Ok(Java { vm: Arc::new(vm), activity })
    }

    /// Runs `f` with this thread's JNI environment and the activity, in a local frame of its
    /// own; a Java exception becomes the error's text.
    fn call<T>(&self, f: impl FnOnce(&mut JNIEnv<'_>, &JObject<'_>) -> JResult<T>) -> Result<T, String> {
        let mut env = self.vm.attach_current_thread().map_err(|e| e.to_string())?;
        let activity = self.activity.as_obj();
        let r = env.with_local_frame(16, |env| f(env, activity));
        r.map_err(|e| describe(&mut env, e))
    }

    /// The configuration and settings RemedyUI follows. Each value is read on its own, so one
    /// Android does not have leaves the others.
    pub(crate) fn settings(&self) -> AndroidSettings {
        fn configuration<'a>(env: &mut JNIEnv<'a>, activity: &JObject<'_>) -> JResult<JObject<'a>> {
            let res =
                env.call_method(activity, "getResources", "()Landroid/content/res/Resources;", &[])?.l()?;
            env.call_method(&res, "getConfiguration", "()Landroid/content/res/Configuration;", &[])?.l()
        }
        fn resolver<'a>(env: &mut JNIEnv<'a>, activity: &JObject<'_>) -> JResult<JObject<'a>> {
            env.call_method(activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()
        }
        let night = self
            .call(|env, a| {
                let c = configuration(env, a)?;
                // UI_MODE_NIGHT_MASK, _YES, _NO.
                Ok(match env.get_field(&c, "uiMode", "I")?.i()? & 0x30 {
                    0x20 => Some(true),
                    0x10 => Some(false),
                    _ => None,
                })
            })
            .ok()
            .flatten();
        let font_scale = self
            .call(|env, a| {
                let c = configuration(env, a)?;
                env.get_field(&c, "fontScale", "F")?.f()
            })
            .ok();
        let locale = self
            .call(|env, a| {
                let c = configuration(env, a)?;
                let list = env.call_method(&c, "getLocales", "()Landroid/os/LocaleList;", &[])?.l()?;
                let first = env.call_method(&list, "get", "(I)Ljava/util/Locale;", &[JValue::Int(0)])?.l()?;
                if first.is_null() {
                    return Ok(None);
                }
                let tag = env.call_method(&first, "toLanguageTag", "()Ljava/lang/String;", &[])?.l()?;
                string(env, tag)
            })
            .ok()
            .flatten();
        let locales = self
            .call(|env, a| {
                let c = configuration(env, a)?;
                let list = env.call_method(&c, "getLocales", "()Landroid/os/LocaleList;", &[])?.l()?;
                let tags = env.call_method(&list, "toLanguageTags", "()Ljava/lang/String;", &[])?.l()?;
                string(env, tags)
            })
            .ok()
            .flatten();
        let animator_scale = self
            .call(|env, a| {
                let r = resolver(env, a)?;
                let key = env.new_string("animator_duration_scale")?;
                env.call_static_method(
                    "android/provider/Settings$Global",
                    "getFloat",
                    "(Landroid/content/ContentResolver;Ljava/lang/String;F)F",
                    &[JValue::Object(&r), JValue::Object(&key), JValue::Float(1.0)],
                )?
                .f()
            })
            .ok();
        let high_contrast_text = self
            .call(|env, a| {
                let r = resolver(env, a)?;
                let key = env.new_string("high_text_contrast_enabled")?;
                let on = env
                    .call_static_method(
                        "android/provider/Settings$Secure",
                        "getInt",
                        "(Landroid/content/ContentResolver;Ljava/lang/String;I)I",
                        &[JValue::Object(&r), JValue::Object(&key), JValue::Int(0)],
                    )?
                    .i()?;
                Ok(on != 0)
            })
            .ok();
        let accent = if AndroidApp::sdk_version() >= 31 {
            self.call(|env, a| {
                let id = env.get_static_field("android/R$color", "system_accent1_500", "I")?.i()?;
                let res =
                    env.call_method(a, "getResources", "()Landroid/content/res/Resources;", &[])?.l()?;
                let none = JObject::null();
                env.call_method(
                    &res,
                    "getColor",
                    "(ILandroid/content/res/Resources$Theme;)I",
                    &[JValue::Int(id), JValue::Object(&none)],
                )?
                .i()
            })
            .ok()
        } else {
            None
        };
        let double_tap_ms = self
            .call(|env, _| {
                env.call_static_method("android/view/ViewConfiguration", "getDoubleTapTimeout", "()I", &[])?
                    .i()
            })
            .ok();
        let television = self
            .call(|env, a| {
                let c = configuration(env, a)?;
                // UI_MODE_TYPE_MASK, UI_MODE_TYPE_TELEVISION.
                Ok(env.get_field(&c, "uiMode", "I")?.i()? & 0x0f == 0x04)
            })
            .unwrap_or(false);
        AndroidSettings {
            night,
            font_scale,
            locale,
            locales,
            animator_scale,
            high_contrast_text,
            accent,
            double_tap_ms,
            television,
        }
    }

    fn clipboard<'a>(env: &mut JNIEnv<'a>, activity: &JObject<'_>) -> JResult<JObject<'a>> {
        let name = env.new_string("clipboard")?;
        env.call_method(
            activity,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[JValue::Object(&name)],
        )?
        .l()
    }

    /// The first clipboard item: as text (`html` false) or its HTML.
    pub(crate) fn clipboard_read(&self, html: bool) -> Result<Option<String>, String> {
        self.call(|env, a| {
            let cm = Self::clipboard(env, a)?;
            if !env.call_method(&cm, "hasPrimaryClip", "()Z", &[])?.z()? {
                return Ok(None);
            }
            // Null when the system withholds it (Android 10 and later, without focus).
            let clip = env.call_method(&cm, "getPrimaryClip", "()Landroid/content/ClipData;", &[])?.l()?;
            if clip.is_null() || env.call_method(&clip, "getItemCount", "()I", &[])?.i()? < 1 {
                return Ok(None);
            }
            let item = env
                .call_method(&clip, "getItemAt", "(I)Landroid/content/ClipData$Item;", &[JValue::Int(0)])?
                .l()?;
            if html {
                let h = env.call_method(&item, "getHtmlText", "()Ljava/lang/String;", &[])?.l()?;
                return string(env, h);
            }
            let cs = env
                .call_method(
                    &item,
                    "coerceToText",
                    "(Landroid/content/Context;)Ljava/lang/CharSequence;",
                    &[JValue::Object(a)],
                )?
                .l()?;
            if cs.is_null() {
                return Ok(None);
            }
            let s = env.call_method(&cs, "toString", "()Ljava/lang/String;", &[])?.l()?;
            string(env, s)
        })
    }

    /// Puts text (and HTML, when given) on the clipboard; `sensitive` marks it so the system
    /// hides it in clipboard previews (`ClipDescription.EXTRA_IS_SENSITIVE`, Android 13).
    pub(crate) fn clipboard_write(
        &self,
        text: &str,
        html: Option<&str>,
        sensitive: bool,
    ) -> Result<(), String> {
        self.call(|env, a| {
            let cm = Self::clipboard(env, a)?;
            let label = env.new_string("")?;
            let t = env.new_string(text)?;
            let clip = match html {
                Some(h) => {
                    let h = env.new_string(h)?;
                    env.call_static_method(
                        "android/content/ClipData",
                        "newHtmlText",
                        "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;Ljava/lang/String;)Landroid/content/ClipData;",
                        &[JValue::Object(&label), JValue::Object(&t), JValue::Object(&h)],
                    )?
                    .l()?
                }
                None => env
                    .call_static_method(
                        "android/content/ClipData",
                        "newPlainText",
                        "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;",
                        &[JValue::Object(&label), JValue::Object(&t)],
                    )?
                    .l()?,
            };
            if sensitive {
                let desc = env.call_method(&clip, "getDescription", "()Landroid/content/ClipDescription;", &[])?.l()?;
                let extras = env.new_object("android/os/PersistableBundle", "()V", &[])?;
                let key = env.new_string("android.content.extra.IS_SENSITIVE")?;
                env.call_method(&extras, "putBoolean", "(Ljava/lang/String;Z)V", &[JValue::Object(&key), JValue::Bool(1)])?;
                env.call_method(&desc, "setExtras", "(Landroid/os/PersistableBundle;)V", &[JValue::Object(&extras)])?;
            }
            env.call_method(&cm, "setPrimaryClip", "(Landroid/content/ClipData;)V", &[JValue::Object(&clip)])?;
            Ok(())
        })
    }

    /// Empties the clipboard (`clearPrimaryClip`, Android 9 and later).
    pub(crate) fn clipboard_clear(&self) -> Result<(), String> {
        self.call(|env, a| {
            let cm = Self::clipboard(env, a)?;
            env.call_method(&cm, "clearPrimaryClip", "()V", &[])?;
            Ok(())
        })
    }

    /// Opens a web link in the user's browser, or a mail link in their mail app.
    pub(crate) fn open_url(&self, url: &str) -> Result<(), String> {
        let mail = url.trim_start().to_ascii_lowercase().starts_with("mailto:");
        self.call(|env, a| {
            let u = env.new_string(url)?;
            let uri = env
                .call_static_method(
                    "android/net/Uri",
                    "parse",
                    "(Ljava/lang/String;)Landroid/net/Uri;",
                    &[JValue::Object(&u)],
                )?
                .l()?;
            let action = env.new_string(if mail {
                "android.intent.action.SENDTO"
            } else {
                "android.intent.action.VIEW"
            })?;
            let intent = env.new_object(
                "android/content/Intent",
                "(Ljava/lang/String;Landroid/net/Uri;)V",
                &[JValue::Object(&action), JValue::Object(&uri)],
            )?;
            // FLAG_ACTIVITY_NEW_TASK: the browser runs as its own task.
            env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(0x1000_0000)])?;
            env.call_method(a, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
            Ok(())
        })
    }

    /// Sets or clears `FLAG_SECURE` on the activity's window, on the UI thread (window flags
    /// belong to it). Screenshots, recordings, casting and the recents thumbnail then show
    /// nothing of the window.
    pub(crate) fn set_secure(&self, app: &AndroidApp, on: bool) {
        const FLAG_SECURE: jint = 0x2000;
        let vm = self.vm.clone();
        let activity = self.activity.clone();
        app.run_on_java_main_thread(Box::new(move || {
            let Ok(mut env) = vm.attach_current_thread() else { return };
            let r = env.with_local_frame(4, |env| -> JResult<()> {
                let window =
                    env.call_method(activity.as_obj(), "getWindow", "()Landroid/view/Window;", &[])?.l()?;
                let method = if on { "addFlags" } else { "clearFlags" };
                env.call_method(&window, method, "(I)V", &[JValue::Int(FLAG_SECURE)])?;
                Ok(())
            });
            if let Err(e) = r {
                let text = describe(&mut env, e);
                log(&format!("RemedyUI: capture protection could not change: {text}"));
            }
        }));
    }

    /// A string extra of the intent that started the activity.
    pub(crate) fn intent_extra(&self, name: &str) -> Result<Option<String>, String> {
        self.call(|env, a| {
            let intent = env.call_method(a, "getIntent", "()Landroid/content/Intent;", &[])?.l()?;
            if intent.is_null() {
                return Ok(None);
            }
            let key = env.new_string(name)?;
            let v = env
                .call_method(
                    &intent,
                    "getStringExtra",
                    "(Ljava/lang/String;)Ljava/lang/String;",
                    &[JValue::Object(&key)],
                )?
                .l()?;
            string(env, v)
        })
    }

    /// Finishes the activity (`Activity.finish`, which Android carries out on its UI thread).
    pub(crate) fn finish(&self) -> Result<(), String> {
        self.call(|env, a| {
            env.call_method(a, "finish", "()V", &[])?;
            Ok(())
        })
    }

    /// Whether this app may offer an APK to the system installer (Android 8+ "install unknown
    /// apps"). Safe to call from any thread.
    pub(crate) fn can_request_package_installs(&self) -> Result<bool, String> {
        self.call(|env, a| {
            let pm =
                env.call_method(a, "getPackageManager", "()Landroid/content/pm/PackageManager;", &[])?.l()?;
            env.call_method(&pm, "canRequestPackageInstalls", "()Z", &[])?.z()
        })
    }

    /// Opens the system installer for an APK under `cache/remedy-share/<request>/<index>/<name>`,
    /// or the install-permission settings when that is not yet allowed. Runs on the UI thread
    /// and waits for the Java call to finish (not for the person to tap Install).
    pub(crate) fn install_apk(
        &self,
        app: &AndroidApp,
        request: &str,
        index: i32,
        mime: &str,
        name: &str,
    ) -> Result<String, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        let request = request.to_owned();
        let mime = mime.to_owned();
        let name = name.to_owned();
        let tx2 = tx.clone();
        self.on_bridge(app, "the package installer", move |env, b| {
            let r = (|| {
                let req = env.new_string(&request)?;
                let mime = env.new_string(&mime)?;
                let name = env.new_string(&name)?;
                let v = env
                    .call_method(
                        b,
                        "installApk",
                        "(Ljava/lang/String;ILjava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
                        &[
                            JValue::Object(&req),
                            JValue::Int(index),
                            JValue::Object(&mime),
                            JValue::Object(&name),
                        ],
                    )?
                    .l()?;
                string(env, v)
            })();
            let posted = match &r {
                Ok(Some(s)) => Ok(s.clone()),
                Ok(None) => Err("the installer returned nothing".into()),
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(posted);
            r.map(|_| ())
        })?;
        drop(tx2);
        rx.recv_timeout(std::time::Duration::from_secs(8))
            .map_err(|_| "the installer call did not finish".to_string())?
    }

    /// The bridge's Java class with its native methods bound: the app's own copy when its APK
    /// carries the bridge's dex (`cargo remedy package`: RemedyActivity, the foreground
    /// service and the notification classes then share it), else one loaded from the dex
    /// embedded here.
    fn bridge_class(&self) -> Result<&'static GlobalRef, String> {
        static CLASS: OnceLock<GlobalRef> = OnceLock::new();
        if let Some(c) = CLASS.get() {
            return Ok(c);
        }
        let packaged = self.call(|env, a| {
            let loader = env.call_method(a, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?.l()?;
            let name = env.new_string(BRIDGE_CLASS)?;
            let class = env
                .call_method(
                    &loader,
                    "loadClass",
                    "(Ljava/lang/String;)Ljava/lang/Class;",
                    &[JValue::Object(&name)],
                )?
                .l()?;
            let class = JClass::from(class);
            env.register_native_methods(&class, &natives())?;
            env.new_global_ref(class)
        });
        let class = match packaged {
            Ok(c) => c,
            Err(why) => {
                // A plain native activity has no Java of the app's: expected. Anything else
                // (an APK carrying another release's dex) is worth a line in the log.
                if !why.contains("ClassNotFoundException") {
                    log(&format!(
                        "RemedyUI: the APK's bridge classes are unusable ({why}); loading the bridge's own"
                    ));
                }
                self.call(|env, _| {
                    let loader_class = env.find_class("dalvik/system/InMemoryDexClassLoader")?;
                    // SAFETY: the buffer covers static bytes that live as long as the process;
                    // the class loader only reads them (it copies the dex while loading).
                    let buffer = unsafe { env.new_direct_byte_buffer(DEX.as_ptr().cast_mut(), DEX.len()) }?;
                    let none = JObject::null();
                    let loader = env.new_object(
                        loader_class,
                        "(Ljava/nio/ByteBuffer;Ljava/lang/ClassLoader;)V",
                        &[JValue::Object(&buffer), JValue::Object(&none)],
                    )?;
                    let name = env.new_string(BRIDGE_CLASS)?;
                    let class = env
                        .call_method(
                            &loader,
                            "loadClass",
                            "(Ljava/lang/String;)Ljava/lang/Class;",
                            &[JValue::Object(&name)],
                        )?
                        .l()?;
                    let class = JClass::from(class);
                    env.register_native_methods(&class, &natives())?;
                    env.new_global_ref(class)
                })?
            }
        };
        Ok(CLASS.get_or_init(|| class))
    }

    /// Starts the Java half on the UI thread: memory trims, configuration and setting
    /// changes, and the window's insets then reach the bridge as [`Notice`]s.
    pub(crate) fn start_bridge(&self, app: &AndroidApp) -> Result<(), String> {
        let class = self.bridge_class()?;
        let vm = self.vm.clone();
        let activity = self.activity.clone();
        app.run_on_java_main_thread(Box::new(move || {
            let made = vm.attach_current_thread().map_err(|e| e.to_string()).and_then(|mut env| {
                let r = env.with_local_frame(4, |env| {
                    let class: &JClass<'_> = class.as_obj().into();
                    let obj = env.new_object(
                        class,
                        "(JLandroid/app/Activity;)V",
                        &[JValue::Long(HANDLE), JValue::Object(activity.as_obj())],
                    )?;
                    env.new_global_ref(obj)
                });
                r.map_err(|e| describe(&mut env, e))
            });
            match made {
                Ok(bridge) => crate::android::set_bridge(bridge),
                Err(e) => log(&format!("the Android bridge could not start its Java half: {e}")),
            }
        }));
        Ok(())
    }

    /// Stops the Java half (on the UI thread).
    pub(crate) fn stop_bridge(&self, app: &AndroidApp, bridge: GlobalRef) {
        let vm = self.vm.clone();
        app.run_on_java_main_thread(Box::new(move || {
            if let Ok(mut env) = vm.attach_current_thread()
                && let Err(e) = env.call_method(bridge.as_obj(), "close", "()V", &[])
            {
                let text = describe(&mut env, e);
                log(&format!("the Android bridge could not stop its Java half: {text}"));
            }
        }));
    }

    /// Runs `f` on the UI thread with the Java half, once it has started; a Java exception
    /// is logged as `what` failing.
    fn on_bridge(
        &self,
        app: &AndroidApp,
        what: &'static str,
        f: impl FnOnce(&mut JNIEnv<'_>, &JObject<'_>) -> JResult<()> + Send + 'static,
    ) -> Result<(), String> {
        let bridge =
            crate::android::bridge().ok_or_else(|| "the bridge's Java half has not started".to_owned())?;
        let vm = self.vm.clone();
        app.run_on_java_main_thread(Box::new(move || {
            let Ok(mut env) = vm.attach_current_thread() else { return };
            let r = env.with_local_frame(16, |env| f(env, bridge.as_obj()));
            if let Err(e) = r {
                let text = describe(&mut env, e);
                log(&format!("RemedyUI: {what} failed: {text}"));
            }
        }));
        Ok(())
    }

    /// Starts or updates the input method session: the field's text around the caret, its
    /// selection in UTF-16 units, and the caret's left edge, top and bottom in window pixels.
    #[allow(clippy::too_many_arguments, reason = "mirrors the Java call")]
    pub(crate) fn text_input(
        &self,
        app: &AndroidApp,
        text: String,
        selection: (i32, i32),
        caret: (f32, f32, f32),
        secret: bool,
        multiline: bool,
        purpose: remedy_ui_host::InputPurpose,
        enter: remedy_ui_host::EnterKey,
    ) -> Result<(), String> {
        let purpose = convert::purpose_code(purpose);
        let action = convert::ime_action(enter);
        self.on_bridge(app, "the text input session", move |env, b| {
            let t = env.new_string(&text)?;
            env.call_method(
                b,
                "textInput",
                "(Ljava/lang/String;IIFFFZZII)V",
                &[
                    JValue::Object(&t),
                    JValue::Int(selection.0),
                    JValue::Int(selection.1),
                    JValue::Float(caret.0),
                    JValue::Float(caret.1),
                    JValue::Float(caret.2),
                    JValue::Bool(u8::from(secret)),
                    JValue::Bool(u8::from(multiline)),
                    JValue::Int(purpose),
                    JValue::Int(action),
                ],
            )?;
            Ok(())
        })
    }

    /// Tells the input method the app committed its composition.
    pub(crate) fn finish_composition(&self, app: &AndroidApp) -> Result<(), String> {
        self.on_bridge(app, "finishing the composition", |env, b| {
            env.call_method(b, "finishComposition", "()V", &[])?;
            Ok(())
        })
    }

    /// Ends the input method session and puts the keyboard away.
    pub(crate) fn end_text_input(&self, app: &AndroidApp) -> Result<(), String> {
        self.on_bridge(app, "ending the text input session", |env, b| {
            env.call_method(b, "endTextInput", "()V", &[])?;
            Ok(())
        })
    }

    /// Asks the person for `permission` (a manifest permission name) unless it is already
    /// granted; the answer comes back as [`Notice::Permission`].
    pub(crate) fn request_permission(
        &self,
        app: &AndroidApp,
        request: u64,
        permission: &str,
    ) -> Result<(), String> {
        let request = jlong::try_from(request).map_err(|e| e.to_string())?;
        let permission = permission.to_owned();
        self.on_bridge(app, "the permission prompt", move |env, b| {
            let name = env.new_string(&permission)?;
            env.call_method(
                b,
                "requestPermission",
                "(JLjava/lang/String;)V",
                &[JValue::Long(request), JValue::Object(&name)],
            )?;
            Ok(())
        })
    }

    /// Opens the share chooser for `share`; how it ended comes back as [`Notice::Share`].
    pub(crate) fn share(
        &self,
        app: &AndroidApp,
        request: u64,
        share: &remedy_ui_host::ShareOptions,
    ) -> Result<(), String> {
        let id = jlong::try_from(request).map_err(|e| e.to_string())?;
        let title = share.title.clone();
        let text = share.text.clone();
        let names: Vec<String> = share.files.iter().map(remedy_ui_host::ShareFile::safe_name).collect();
        let types: Vec<String> = share.files.iter().map(|f| f.mime_type().to_owned()).collect();
        let contents: Vec<Vec<u8>> = share.files.iter().map(|f| f.bytes.clone()).collect();
        let mime = share.mime_type();
        let require = share.require_files;
        self.on_bridge(app, "the share chooser", move |env, b| {
            let r = (|| {
                let names = string_array(env, &names)?;
                let types = string_array(env, &types)?;
                let len = jint::try_from(contents.len()).unwrap_or(jint::MAX);
                let bytes = env.new_object_array(len, "[B", JObject::null())?;
                for (i, c) in contents.iter().enumerate() {
                    let jb = env.byte_array_from_slice(c)?;
                    env.set_object_array_element(&bytes, jint::try_from(i).unwrap_or(0), &jb)?;
                    env.delete_local_ref(jb)?;
                }
                let title = env.new_string(&title)?;
                let text = env.new_string(&text)?;
                let mime = env.new_string(&mime)?;
                env.call_method(
                    b,
                    "share",
                    "(JLjava/lang/String;Ljava/lang/String;[Ljava/lang/String;[Ljava/lang/String;[[BLjava/lang/String;Z)V",
                    &[
                        JValue::Long(id),
                        JValue::Object(&title),
                        JValue::Object(&text),
                        JValue::Object(&names),
                        JValue::Object(&types),
                        JValue::Object(&bytes),
                        JValue::Object(&mime),
                        JValue::Bool(u8::from(require)),
                    ],
                )?;
                Ok(())
            })();
            if r.is_err() {
                // The app is told, so it never waits on a chooser that did not open.
                notify(Notice::Share(
                    request,
                    remedy_ui_host::ShareOutcome::Unsupported(remedy_ui_host::Unsupported::new(
                        "sharing",
                        "the share could not reach the chooser",
                    )),
                ));
            }
            r
        })
    }

    /// Opens the print dialog for an HTML page.
    pub(crate) fn print_html(&self, app: &AndroidApp, title: String, html: String) -> Result<(), String> {
        self.on_bridge(app, "printing", move |env, b| {
            let title = env.new_string(&title)?;
            let html = env.new_string(&html)?;
            env.call_method(
                b,
                "print",
                "(Ljava/lang/String;Ljava/lang/String;)V",
                &[JValue::Object(&title), JValue::Object(&html)],
            )?;
            Ok(())
        })
    }

    /// Opens the system's document picker; the choice comes back as [`Notice::Dialog`].
    #[allow(clippy::too_many_arguments, reason = "mirrors the Java call")]
    pub(crate) fn pick(
        &self,
        app: &AndroidApp,
        request: u64,
        save: bool,
        multiple: bool,
        extensions: Vec<String>,
        file_name: Option<String>,
        title: String,
    ) -> Result<(), String> {
        let request = jlong::try_from(request).map_err(|e| e.to_string())?;
        self.on_bridge(app, "the document picker", move |env, b| {
            let types = env.new_object_array(
                jint::try_from(extensions.len()).unwrap_or(0),
                "java/lang/String",
                JObject::null(),
            )?;
            for (i, m) in extensions.iter().enumerate() {
                let s = env.new_string(m)?;
                env.set_object_array_element(&types, jint::try_from(i).unwrap_or(0), s)?;
            }
            let name = match &file_name {
                Some(n) => JObject::from(env.new_string(n)?),
                None => JObject::null(),
            };
            let title = env.new_string(&title)?;
            env.call_method(
                b,
                "pick",
                "(JZZ[Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V",
                &[
                    JValue::Long(request),
                    JValue::Bool(u8::from(save)),
                    JValue::Bool(u8::from(multiple)),
                    JValue::Object(&types),
                    JValue::Object(&name),
                    JValue::Object(&title),
                ],
            )?;
            Ok(())
        })
    }

    /// Starts a system drag of text and links; how it ended comes back as
    /// [`Notice::DragEnded`].
    pub(crate) fn start_drag(
        &self,
        app: &AndroidApp,
        text: Option<String>,
        uris: Vec<String>,
        shadow_px: i32,
    ) -> Result<(), String> {
        self.on_bridge(app, "starting a drag", move |env, b| {
            let text = match &text {
                Some(t) => JObject::from(env.new_string(t)?),
                None => JObject::null(),
            };
            let list = env.new_object_array(
                jint::try_from(uris.len()).unwrap_or(0),
                "java/lang/String",
                JObject::null(),
            )?;
            for (i, u) in uris.iter().enumerate() {
                let s = env.new_string(u)?;
                env.set_object_array_element(&list, jint::try_from(i).unwrap_or(0), s)?;
            }
            env.call_method(
                b,
                "startDrag",
                "(Ljava/lang/String;[Ljava/lang/String;I)V",
                &[JValue::Object(&text), JValue::Object(&list), JValue::Int(shadow_px)],
            )?;
            Ok(())
        })
    }

    /// Calls one of the bridge class's static methods taking the activity, the handle and a
    /// notification's parts (`extra` follows them); a Java exception is the error's text.
    fn notification_call(
        &self,
        method: &str,
        n: &remedy_ui_host::Notification,
        extra: Option<jint>,
    ) -> Result<(), String> {
        let class = self.bridge_class()?;
        let picture = n.icon.as_ref().and_then(convert::argb_pixels);
        self.call(|env, a| {
            let class: &JClass<'_> = class.as_obj().into();
            let title = env.new_string(&n.title)?;
            let body = env.new_string(&n.body)?;
            let ids = strings(env, n.actions.iter().map(|x| x.id.as_str()))?;
            let labels = strings(env, n.actions.iter().map(|x| x.label.as_str()))?;
            let app = env.new_string(&n.app)?;
            let (pixels, side) = match &picture {
                Some((px, side)) => {
                    let arr = env.new_int_array(jint::try_from(px.len()).unwrap_or(0))?;
                    env.set_int_array_region(&arr, 0, px)?;
                    (JObject::from(arr), *side)
                }
                None => (JObject::null(), 0),
            };
            let mut args = vec![
                JValue::Object(a),
                JValue::Long(HANDLE),
                JValue::Long(bits(n.id)),
                JValue::Object(&title),
                JValue::Object(&body),
                JValue::Object(&ids),
                JValue::Object(&labels),
                JValue::Bool(u8::from(n.silent)),
                JValue::Object(&app),
                JValue::Object(&pixels),
                JValue::Int(side),
                JValue::Int(match n.urgency {
                    remedy_ui_host::Urgency::Warning => 1,
                    remedy_ui_host::Urgency::Error => 2,
                    _ => 0,
                }),
            ];
            let mut sig = String::from(
                "(Landroid/app/Activity;JJLjava/lang/String;Ljava/lang/String;[Ljava/lang/String;[Ljava/lang/String;ZLjava/lang/String;[III",
            );
            if let Some(x) = extra {
                args.push(JValue::Int(x));
                sig.push('I');
            }
            sig.push_str(")V");
            env.call_static_method(class, method, sig.as_str(), &args)?;
            Ok(())
        })
        .map_err(|e| convert::java_reason(&e))
    }

    /// Shows or replaces a notification; fails when the person turned them off.
    pub(crate) fn show_notification(&self, n: &remedy_ui_host::Notification) -> Result<(), String> {
        self.notification_call("showNotification", n, None)
    }

    /// Takes a notification away.
    pub(crate) fn withdraw_notification(&self, id: u64) -> Result<(), String> {
        let class = self.bridge_class()?;
        self.call(|env, a| {
            let class: &JClass<'_> = class.as_obj().into();
            env.call_static_method(
                class,
                "withdrawNotification",
                "(Landroid/app/Activity;JJ)V",
                &[JValue::Object(a), JValue::Long(HANDLE), JValue::Long(bits(id))],
            )?;
            Ok(())
        })
    }

    /// Whether the app's notifications reach the person.
    pub(crate) fn notifications_enabled(&self) -> Result<bool, String> {
        let class = self.bridge_class()?;
        self.call(|env, a| {
            let class: &JClass<'_> = class.as_obj().into();
            env.call_static_method(
                class,
                "notificationsEnabled",
                "(Landroid/app/Activity;J)Z",
                &[JValue::Object(a), JValue::Long(HANDLE)],
            )?
            .z()
        })
    }

    /// Starts or changes the app's foreground service of `types`, shown by `n`; fails with
    /// what Android would refuse.
    pub(crate) fn start_foreground(
        &self,
        n: &remedy_ui_host::Notification,
        types: i32,
    ) -> Result<(), String> {
        self.notification_call("startForeground", n, Some(types))
    }

    /// Ends the app's foreground service.
    pub(crate) fn stop_foreground(&self) -> Result<(), String> {
        let class = self.bridge_class()?;
        self.call(|env, a| {
            let class: &JClass<'_> = class.as_obj().into();
            env.call_static_method(
                class,
                "stopForeground",
                "(Landroid/app/Activity;)V",
                &[JValue::Object(a)],
            )?;
            Ok(())
        })
    }

    /// Starts (`on`) or stops the service that keeps the process running out of sight.
    pub(crate) fn hold_process(&self, on: bool) -> Result<(), String> {
        let class = self.bridge_class()?;
        self.call(|env, a| {
            let class: &JClass<'_> = class.as_obj().into();
            env.call_static_method(
                class,
                "holdProcess",
                "(Landroid/app/Activity;Z)V",
                &[JValue::Object(a), JValue::Bool(u8::from(on))],
            )?;
            Ok(())
        })
    }

    /// Adds "set text" to text fields' accessibility nodes, once the adapter is in place.
    pub(crate) fn extend_accessibility(&self, app: &AndroidApp) -> Result<(), String> {
        self.on_bridge(app, "extending accessibility", |env, b| {
            env.call_method(b, "extendAccessibility", "()V", &[])?;
            Ok(())
        })
    }

    /// Serves the activity's drawing view to assistive technology through AccessKit.
    pub(crate) fn accessibility(
        &self,
        activation: impl remedy_ui_host::accesskit::ActivationHandler + Send + 'static,
        action: impl remedy_ui_host::accesskit::ActionHandler + Send + 'static,
    ) -> Result<accesskit_android::InjectingAdapter, String> {
        let class = self.bridge_class()?;
        self.call(|env, a| {
            let class: &JClass<'_> = class.as_obj().into();
            let view = env
                .call_static_method(
                    class,
                    "contentView",
                    "(Landroid/app/Activity;)Landroid/view/View;",
                    &[JValue::Object(a)],
                )?
                .l()?;
            if view.is_null() {
                return Err(jni::errors::Error::NullPtr("the activity has no content view"));
            }
            Ok(accesskit_android::InjectingAdapter::new(env, &view, activation, action))
        })
    }
}

/// A 64-bit id as Java's `long`, bit for bit (Java prints it unsigned where it shows).
fn bits(id: u64) -> jlong {
    jlong::from_ne_bytes(id.to_ne_bytes())
}

/// A Java `String[]` of `items`.
fn strings<'a, 'b>(
    env: &mut JNIEnv<'a>,
    items: impl ExactSizeIterator<Item = &'b str>,
) -> JResult<jni::objects::JObjectArray<'a>> {
    let arr =
        env.new_object_array(jint::try_from(items.len()).unwrap_or(0), "java/lang/String", JObject::null())?;
    for (i, item) in items.enumerate() {
        let s = env.new_string(item)?;
        env.set_object_array_element(&arr, jint::try_from(i).unwrap_or(0), s)?;
    }
    Ok(arr)
}

/// The native methods of the bridge's Java class.
fn natives() -> Vec<NativeMethod> {
    let m = |name: &str, sig: &str, f: *mut c_void| NativeMethod {
        name: name.into(),
        sig: sig.into(),
        fn_ptr: f,
    };
    vec![
        m("trimMemory", "(JI)V", trim_memory as *mut c_void),
        m("configurationChanged", "(J)V", configuration_changed as *mut c_void),
        m("safeArea", "(JIIII)V", safe_area as *mut c_void),
        m("imePreedit", "(JLjava/lang/String;I)V", ime_preedit as *mut c_void),
        m("imeCommit", "(JLjava/lang/String;)V", ime_commit as *mut c_void),
        m("imeDelete", "(JII)V", ime_delete as *mut c_void),
        m("permissionDone", "(JJZ)V", permission_done as *mut c_void),
        m("shareDone", "(JJILjava/lang/String;)V", share_done as *mut c_void),
        m("dialogDone", "(JJ[Ljava/lang/String;Ljava/lang/String;)V", dialog_done as *mut c_void),
        m("documentSaved", "(Ljava/lang/String;Ljava/lang/String;)V", document_saved as *mut c_void),
        m("dragEnded", "(JI)V", drag_ended as *mut c_void),
        m("a11ySetText", "(JLjava/lang/String;)V", a11y_set_text as *mut c_void),
        m("notificationEvent", "(JJILjava/lang/String;)V", notification_event as *mut c_void),
        m("foregroundStopped", "(JLjava/lang/String;)V", foreground_stopped as *mut c_void),
        m("activityPaused", "(JZ)V", activity_paused as *mut c_void),
    ]
}

// Called by the Java half on the UI thread. They only queue and wake: nothing here may panic
// (a panic cannot cross into Java).

extern "system" fn trim_memory(_env: JNIEnv<'_>, _class: JClass<'_>, _handle: jlong, level: jint) {
    notify(Notice::Trim(level));
}

extern "system" fn configuration_changed(_env: JNIEnv<'_>, _class: JClass<'_>, _handle: jlong) {
    notify(Notice::Settings);
}

extern "system" fn safe_area(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    _handle: jlong,
    top: jint,
    right: jint,
    bottom: jint,
    left: jint,
) {
    notify(Notice::SafeArea([top, right, bottom, left]));
}

/// A Java string as text; `None` for null or unreadable.
fn jstring(env: &mut JNIEnv<'_>, s: &JString<'_>) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let r = env.get_string(s).map(String::from).ok();
    if r.is_none() {
        let _ = env.exception_clear();
    }
    r
}

extern "system" fn ime_preedit(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    _handle: jlong,
    text: JString<'_>,
    cursor: jint,
) {
    let text = jstring(&mut env, &text).unwrap_or_default();
    let c = usize::try_from(cursor).unwrap_or(0).min(text.len());
    let cursor = text.is_char_boundary(c).then_some(c..c);
    notify(Notice::Ime(remedy_ui_host::ImeEvent::Preedit { text, cursor }));
}

extern "system" fn ime_commit(mut env: JNIEnv<'_>, _class: JClass<'_>, _handle: jlong, text: JString<'_>) {
    let text = jstring(&mut env, &text).unwrap_or_default();
    notify(Notice::Ime(remedy_ui_host::ImeEvent::Commit(text)));
}

extern "system" fn ime_delete(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    _handle: jlong,
    before: jint,
    after: jint,
) {
    notify(Notice::Ime(remedy_ui_host::ImeEvent::DeleteSurrounding {
        before: usize::try_from(before).unwrap_or(0),
        after: usize::try_from(after).unwrap_or(0),
    }));
}

extern "system" fn dialog_done(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    _handle: jlong,
    request: jlong,
    paths: jni::objects::JObjectArray<'_>,
    error: JString<'_>,
) {
    let mut out = Vec::new();
    let n = env.get_array_length(&paths).unwrap_or(0);
    for i in 0..n {
        if let Ok(o) = env.get_object_array_element(&paths, i)
            && let Some(p) = jstring(&mut env, &JString::from(o))
        {
            out.push(std::path::PathBuf::from(p));
        }
    }
    let _ = env.exception_clear();
    if let Some(e) = jstring(&mut env, &error) {
        log(&format!("RemedyUI: the document picker: {e}"));
    }
    notify(Notice::Dialog(u64::try_from(request).unwrap_or(0), out));
}

/// A Java `String[]` of `list`.
fn string_array<'l>(env: &mut JNIEnv<'l>, list: &[String]) -> JResult<JObjectArray<'l>> {
    let len = jint::try_from(list.len()).unwrap_or(jint::MAX);
    let array = env.new_object_array(len, "java/lang/String", JObject::null())?;
    for (i, s) in list.iter().enumerate() {
        let js = env.new_string(s)?;
        env.set_object_array_element(&array, jint::try_from(i).unwrap_or(0), &js)?;
        env.delete_local_ref(js)?;
    }
    Ok(array)
}

extern "system" fn share_done(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    _handle: jlong,
    request: jlong,
    outcome: jint,
    error: JString<'_>,
) {
    let why = jstring(&mut env, &error);
    let _ = env.exception_clear();
    let outcome = match outcome {
        0 => remedy_ui_host::ShareOutcome::Shared,
        1 => remedy_ui_host::ShareOutcome::Cancelled,
        _ => remedy_ui_host::ShareOutcome::Unsupported(remedy_ui_host::Unsupported::new(
            "sharing",
            why.unwrap_or_else(|| "the chooser failed".into()),
        )),
    };
    notify(Notice::Share(u64::try_from(request).unwrap_or(0), outcome));
}

extern "system" fn permission_done(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    _handle: jlong,
    request: jlong,
    granted: jni::sys::jboolean,
) {
    notify(Notice::Permission(u64::try_from(request).unwrap_or(0), granted != 0));
}

extern "system" fn document_saved(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    path: JString<'_>,
    error: JString<'_>,
) {
    let path = jstring(&mut env, &path).unwrap_or_default();
    match jstring(&mut env, &error) {
        None => log(&format!("RemedyUI: saved {path} into its document")),
        Some(e) => log(&format!("RemedyUI: {path} could not be saved into its document: {e}")),
    }
}

extern "system" fn a11y_set_text(mut env: JNIEnv<'_>, _class: JClass<'_>, _handle: jlong, text: JString<'_>) {
    let text = jstring(&mut env, &text).unwrap_or_default();
    notify(Notice::SetText(text));
}

extern "system" fn notification_event(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    _handle: jlong,
    id: jlong,
    kind: jint,
    action: JString<'_>,
) {
    let action = jstring(&mut env, &action);
    if let Some(r) = convert::notification_response(kind, action) {
        notify(Notice::Notification(u64::from_ne_bytes(id.to_ne_bytes()), r));
    }
}

extern "system" fn foreground_stopped(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    _handle: jlong,
    why: JString<'_>,
) {
    let why = jstring(&mut env, &why).unwrap_or_else(|| "Android ended the foreground work".into());
    notify(Notice::ForegroundStopped(why));
}

extern "system" fn activity_paused(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    _handle: jlong,
    paused: jni::sys::jboolean,
) {
    crate::android::set_paused(paused != 0);
}

extern "system" fn drag_ended(_env: JNIEnv<'_>, _class: JClass<'_>, _handle: jlong, effect: jint) {
    notify(Notice::DragEnded(effect));
}

/// Seconds east of UTC at `unix_seconds` in the device's time zone.
pub(crate) fn utc_offset(unix_seconds: i64) -> Option<i32> {
    // `time_t` is 32 bits on 32-bit Android: dates past 2038 do not fit there.
    #[allow(clippy::useless_conversion, reason = "time_t is i32 on 32-bit targets")]
    let t: libc::time_t = unix_seconds.try_into().ok()?;
    // SAFETY: localtime_r reads `t` and writes only the `tm` it is given; bionic's
    // localtime_r follows the system time zone property itself.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return None;
        }
        tm
    };
    i32::try_from(tm.tm_gmtoff).ok()
}

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(priority: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

/// Writes a line to the system log (logcat) at info priority, under the tag `RemedyUI`.
pub(crate) fn log(text: &str) {
    const INFO: c_int = 4;
    let Ok(text) = CString::new(text.replace('\0', " ")) else { return };
    // SAFETY: both strings are NUL-terminated and outlive the call, which only reads them.
    unsafe {
        __android_log_write(INFO, c"RemedyUI".as_ptr(), text.as_ptr());
    }
}
