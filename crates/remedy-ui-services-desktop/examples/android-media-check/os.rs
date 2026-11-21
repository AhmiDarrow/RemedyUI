//! The example's JNI entry points and Context set-up: all of its unsafe code.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use jni::JNIEnv;
use jni::objects::{JClass, JObject};
use jni::sys::jstring;
use remedy_ui_services_desktop::media_session::{MediaSession, MediaState, NowPlaying, NowPlayingSink};

static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
static SESSION: Mutex<Option<MediaSession>> = Mutex::new(None);

fn start(env: &mut JNIEnv<'_>, context: &JObject<'_>) -> Result<(), String> {
    let vm = env.get_java_vm().map_err(|e| e.to_string())?;
    let context = env.new_global_ref(context).map_err(|e| e.to_string())?;
    // SAFETY: the VM pointer is the process's; the context is a global reference kept for
    // the process's life (forgotten below), as android-activity does for an activity.
    unsafe {
        ndk_context::initialize_android_context(
            vm.get_java_vm_pointer().cast(),
            context.as_obj().as_raw().cast(),
        )
    };
    std::mem::forget(context);
    let session = MediaSession::new(|c| {
        SEEN.lock().unwrap_or_else(PoisonError::into_inner).push(format!("{c:?}"));
    })
    .map_err(|e| e.to_string())?;
    session.set_now_playing(Some(&NowPlaying::new("RemedyUI media check").subtitle("S1 E3")));
    session.set_can_next(true);
    session.set_playback(MediaState::Playing, Duration::from_secs(30), Some(Duration::from_secs(100)));
    *SESSION.lock().unwrap_or_else(PoisonError::into_inner) = Some(session);
    Ok(())
}

/// `MediaCheck.start(context)`: "" once the session shows, else why not.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_remedyui_mediacheck_MediaCheck_start<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    context: JObject<'l>,
) -> jstring {
    let r = start(&mut env, &context).err().unwrap_or_default();
    env.new_string(r).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

/// `MediaCheck.commands()`: what the system pressed so far, one per line.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_remedyui_mediacheck_MediaCheck_commands<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
) -> jstring {
    let seen = SEEN.lock().unwrap_or_else(PoisonError::into_inner).join("\n");
    env.new_string(seen).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

/// `MediaCheck.stop()`: ends the session.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_remedyui_mediacheck_MediaCheck_stop<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
) {
    let gone = SESSION.lock().unwrap_or_else(PoisonError::into_inner).take();
    drop(gone);
}
