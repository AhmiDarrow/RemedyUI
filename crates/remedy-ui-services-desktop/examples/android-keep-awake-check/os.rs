//! The example's JNI entry points and Context set-up: all of its unsafe code.

use std::sync::{Mutex, PoisonError};

use jni::JNIEnv;
use jni::objects::{JClass, JObject};
use jni::sys::jstring;
use remedy_ui_services_desktop::power::{Inhibit, Inhibitor, inhibit};

static HOLDS: Mutex<Vec<Inhibitor>> = Mutex::new(Vec::new());

fn hold(env: &mut JNIEnv<'_>, activity: &JObject<'_>) -> Result<(), String> {
    let vm = env.get_java_vm().map_err(|e| e.to_string())?;
    let activity = env.new_global_ref(activity).map_err(|e| e.to_string())?;
    // SAFETY: the VM pointer is the process's; the activity is a global reference kept for
    // the process's life (forgotten below), as android-activity does for its activity.
    unsafe {
        ndk_context::initialize_android_context(
            vm.get_java_vm_pointer().cast(),
            activity.as_obj().as_raw().cast(),
        )
    };
    std::mem::forget(activity);
    let mut holds = HOLDS.lock().unwrap_or_else(PoisonError::into_inner);
    // Two holds, as two players would take: the flag stays until the last goes.
    for _ in 0..2 {
        holds
            .push(inhibit(Inhibit::Display, "keep-awake check", "Playing video").map_err(|e| e.to_string())?);
    }
    Ok(())
}

/// `KeepAwakeCheck.hold(activity)`: "" once both holds are taken, else why not.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_remedyui_keepawakecheck_KeepAwakeCheck_hold<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    activity: JObject<'l>,
) -> jstring {
    let r = hold(&mut env, &activity).err().unwrap_or_default();
    env.new_string(r).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

/// `KeepAwakeCheck.release()`: lets go of one hold; returns how many are left.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_remedyui_keepawakecheck_KeepAwakeCheck_release<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
) -> jni::sys::jint {
    let mut holds = HOLDS.lock().unwrap_or_else(PoisonError::into_inner);
    drop(holds.pop());
    jni::sys::jint::try_from(holds.len()).unwrap_or(0)
}
